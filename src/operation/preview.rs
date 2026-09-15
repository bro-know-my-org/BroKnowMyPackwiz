//! Preconditions shared by read-only previews and queued application.
use super::{Control, Error, ErrorCode, Result, durable};
use crate::{config::ProjectConfig, layout::PackLayout, scan::ScanReport};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Guard {
    metadata: Vec<String>,
    fingerprints: BTreeMap<String, Option<String>>,
}
impl Guard {
    pub fn merge(&mut self, other: Self) -> Result<()> {
        if self.metadata != other.metadata {
            return Err(stale("metadata inventory"));
        }
        for (path, value) in other.fingerprints {
            if self
                .fingerprints
                .get(&path)
                .is_some_and(|previous| previous != &value)
            {
                return Err(stale(&path));
            }
            self.fingerprints.insert(path, value);
        }
        Ok(())
    }
    pub fn capture(root: &Path, control: &Control) -> Result<Self> {
        let config = ProjectConfig::load(root).map_err(Error::from)?;
        let layout = PackLayout::from_config(&config);
        let report = ScanReport::build(root, &config, &layout).map_err(Error::from)?;
        let mut guard = Self {
            metadata: report.metadata.into_iter().map(|m| m.path).collect(),
            fingerprints: BTreeMap::new(),
        };
        let mut paths = guard.metadata.clone();
        paths.extend(
            [
                "pack.toml",
                "index.toml",
                "packwiz.json",
                ".pw/config.toml",
                ".gitignore",
            ]
            .into_iter()
            .map(str::to_string),
        );
        paths.push(
            config
                .scan
                .packwizignore
                .to_string_lossy()
                .replace('\\', "/"),
        );
        paths.extend(
            report
                .included
                .into_iter()
                .filter(|p| p.ends_with("/.gitignore")),
        );
        for path in paths {
            control.check()?;
            guard.watch(root, &path)?;
        }
        Ok(guard)
    }
    pub fn watch(&mut self, root: &Path, relative: &str) -> Result<()> {
        crate::operation::paths::relative(relative)?;
        let target = durable::absolute(&durable::canonical(root)?.join(relative))?;
        let fingerprint = durable::fingerprint(&target)?;
        if self
            .fingerprints
            .get(relative)
            .is_some_and(|old| *old != fingerprint)
        {
            return Err(stale(relative));
        }
        self.fingerprints.insert(relative.into(), fingerprint);
        Ok(())
    }
    pub fn validate(&self, root: &Path, control: &Control) -> Result<()> {
        for (relative, expected) in &self.fingerprints {
            control.check()?;
            crate::operation::paths::relative(relative)?;
            let path = durable::absolute(&durable::canonical(root)?.join(relative))?;
            if durable::fingerprint(&path)? != *expected {
                return Err(stale(relative));
            }
        }
        let current = Self::capture(root, control)?;
        if current.metadata != self.metadata {
            return Err(stale("metadata inventory"));
        }
        // Includes newly introduced nested ignore rules, not only existing paths.
        for (path, value) in current.fingerprints {
            if self.fingerprints.get(&path) != Some(&value) {
                return Err(stale(&path));
            }
        }
        Ok(())
    }
}
fn stale(path: &str) -> Error {
    Error::named(
        ErrorCode::Conflict,
        "preview_stale",
        format!("preview_stale: {path}"),
    )
    .context(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn stale_queued_preview_never_publishes_drafts() {
        let base = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(base.join("pack/mods")).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let root = base.join("pack");
        fs::write(root.join("pack.toml"), "name = \"Test\"\n").unwrap();
        let state = base.join("state");
        let control = Control::default();
        let guard = Guard::capture(&root, &control).unwrap();
        let document = "name = \"New\"\nfilename = \"new.jar\"\n".parse().unwrap();
        let draft = super::super::edit::draft(&state, "mods/new.pw.toml", None, &document).unwrap();
        fs::write(root.join("pack.toml"), "name = \"Changed\"\n").unwrap();
        let result = super::super::edit::execute(
            &root,
            &state,
            &base.join("task"),
            &[draft],
            Some(&guard),
            &control,
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
        assert!(!root.join("mods/new.pw.toml").exists());
        assert!(!root.join("index.toml").exists());
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn preview_detects_config_changes_new_metadata_and_new_manual_files() {
        let dir = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(dir.join("mods")).unwrap();
        let dir = fs::canonicalize(dir).unwrap();
        fs::write(dir.join("pack.toml"), "name = \"Test\"\n").unwrap();
        let control = Control::default();
        let mut guard = Guard::capture(&dir, &control).unwrap();
        guard.watch(&dir, "mods/new.jar").unwrap();
        guard.validate(&dir, &control).unwrap();
        fs::write(dir.join("mods/new.jar"), b"manual").unwrap();
        assert!(
            guard
                .validate(&dir, &control)
                .unwrap_err()
                .detail
                .contains("new.jar")
        );
        fs::remove_file(dir.join("mods/new.jar")).unwrap();
        fs::write(dir.join("mods/new.pw.toml"), "name = \"new\"").unwrap();
        assert!(guard.validate(&dir, &control).is_err());
        fs::remove_file(dir.join("mods/new.pw.toml")).unwrap();
        fs::write(dir.join("pack.toml"), "name = \"Changed\"").unwrap();
        assert!(
            guard
                .validate(&dir, &control)
                .unwrap_err()
                .detail
                .contains("pack.toml")
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
