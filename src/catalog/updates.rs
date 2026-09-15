use super::curseforge::{Client, File, Filter};
use crate::{
    config::ProjectConfig,
    github::GitHubFileInfo,
    layout::PackLayout,
    metadata::{ModMetadata, Side},
    operation::{Control, Error, ErrorCode, Event, Result, preview::Guard},
    scan::ScanReport,
};
use std::{collections::BTreeSet, path::Path};

#[derive(Clone)]
pub enum Version {
    CurseForge(File),
    GitHub(GitHubFileInfo),
}
#[derive(Clone)]
pub struct Candidate {
    pub relative: String,
    pub name: String,
    pub side: Side,
    pub before: String,
    pub after: String,
    pub version: Version,
}
#[derive(Clone)]
pub struct Preview {
    pub guard: Guard,
    pub candidates: Vec<Candidate>,
    pub skipped: Vec<(String, &'static str)>,
}
pub trait Provider {
    fn curseforge(&self, metadata: &ModMetadata, filter: &Filter) -> Result<File>;
    fn github(&self, metadata: &ModMetadata) -> Result<GitHubFileInfo>;
}
pub struct Online<'a> {
    pub root: &'a Path,
}
impl Provider for Online<'_> {
    fn curseforge(&self, metadata: &ModMetadata, filter: &Filter) -> Result<File> {
        let id = metadata
            .curseforge_project_id
            .ok_or_else(|| invalid("missing_curseforge_project"))?;
        let client = Client::for_pack(self.root)?;
        let project = client.project(id)?;
        let mut filter = filter.clone();
        if matches!(project.class_id, 12 | 6552) {
            filter.loader = None;
        }
        client.latest_compatible(id, &filter)
    }
    fn github(&self, metadata: &ModMetadata) -> Result<GitHubFileInfo> {
        crate::github::resolve_github_release_asset(
            metadata
                .github_project
                .as_deref()
                .ok_or_else(|| invalid("missing_github_project"))?,
            metadata.github_tag.as_deref(),
            metadata.github_asset.as_deref(),
            None,
            None,
        )
        .map_err(Error::from)
    }
}
/// Query only. The apply phase receives these exact candidates, never "latest".
pub fn query(
    root: &Path,
    paths: &[String],
    provider: &impl Provider,
    control: &Control,
) -> Result<Preview> {
    let mut guard = Guard::capture(root, control)?;
    let config = ProjectConfig::load_operation(root)?;
    let layout = PackLayout::from_config(&config);
    let filter = Filter::for_pack(root)?;
    let report = ScanReport::build_operation(root, &config, &layout)?;
    let requested: BTreeSet<_> = paths.iter().cloned().collect();
    for path in &requested {
        if !report.metadata.iter().any(|entry| &entry.path == path) {
            return Err(invalid("update_target_missing"));
        }
    }
    let mut candidates = Vec::new();
    let mut skipped = Vec::new();
    for entry in report.metadata {
        if !requested.is_empty() && !requested.contains(&entry.path) {
            continue;
        }
        control.check()?;
        let metadata = ModMetadata::load_operation(&root.join(&entry.path))?;
        if metadata.pin {
            skipped.push((entry.path, "update_pinned"));
            continue;
        }
        if let Some(filename) = &metadata.filename {
            guard.watch(
                root,
                &crate::install::resolve_pack_file_path(&entry.path, filename, &layout)
                    .map_err(Error::from)?,
            )?;
        }
        control.emit(Event::Phase("querying_updates".into()));
        control.emit(Event::Log(entry.path.clone()));
        let (version, filename, hash, before, after, unchanged) =
            if metadata.download_mode.as_deref() == Some("metadata:curseforge") {
                let file = provider.curseforge(&metadata, &filter)?;
                if Some(file.project_id) != metadata.curseforge_project_id {
                    return Err(invalid("update_project_mismatch"));
                }
                let unchanged = metadata.curseforge_file_id == Some(file.id)
                    && metadata.download_hash_format.as_deref() == Some("sha1")
                    && metadata.filename.as_deref() == Some(&file.filename)
                    && metadata
                        .download_hash
                        .as_ref()
                        .is_some_and(|hash| hash.eq_ignore_ascii_case(&file.sha1));
                let before = format!(
                    "{} · {}",
                    metadata.filename.as_deref().unwrap_or("?"),
                    metadata
                        .curseforge_file_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "?".into())
                );
                let after = format!("{} · {}", file.filename, file.id);
                let (filename, hash) = (file.filename.clone(), file.sha1.clone());
                (
                    Version::CurseForge(file),
                    filename,
                    hash,
                    before,
                    after,
                    unchanged,
                )
            } else if metadata.github_project.is_some() {
                let file = provider.github(&metadata)?;
                let unchanged = metadata.filename.as_deref() == Some(&file.filename)
                    && metadata.download_hash_format.as_deref() == Some(file.hash_format.as_str())
                    && metadata.download_url.as_deref() == Some(&file.url)
                    && metadata
                        .download_hash
                        .as_ref()
                        .is_some_and(|hash| hash.eq_ignore_ascii_case(&file.hash));
                let before = format!(
                    "{} · {}",
                    metadata.filename.as_deref().unwrap_or("?"),
                    short(metadata.download_hash.as_deref().unwrap_or("?"))
                );
                let after = format!("{} · {}", file.filename, short(&file.hash));
                let (filename, hash) = (file.filename.clone(), file.hash.clone());
                (
                    Version::GitHub(file),
                    filename,
                    hash,
                    before,
                    after,
                    unchanged,
                )
            } else {
                skipped.push((entry.path, "update_no_provider"));
                continue;
            };
        control.check()?;
        crate::operation::paths::filename(&filename)?;
        if hash.is_empty() {
            return Err(invalid("update_hash_missing"));
        }
        if unchanged {
            skipped.push((entry.path, "update_unchanged"));
            continue;
        }
        guard.watch(
            root,
            &crate::install::resolve_pack_file_path(&entry.path, &filename, &layout)
                .map_err(Error::from)?,
        )?;
        candidates.push(Candidate {
            relative: entry.path.clone(),
            name: metadata.name.clone().unwrap_or(entry.path),
            side: crate::install::side_from_directory_or_metadata(entry.side_hint, &metadata),
            before,
            after,
            version,
        });
    }
    guard.validate(root, control)?;
    Ok(Preview {
        guard,
        candidates,
        skipped,
    })
}
fn short(value: &str) -> String {
    value.chars().take(12).collect()
}
fn invalid(key: &str) -> Error {
    Error::key(ErrorCode::Invalid, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::durable;
    use std::{cell::Cell, fs};
    struct Fake {
        calls: Cell<usize>,
    }
    impl Provider for Fake {
        fn curseforge(&self, _: &ModMetadata, _: &Filter) -> Result<File> {
            panic!("unexpected CF query")
        }
        fn github(&self, _: &ModMetadata) -> Result<GitHubFileInfo> {
            self.calls.set(self.calls.get() + 1);
            Ok(GitHubFileInfo {
                name: "New".into(),
                filename: "new.jar".into(),
                url: "https://example.invalid/new.jar".into(),
                hash_format: "sha256".into(),
                hash: "a".repeat(64),
            })
        }
    }
    #[test]
    fn querying_skips_pins_and_freezes_candidates_without_writes() {
        let root = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(root.join("mods")).unwrap();
        let root = fs::canonicalize(root).unwrap();
        fs::write(root.join("pack.toml"), "name = \"Test\"\n").unwrap();
        let text =
            "name = \"Old\"\nfilename = \"old.jar\"\n[update.github]\nproject = \"owner/repo\"\n";
        fs::write(root.join("mods/a.pw.toml"), text).unwrap();
        fs::write(root.join("mods/b.pw.toml"), format!("pin = true\n{text}")).unwrap();
        let provider = Fake {
            calls: Cell::new(0),
        };
        let preview = query(&root, &[], &provider, &Control::default()).unwrap();
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(provider.calls.get(), 1);
        assert!(preview.candidates[0].after.contains("new.jar"));
        assert_eq!(preview.skipped[0].1, "update_pinned");
        assert_eq!(
            fs::read_to_string(root.join("mods/a.pw.toml")).unwrap(),
            text
        );
        assert!(!root.join("index.toml").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
