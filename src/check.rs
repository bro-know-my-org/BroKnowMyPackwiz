use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::ProjectConfig;
use crate::install;
use crate::layout::PackLayout;
use crate::metadata::{ModMetadata, Side};
use crate::pathutil::join_slash;
use crate::scan::{ScanReport, SideHint};

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub ok: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

impl CheckResult {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

pub fn check(root: &Path) -> CheckResult {
    let mut result = CheckResult {
        ok: Vec::new(),
        warnings: Vec::new(),
        errors: Vec::new(),
    };

    if !root.exists() {
        result
            .errors
            .push(format!("pack root does not exist: {}", root.display()));
        return result;
    }
    result.ok.push(format!("pack root: {}", root.display()));

    for required in ["pack.toml", "index.toml", ".packwizignore", ".gitignore"] {
        let path = root.join(required);
        if path.exists() {
            result.ok.push(format!("found {required}"));
        } else {
            result.errors.push(format!("missing {required}"));
        }
    }

    let config = match ProjectConfig::load(root) {
        Ok(config) => {
            result
                .ok
                .push(format!("config: {}", config.source.display()));
            config
        }
        Err(err) => {
            result.errors.push(err);
            return result;
        }
    };
    let layout = PackLayout::from_config(&config);

    let report = match ScanReport::build(root, &config, &layout) {
        Ok(report) => {
            result
                .ok
                .push(format!("metadata files: {}", report.metadata.len()));
            result
                .ok
                .push(format!("included files: {}", report.included.len()));
            report
        }
        Err(err) => {
            result.errors.push(err);
            return result;
        }
    };

    let managed_files = match install::read_manifest_paths(root) {
        Ok(paths) => paths,
        Err(err) => {
            result.errors.push(err);
            BTreeSet::new()
        }
    };
    let unmanaged_pack_files = report
        .included
        .iter()
        .filter(|rel| !crate::scan::is_metadata_file(rel, &layout))
        .filter(|rel| !managed_files.contains(*rel))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut targets: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in &report.metadata {
        let metadata_path = join_slash(root, &entry.path);
        let metadata = match ModMetadata::load(&metadata_path) {
            Ok(metadata) => metadata,
            Err(err) => {
                result.errors.push(err);
                continue;
            }
        };
        if metadata.name.as_deref().unwrap_or("").trim().is_empty() {
            result
                .warnings
                .push(format!("metadata has no name: {}", entry.path));
        }
        let Some(filename) = metadata.filename.as_deref() else {
            result
                .errors
                .push(format!("metadata has no filename: {}", entry.path));
            continue;
        };
        if metadata.download_hash_format.is_none() || metadata.download_hash.is_none() {
            result
                .warnings
                .push(format!("metadata has no download hash: {}", entry.path));
        } else if let (Some(format), Some(hash)) = (
            metadata.download_hash_format.as_deref(),
            metadata.download_hash.as_deref(),
        ) && let Err(err) = validate_hash(format, hash)
        {
            result.errors.push(format!(
                "metadata has invalid download hash: {}: {err}",
                entry.path
            ));
        }
        let has_download_url = metadata
            .download_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        if metadata.download_mode.as_deref() == Some("metadata:curseforge") {
            if metadata.curseforge_project_id.is_none() || metadata.curseforge_file_id.is_none() {
                result.errors.push(format!(
                    "CurseForge metadata is missing update.curseforge project-id/file-id: {}",
                    entry.path
                ));
            }
        } else if !has_download_url {
            result.warnings.push(format!(
                "metadata has no usable download source: {}",
                entry.path
            ));
        }
        let declared_side = side_from_directory_or_metadata(&entry.side_hint, &metadata);
        if matches!(declared_side, Side::Unknown(_)) {
            result
                .errors
                .push(format!("metadata has unsupported side: {}", entry.path));
        }
        let target = match install::resolve_pack_file_path(&entry.path, filename, &layout) {
            Ok(target) => target,
            Err(err) => {
                result.errors.push(format!("{}: {err}", entry.path));
                continue;
            }
        };
        if unmanaged_pack_files.contains(&target) {
            result.errors.push(format!(
                "metadata target collides with existing file {target}: {}",
                entry.path
            ));
        }
        targets.entry(target).or_default().push(entry.path.clone());
    }

    for (target, sources) in targets {
        if sources.len() > 1 {
            result.errors.push(format!(
                "duplicate target file {target}: {}",
                sources.join(", ")
            ));
        }
    }

    check_git_ignore(root, &layout, &mut result);

    result
}

fn side_from_directory_or_metadata(hint: &SideHint, metadata: &ModMetadata) -> Side {
    match hint {
        SideHint::Server => Side::Server,
        SideHint::Client => Side::Client,
        SideHint::Common => Side::Both,
        SideHint::Unknown => metadata.side.clone().unwrap_or(Side::Both),
    }
}

fn check_git_ignore(root: &Path, layout: &PackLayout, result: &mut CheckResult) {
    if !root.join(".git").exists() {
        result
            .warnings
            .push("git ignore checks skipped: .git not found".to_string());
        return;
    }
    let jar_sample = crate::pathutil::to_slash(&layout.jar_root.join("example.jar"));
    match git_check_ignore(root, &jar_sample) {
        Ok(true) => result.ok.push("mods/*.jar is ignored".to_string()),
        Ok(false) => result
            .errors
            .push("mods/*.jar does not appear to be ignored".to_string()),
        Err(err) => result.warnings.push(err),
    }
    let metadata_sample = crate::pathutil::to_slash(
        &layout
            .metadata_root
            .join(format!("example.{}", layout.metadata_extension)),
    );
    match git_check_ignore(root, &metadata_sample) {
        Ok(false) => result.ok.push("metadata files are trackable".to_string()),
        Ok(true) => result
            .errors
            .push("metadata files appear to be ignored".to_string()),
        Err(err) => result.warnings.push(err),
    }
}

fn validate_hash(format: &str, hash: &str) -> Result<(), String> {
    let format = format.trim().to_ascii_lowercase();
    let hash = hash.trim();
    let expected_len = match format.as_str() {
        "sha1" => 40,
        "sha256" => 64,
        "sha512" => 128,
        other => return Err(format!("unsupported hash format: {other}")),
    };
    if hash.len() != expected_len || !hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("expected {expected_len} hex chars for {format}"));
    }
    Ok(())
}

