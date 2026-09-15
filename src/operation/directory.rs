//! Directory metadata transitions complement the transaction's file entries.
use super::{Control, Error, ErrorCode, Result, durable};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn mode(path: &Path) -> Result<Option<u32>> {
    durable::absolute(path)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::key(ErrorCode::Conflict, "directory_changed")
            .context(path.display().to_string()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(Some(metadata.permissions().mode() & 0o7777))
    }
    #[cfg(not(unix))]
    {
        Ok(Some(u32::from(metadata.permissions().readonly())))
    }
}

pub fn set_mode(path: &Path, mode: u32) -> Result<()> {
    durable::absolute(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(mode != 0);
        fs::set_permissions(path, permissions)?;
    }
    durable::sync_dir(path)?;
    Ok(())
}

pub struct Change {
    pub target: PathBuf,
    pub before: u32,
    pub after: Option<u32>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
enum Step {
    Pending,
    Applying,
    Applied,
    Restoring,
    Restored,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Entry {
    pub target: PathBuf,
    before: u32,
    after: Option<u32>,
    step: Step,
    #[serde(default)]
    override_enabled: bool,
    #[serde(default)]
    override_current: Option<u32>,
}

impl Entry {
    pub fn prepare(change: Change) -> Result<Self> {
        let target = durable::absolute(&change.target)?;
        if mode(&target)? != Some(change.before) {
            return Err(conflict(&target));
        }
        Ok(Self {
            target,
            before: change.before,
            after: change.after,
            step: Step::Pending,
            override_enabled: false,
            override_current: None,
        })
    }
}

fn conflict(path: &Path) -> Error {
    Error::key(ErrorCode::Conflict, "directory_changed").context(path.display().to_string())
}

impl super::transaction::Transaction {
    pub(super) fn prepare_created_modes(&mut self) -> Result<()> {
        for (target, after) in &self.journal.requested_modes {
            let before = mode(target)?.ok_or_else(|| conflict(target))?;
            self.journal.directory_changes.push(Entry::prepare(Change {
                target: target.clone(),
                before,
                after: Some(*after),
            })?);
        }
        self.journal
            .directory_changes
            .sort_by(|a, b| b.target.cmp(&a.target));
        self.save()
    }

    pub(super) fn created_mode_matches(&self, path: &Path) -> bool {
        self.journal
            .directory_changes
            .iter()
            .find(|entry| entry.target == path)
            .is_none_or(|entry| {
                mode(path).is_ok_and(|mode| mode.is_none() || mode == Some(entry.before))
            })
    }

    pub fn change_directories(&mut self, changes: Vec<Change>) -> Result<()> {
        if self.journal.state != super::transaction::State::Prepared {
            return Err(Error::key(ErrorCode::Invalid, "transaction_not_prepared"));
        }
        let mut entries = changes
            .into_iter()
            .map(Entry::prepare)
            .collect::<Result<Vec<_>>>()?;
        // Children are removed before their parents; recovery reverses this.
        entries.sort_by(|a, b| b.target.cmp(&a.target));
        if entries
            .windows(2)
            .any(|pair| pair[0].target == pair[1].target)
        {
            return Err(Error::key(ErrorCode::Invalid, "duplicate_batch_target"));
        }
        self.journal.directory_changes = entries;
        self.save()
    }

    pub(super) fn apply_directories(&mut self, control: &Control) -> Result<()> {
        for index in 0..self.journal.directory_changes.len() {
            control.check()?;
            let entry = &self.journal.directory_changes[index];
            let target = entry.target.clone();
            let after = entry.after;
            if mode(&target)? != Some(entry.before) {
                return Err(conflict(&target));
            }
            self.journal.directory_changes[index].step = Step::Applying;
            self.save()?;
            match after {
                Some(value) => set_mode(&target, value)?,
                None => {
                    fs::remove_dir(&target)?;
                    durable::sync_dir(target.parent().unwrap())?;
                }
            }
            self.journal.directory_changes[index].step = Step::Applied;
            self.save()?;
        }
        Ok(())
    }

    pub(super) fn restore_directories(&mut self) -> Result<()> {
        for index in (0..self.journal.directory_changes.len()).rev() {
            let entry = &self.journal.directory_changes[index];
            if matches!(entry.step, Step::Pending | Step::Restored) {
                continue;
            }
            let target = entry.target.clone();
            let before = entry.before;
            let current = match mode(&target) {
                Ok(value)
                    if value == Some(before)
                        || value == entry.after
                        || (entry.override_enabled && value == entry.override_current) =>
                {
                    value
                }
                _ => {
                    self.journal.conflicts.push(target);
                    continue;
                }
            };
            self.journal.directory_changes[index].step = Step::Restoring;
            self.save()?;
            if current != Some(before) {
                if current.is_none() {
                    fs::create_dir(&target)?;
                    durable::sync_dir(target.parent().unwrap())?;
                }
                set_mode(&target, before)?;
            }
            self.journal.directory_changes[index].step = Step::Restored;
            self.save()?;
        }
        Ok(())
    }

