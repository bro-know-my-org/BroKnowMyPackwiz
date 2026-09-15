use super::{
    prepare,
    updates::{Preview, Version},
};
use crate::{
    config::ProjectConfig,
    layout::PackLayout,
    metadata::ModMetadata,
    operation::{
        Control, Error, ErrorCode, Result,
        download::{Download, Source},
        durable, edit,
        queue::Request,
    },
    scan::ScanReport,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use toml_edit::Value;

pub struct Row {
    pub name: String,
    pub before: String,
    pub after: String,
    pub action: &'static str,
}
pub struct Planned {
    pub request: Request,
    pub rows: Vec<Row>,
    pub download: bool,
}

pub fn prepare(
    root: &Path,
    state: &Path,
    preview: &Preview,
    selected: &[String],
    download: bool,
    control: &Control,
) -> Result<Planned> {
    preview.guard.validate(root, control)?;
    let chosen: BTreeSet<_> = selected.iter().collect();
    if chosen
        .iter()
        .any(|path| !preview.candidates.iter().any(|c| &c.relative == *path))
    {
        return Err(invalid("unknown_update_candidate"));
    }
    let candidates: Vec<_> = preview
        .candidates
        .iter()
        .filter(|c| chosen.contains(&c.relative))
        .collect();
    let selections: Vec<_> = candidates
        .iter()
        .filter_map(|c| match &c.version {
            Version::CurseForge(file) => Some((file.clone(), c.side.clone())),
            _ => None,
        })
        .collect();
    let mut guard = preview.guard.clone();
    let mut drafts = Vec::new();
    let mut downloads = Vec::new();
    let mut rows = Vec::new();
    if !selections.is_empty() {
        let client = super::curseforge::Client::for_pack(root)?;
        let planned = prepare::curseforge_many(root, state, &client, selections, false, control)?;
        guard.merge(planned.guard)?;
        drafts = planned.drafts;
        downloads = planned.downloads;
        for row in planned.rows {
            rows.push(Row {
                name: row.name,
                before: row
                    .old
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "—".into()),
                after: format!("{} · {}", row.file.filename, row.file.id),
                action: match row.action {
                    super::dependencies::Action::Add => "add",
                    super::dependencies::Action::Update => "cf_update",
                    super::dependencies::Action::Reuse => "cf_reuse",
                },
            });
        }
    }
    let config = ProjectConfig::load(root).map_err(Error::from)?;
    let layout = PackLayout::from_config(&config);
    let report = ScanReport::build(root, &config, &layout).map_err(Error::from)?;
    let mut existing_targets = BTreeMap::new();
    for item in report.metadata {
        let meta = ModMetadata::load(&root.join(&item.path)).map_err(Error::from)?;
        if let Some(filename) = meta.filename {
            let target = crate::install::resolve_pack_file_path(&item.path, &filename, &layout)
                .map_err(Error::from)?;
            if existing_targets
                .insert(target.to_lowercase(), item.path)
                .is_some()
            {
                return Err(invalid("duplicate_managed_target"));
            }
        }
    }
    for candidate in candidates {
        control.check()?;
        let Version::GitHub(file) = &candidate.version else {
            continue;
        };
        let (mut doc, expected) = edit::document(root, &candidate.relative)?;
        let metadata = ModMetadata::load(&root.join(&candidate.relative)).map_err(Error::from)?;
        let target =
            crate::install::resolve_pack_file_path(&candidate.relative, &file.filename, &layout)
                .map_err(Error::from)?;
        if existing_targets
            .get(&target.to_lowercase())
            .is_some_and(|owner| owner != &candidate.relative)
        {
            return Err(Error::new(
                ErrorCode::Conflict,
                format!("file_collision: {target}"),
            ));
        }
        crate::update::reject_manual_target_collision(
            root,
            &layout,
            &root.join(&candidate.relative),
            &metadata,
            &file.filename,
            &file.hash,
            &file.hash_format,
        )
        .map_err(Error::from)?;
        for (keys, value) in [
            (vec!["filename"], file.filename.as_str()),
            (vec!["download", "url"], file.url.as_str()),
            (vec!["download", "hash-format"], file.hash_format.as_str()),
            (vec!["download", "hash"], file.hash.as_str()),
        ] {
            edit::set(&mut doc, &keys, Value::from(value))?;
        }
        drafts.push(edit::draft(state, &candidate.relative, expected, &doc)?);
        guard.watch(root, &target)?;
        downloads.push(Download {
            expected: durable::fingerprint(&root.join(&target))?,
            relative: target,
            source: Source::Url(file.url.clone()),
            hash_format: file.hash_format.clone(),
            hash: file.hash.clone(),
            preserve: metadata.preserve,
        });
        rows.push(Row {
            name: candidate.name.clone(),
            before: candidate.before.clone(),
            after: candidate.after.clone(),
            action: "cf_update",
        });
    }
    let mut paths = BTreeSet::new();
    for draft in &drafts {
        if !paths.insert(draft.relative.to_lowercase()) {
            return Err(invalid("duplicate_update_target"));
        }
    }
    paths.clear();
    for download in &downloads {
        if !paths.insert(download.relative.to_lowercase()) {
            return Err(Error::new(ErrorCode::Conflict, "duplicate_download_target"));
        }
    }
    guard.validate(root, control)?;
    let request = if download {
        Request::Download {
            drafts,
            downloads,
            guard,
        }
    } else {
        Request::PreparedEdit { drafts, guard }
    };
    Ok(Planned {
        request,
        rows,
        download,
    })
}
fn invalid(key: &str) -> Error {
    Error::new(ErrorCode::Invalid, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::updates::Candidate, github::GitHubFileInfo, metadata::Side,
        operation::preview::Guard,
    };
    use std::fs;
    #[test]
    fn frozen_github_candidate_applies_without_lookup_and_stale_reuse_is_rejected() {
        let base = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(base.join("pack/mods")).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let root = base.join("pack");
        fs::write(root.join("pack.toml"), "name = \"Test\"\n").unwrap();
        fs::write(root.join("mods/a.pw.toml"),"# author note\nname = \"A\"\nfilename = \"old.jar\"\n[update.github]\nproject = \"owner/repo\"\n[extra]\ncustom = 42\n").unwrap();
        fs::write(root.join("mods/manual.jar"), b"manual").unwrap();
        let control = Control::default();
        let preview = Preview {
            guard: Guard::capture(&root, &control).unwrap(),
            skipped: Vec::new(),
            candidates: vec![Candidate {
                relative: "mods/a.pw.toml".into(),
                name: "A".into(),
                side: Side::Both,
                before: "old.jar".into(),
                after: "new.jar".into(),
                version: Version::GitHub(GitHubFileInfo {
                    name: "A".into(),
                    filename: "new.jar".into(),
                    url: "https://example.invalid/frozen.jar".into(),
                    hash_format: "sha256".into(),
                    hash: "a".repeat(64),
                }),
            }],
        };
        let state = base.join("state");
        let selected = vec!["mods/a.pw.toml".into()];
        let planned = prepare(&root, &state, &preview, &selected, false, &control).unwrap();
        let Request::PreparedEdit { drafts, guard } = planned.request else {
            panic!()
        };
        edit::execute(
            &root,
            &state,
            &base.join("task"),
            &drafts,
            Some(&guard),
            &control,
        )
        .unwrap();
        let text = fs::read_to_string(root.join("mods/a.pw.toml")).unwrap();
        assert!(
            text.contains("frozen.jar")
                && text.contains("# author note")
                && text.contains("custom = 42")
        );
        assert_eq!(fs::read(root.join("mods/manual.jar")).unwrap(), b"manual");
        assert!(matches!(
            prepare(&root, &state, &preview, &selected, false, &control),
            Err(Error {
                code: ErrorCode::Conflict,
                ..
            })
        ));
        fs::remove_dir_all(base).unwrap();
    }
}