fn git_check_ignore(root: &Path, path: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .arg("check-ignore")
        .arg("--quiet")
        .arg("--")
        .arg(path)
        .current_dir(root)
        .output()
        .map_err(|err| format!("failed to run git check-ignore: {err}"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        Some(code) => Err(format!("git check-ignore {path} exited with {code}")),
        None => Err(format!("git check-ignore {path} was terminated")),
    }
}

#[allow(dead_code)]
fn _assert_pathbuf_send_sync(_: PathBuf) {}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn validates_supported_hash_lengths() {
        assert!(validate_hash("sha256", &"a".repeat(64)).is_ok());
        assert!(validate_hash("sha256", "abc").is_err());
        assert!(validate_hash("md5", &"a".repeat(32)).is_err());
    }

    #[test]
    fn reports_metadata_target_colliding_with_existing_file() {
        let root = temp_root("check-target-collision");
        fs::create_dir_all(root.join("mods")).unwrap();
        for file in ["pack.toml", "index.toml", ".packwizignore", ".gitignore"] {
            fs::write(root.join(file), "").unwrap();
        }
        fs::write(root.join("mods").join("foo.jar"), b"manual").unwrap();
        fs::write(
            root.join("mods").join("foo.pw"),
            "name = \"Foo\"\n\
             filename = \"foo.jar\"\n\
             [download]\n\
             url = \"https://example.invalid/foo.jar\"\n\
             hash-format = \"sha256\"\n\
             hash = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n",
        )
        .unwrap();

        let result = check(&root);

        assert!(
            result
                .errors
                .iter()
                .any(|item| item.contains("metadata target collides"))
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn does_not_report_collision_for_manifest_managed_file() {
        let root = temp_root("check-managed-target");
        fs::create_dir_all(root.join("mods")).unwrap();
        for file in ["pack.toml", "index.toml", ".packwizignore", ".gitignore"] {
            fs::write(root.join(file), "").unwrap();
        }
        fs::write(
            root.join("packwiz.json"),
            "{ \"format\": \"bkmpw:1\", \"files\": [{\"path\":\"mods/foo.jar\"}] }",
        )
        .unwrap();
        fs::write(root.join("mods").join("foo.jar"), b"managed").unwrap();
        fs::write(
            root.join("mods").join("foo.pw"),
            "name = \"Foo\"\n\
             filename = \"foo.jar\"\n\
             [download]\n\
             url = \"https://example.invalid/foo.jar\"\n\
             hash-format = \"sha256\"\n\
             hash = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n",
        )
        .unwrap();

        let result = check(&root);

        assert!(
            !result
                .errors
                .iter()
                .any(|item| item.contains("metadata target collides"))
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reports_curseforge_metadata_missing_ids() {
        let root = temp_root("check-cf-ids");
        fs::create_dir_all(root.join("mods")).unwrap();
        for file in ["pack.toml", "index.toml", ".packwizignore", ".gitignore"] {
            fs::write(root.join(file), "").unwrap();
        }
        fs::write(
            root.join("mods").join("foo.pw"),
            "name = \"Foo\"\n\
             filename = \"foo.jar\"\n\
             [download]\n\
             mode = \"metadata:curseforge\"\n\
             hash-format = \"sha1\"\n\
             hash = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n",
        )
        .unwrap();

        let result = check(&root);

        assert!(
            result
                .errors
                .iter()
                .any(|item| item.contains("CurseForge metadata is missing"))
        );

        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(prefix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "{}-{}",
            prefix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }
}