    pub(super) fn resolve_directory_conflicts(&mut self, keep: bool) -> Result<()> {
        for (index, entry) in self.journal.directory_changes.iter_mut().enumerate() {
            if self.journal.conflicts.contains(&entry.target) {
                if keep {
                    entry.step = Step::Restored;
                } else {
                    let current = mode(&entry.target)?;
                    durable::write(
                        &self.directory.join(format!(
                            "external-directory-{index}-{}.json",
                            durable::unique_id()
                        )),
                        &serde_json::to_vec(&current)
                            .map_err(|error| Error::new(ErrorCode::Invalid, error.to_string()))?,
                    )?;
                    entry.override_enabled = true;
                    entry.override_current = current;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::transaction::{State, Transaction};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(durable::unique_id());
            fs::create_dir_all(root.join("tree/empty")).unwrap();
            Self(fs::canonicalize(root).unwrap())
        }
        fn transaction(&self) -> Transaction {
            let mut tx =
                Transaction::prepare(&self.0.join("state"), Vec::new(), &Control::default())
                    .unwrap();
            tx.change_directories(
                ["tree", "tree/empty"]
                    .iter()
                    .map(|relative| {
                        let target = self.0.join(relative);
                        Change {
                            before: mode(&target).unwrap().unwrap(),
                            target,
                            after: None,
                        }
                    })
                    .collect(),
            )
            .unwrap();
            tx
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn removes_nested_empty_directories_and_recovers_after_later_failure() {
        let f = Fixture::new();
        let mut tx = f.transaction();
        fs::write(f.0.join("tree/external"), "external").unwrap();
        assert!(tx.commit(&Control::default()).is_err());
        assert!(f.0.join("tree/empty").is_dir());
        assert_eq!(
            fs::read_to_string(f.0.join("tree/external")).unwrap(),
            "external"
        );
        fs::remove_file(f.0.join("tree/external")).unwrap();
        let mut tx = f.transaction();
        tx.commit(&Control::default()).unwrap();
        assert!(!f.0.join("tree").exists());
    }

    #[test]
    fn restart_recreates_deleted_parents_before_their_children() {
        let f = Fixture::new();
        let mut tx = f.transaction();
        tx.journal.state = State::Committing;
        tx.save().unwrap();
        tx.apply_directories(&Control::default()).unwrap();
        assert!(!f.0.join("tree").exists());
        let mut reopened = Transaction::open(&tx.directory).unwrap();
        reopened.rollback().unwrap();
        assert!(f.0.join("tree/empty").is_dir());
        Transaction::open(&tx.directory)
            .unwrap()
            .rollback()
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn permission_recovery_preserves_external_permissions_until_explicit_resolution() {
        let f = Fixture::new();
        let target = f.0.join("tree");
        set_mode(&target, 0o700).unwrap();
        let mut tx =
            Transaction::prepare(&f.0.join("state"), Vec::new(), &Control::default()).unwrap();
        tx.change_directories(vec![Change {
            target: target.clone(),
            before: 0o700,
            after: Some(0o755),
        }])
        .unwrap();
        tx.journal.state = State::Committing;
        tx.apply_directories(&Control::default()).unwrap();
        assert_eq!(mode(&target).unwrap(), Some(0o755));
        Transaction::open(&tx.directory)
            .unwrap()
            .rollback()
            .unwrap();
        assert_eq!(mode(&target).unwrap(), Some(0o700));

        let mut tx =
            Transaction::prepare(&f.0.join("other"), Vec::new(), &Control::default()).unwrap();
        tx.change_directories(vec![Change {
            target: target.clone(),
            before: 0o700,
            after: Some(0o755),
        }])
        .unwrap();
        tx.journal.state = State::Committing;
        tx.apply_directories(&Control::default()).unwrap();
        set_mode(&target, 0o711).unwrap();
        assert_eq!(tx.rollback().unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(mode(&target).unwrap(), Some(0o711));
        assert!(
            tx.conflict_files()
                .iter()
                .any(|file| file.directory && file.target == target)
        );
        tx.resolve_all(true).unwrap();
        assert_eq!(mode(&target).unwrap(), Some(0o711));

        let mut tx =
            Transaction::prepare(&f.0.join("restore"), Vec::new(), &Control::default()).unwrap();
        tx.change_directories(vec![Change {
            target: target.clone(),
            before: 0o711,
            after: Some(0o755),
        }])
        .unwrap();
        tx.journal.state = State::Committing;
        tx.apply_directories(&Control::default()).unwrap();
        set_mode(&target, 0o700).unwrap();
        assert!(tx.rollback().is_err());
        tx.resolve_all(false).unwrap();
        assert_eq!(mode(&target).unwrap(), Some(0o711));
        assert!(fs::read_dir(&tx.directory).unwrap().any(|item| {
            item.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("external-directory-")
        }));
    }
}
