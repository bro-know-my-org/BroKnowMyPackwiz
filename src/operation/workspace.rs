use super::{Control, Error, ErrorCode, Event, Result, durable, transaction::Change};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Existing core operations run against a private copy; only the resulting file
/// changes can be published by Transaction. Never use hard links here.
pub struct Workspace {
    pub original: PathBuf,
    pub staged: PathBuf,
    baseline: BTreeMap<PathBuf, String>,
}

impl Workspace {
    pub fn create(original: &Path, staged: &Path, control: &Control) -> Result<Self> {
        let original = durable::canonical(original)?;
        let staged = durable::absolute(staged)?;
        if staged.starts_with(&original) || original.starts_with(&staged) {
            return Err(Error::new(
                ErrorCode::Invalid,
                "staging must be outside the pack",
            ));
        }
        if staged.exists() {
            return Err(Error::new(ErrorCode::Invalid, "staging already exists"));
        }
        fs::create_dir_all(&staged)?;
        let baseline = inventory(&original, control)?;
        let required = baseline.keys().try_fold(0u64, |sum, rel| {
            fs::metadata(original.join(rel)).map(|m| sum.saturating_add(m.len()))
        })?;
        if fs2::available_space(&staged)? < required.saturating_mul(3).saturating_add(1024 * 1024) {
            return Err(Error::new(
                ErrorCode::Io,
                "insufficient space for staging and recovery backups",
            ));
        }
        control.emit(Event::Phase("snapshotting".into()));
        for (index, (rel, expected)) in baseline.iter().enumerate() {
            copy(&original.join(rel), &staged.join(rel), control)?;
            if durable::fingerprint(&staged.join(rel))?.as_ref() != Some(expected)
                || durable::fingerprint(&original.join(rel))?.as_ref() != Some(expected)
            {
                return Err(Error::new(
                    ErrorCode::Conflict,
                    original.join(rel).display().to_string(),
                ));
            }
            control.emit(Event::Progress {
                label: rel.display().to_string(),
                current: (index + 1) as u64,
                total: Some(baseline.len() as u64),
            });
        }
        Ok(Self {
            original,
            staged,
            baseline,
        })
    }

    pub fn changes(&self, control: &Control) -> Result<Vec<Change>> {
        control.check()?;
        if inventory(&self.original, control)? != self.baseline {
            return Err(Error::new(
                ErrorCode::Conflict,
                format!("workspace changed: {}", self.original.display()),
            ));
        }
        let updated = inventory(&self.staged, control)?;
        let mut paths: Vec<_> = self
            .baseline
            .keys()
            .chain(updated.keys())
            .cloned()
            .collect();
        paths.sort();
        paths.dedup();
        Ok(paths
            .into_iter()
            .filter(|p| self.baseline.get(p) != updated.get(p))
            .map(|rel| Change {
                target: self.original.join(&rel),
                expected: self.baseline.get(&rel).cloned(),
                source: updated.contains_key(&rel).then(|| self.staged.join(&rel)),
            })
            .collect())
    }
}

fn inventory(root: &Path, control: &Control) -> Result<BTreeMap<PathBuf, String>> {
    let mut files = BTreeMap::new();
    if !root.exists() {
        return Ok(files);
    }
    walk(root, root, control, &mut files)?;
    Ok(files)
}

fn walk(
    root: &Path,
    current: &Path,
    control: &Control,
    files: &mut BTreeMap<PathBuf, String>,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        control.check()?;
        let entry = entry?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?;
        if current == root && (entry.file_name() == ".git" || entry.file_name() == "target") {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(Error::new(
                ErrorCode::Invalid,
                format!("symlink: {}", path.display()),
            ));
        }
        if kind.is_dir() {
            walk(root, &path, control, files)?;
        } else {
            let stamp = durable::fingerprint(&path)?
                .ok_or_else(|| Error::new(ErrorCode::Conflict, path.display().to_string()))?;
            files.insert(rel.to_path_buf(), stamp);
        }
    }
    Ok(())
}

fn copy(source: &Path, target: &Path, control: &Control) -> Result<()> {
    fs::create_dir_all(target.parent().unwrap())?;
    let mut input = fs::File::open(source)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let mut buffer = vec![0; 256 * 1024];
    loop {
        control.check()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
    }
    output.sync_all()?;
    fs::set_permissions(target, fs::metadata(source)?.permissions())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_is_isolated_and_detects_external_changes() {
        let dir = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(dir.join("pack")).unwrap();
        let dir = fs::canonicalize(dir).unwrap();
        fs::write(dir.join("pack/manual.jar"), b"manual").unwrap();
        fs::write(dir.join("pack/metadata"), b"old").unwrap();
        let workspace =
            Workspace::create(&dir.join("pack"), &dir.join("stage"), &Control::default()).unwrap();
        fs::write(dir.join("stage/metadata"), b"new").unwrap();
        assert_eq!(fs::read(dir.join("pack/metadata")).unwrap(), b"old");
        let changes = workspace.changes(&Control::default()).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(changes[0].target.ends_with("metadata"));
        fs::write(dir.join("pack/manual.jar"), b"external").unwrap();
        assert_eq!(
            workspace.changes(&Control::default()).err().unwrap().code,
            ErrorCode::Conflict
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
