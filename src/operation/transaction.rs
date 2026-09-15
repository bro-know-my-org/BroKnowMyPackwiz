use super::{Control, Error, ErrorCode, Event, Result, durable};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// All sources are private staged files. `None` means delete the target.
pub struct Change {
    pub target: PathBuf,
    pub expected: Option<String>,
    pub source: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Prepared,
    Committing,
    RollingBack,
    Committed,
    RolledBack,
    Conflict,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Step {
    Pending,
    Applying,
    Applied,
    Restoring,
    Restored,
}
#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    target: PathBuf,
    before: Option<String>,
    after: Option<String>,
    step: Step,
    #[serde(default)]
    override_enabled: bool,
    #[serde(default)]
    override_current: Option<String>,
}

pub struct ConflictFile {
    pub target: PathBuf,
    pub backup: PathBuf,
    pub staged: PathBuf,
    pub directory: bool,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Journal {
    version: u32,
    pub(super) state: State,
    entries: Vec<Entry>,
    directories: Vec<PathBuf>,
    #[serde(default)]
    requested_directories: Vec<PathBuf>,
    #[serde(default)]
    pub(super) requested_modes: Vec<(PathBuf, u32)>,
    pub(super) conflicts: Vec<PathBuf>,
    #[serde(default)]
    pub(super) directory_changes: Vec<super::directory::Entry>,
}

pub struct Transaction {
    pub directory: PathBuf,
    pub(super) journal: Journal,
}

impl Transaction {
    pub fn prepare(state: &Path, changes: Vec<Change>, control: &Control) -> Result<Self> {
        Self::prepare_tree(state, changes, Vec::new(), Vec::new(), control)
    }

    pub fn prepare_tree(
        state: &Path,
        changes: Vec<Change>,
        directory_changes: Vec<super::directory::Change>,
        created: Vec<(PathBuf, u32)>,
        control: &Control,
    ) -> Result<Self> {
        let deleted = changes
            .iter()
            .filter(|change| change.expected.is_some() && change.source.is_none())
            .map(|change| durable::absolute(&change.target))
            .collect::<Result<Vec<_>>>()?;
        let directory = state.join("transactions").join(durable::unique_id());
        fs::create_dir_all(directory.join("before"))?;
        fs::create_dir(directory.join("after"))?;
        let mut txn = Self {
            directory,
            journal: Journal {
                version: 1,
                state: State::Prepared,
                entries: Vec::new(),
                directories: Vec::new(),
                requested_directories: Vec::new(),
                requested_modes: Vec::new(),
                conflicts: Vec::new(),
                directory_changes: Vec::new(),
            },
        };
        txn.save()?;
        txn.change_directories(directory_changes)?;
        let mut targets = BTreeSet::new();
        control.emit(Event::Phase("preparing".into()));
        for change in changes {
            control.check()?;
            let target = planned_path(&change.target, &deleted)?;
            if !targets.insert(target.clone()) {
                return Err(Error::named(
                    ErrorCode::Invalid,
                    "duplicate_transaction_target",
                    "duplicate transaction target",
                ));
            }
            let before = if change.expected.is_none()
                && (txn.removes_directory(&target)
                    || deleted
                        .iter()
                        .any(|parent| parent != &target && target.starts_with(parent)))
            {
                None
            } else {
                durable::fingerprint(&target)?
            };
            if before != change.expected {
                return Err(conflict(&target));
            }
            let i = txn.journal.entries.len();
            if before.is_some() {
                #[cfg(test)]
                super::faults::check(super::faults::Point::BackupBefore, &txn.backup(i))?;
                fs::copy(&target, txn.backup(i))?;
                #[cfg(test)]
                super::faults::check(super::faults::Point::BackupCopied, &txn.backup(i))?;
                fs::File::open(txn.backup(i))?.sync_all()?;
                if durable::fingerprint(&txn.backup(i))? != before
                    || durable::fingerprint(&target)? != before
                {
                    return Err(conflict(&target));
                }
            }
            let after = match change.source {
                Some(source) => {
                    let stamp = durable::fingerprint(&source)?;
                    if stamp.is_none() {
                        return Err(Error::named(
                            ErrorCode::Invalid,
                            "missing_staged_source",
                            "missing staged source",
                        ));
                    }
                    fs::copy(&source, txn.staged(i))?;
                    fs::File::open(txn.staged(i))?.sync_all()?;
                    if durable::fingerprint(&txn.staged(i))? != stamp {
                        return Err(conflict(&source));
                    }
                    stamp
                }
                None => None,
            };
            txn.journal.entries.push(Entry {
                target,
                before,
                after,
                step: Step::Pending,
                override_enabled: false,
                override_current: None,
            });
        }
        durable::sync_dir(&txn.directory.join("before"))?;
        durable::sync_dir(&txn.directory.join("after"))?;
        txn.create_directories_with_modes(created)?;
        txn.save()?;
        durable::sync_dir(txn.directory.parent().unwrap())?;
        Ok(txn)
    }

    pub fn open(directory: &Path) -> Result<Self> {
        let journal: Journal = serde_json::from_slice(&fs::read(directory.join("journal.json"))?)
            .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?;
        if journal.version != 1 {
            return Err(Error::named(
                ErrorCode::Invalid,
                "unsupported_transaction_version",
                "unsupported transaction version",
            ));
        }
        for entry in &journal.entries {
            if !entry.target.is_absolute() {
                return Err(Error::named(
                    ErrorCode::Invalid,
                    "transaction_target_not_absolute",
                    "non-absolute transaction target",
                ));
            }
        }
        if journal
            .directories
            .iter()
            .chain(&journal.requested_directories)
            .chain(journal.directory_changes.iter().map(|entry| &entry.target))
            .chain(journal.requested_modes.iter().map(|(path, _)| path))
            .any(|path| !path.is_absolute())
        {
            return Err(Error::named(
                ErrorCode::Invalid,
                "transaction_directory_not_absolute",
                "non-absolute transaction directory",
            ));
        }
        Ok(Self {
            directory: directory.to_path_buf(),
            journal,
        })
    }

    pub fn state(&self) -> State {
        self.journal.state
    }
    pub fn create_directories(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        if self.journal.state != State::Prepared {
            return Err(Error::named(
                ErrorCode::Invalid,
                "transaction_not_prepared",
                "transaction is not prepared",
            ));
        }
        let mut directories = BTreeSet::new();
        let deleted = self.deleted_files();
        for path in paths {
            let path = planned_path(&path, &deleted)?;
            if path.exists() && !deleted.contains(&path) {
                return Err(conflict(&path));
            }
            directories.insert(path);
        }
        self.journal.requested_directories = directories.into_iter().collect();
        self.save()
    }
    pub fn create_directories_with_modes(&mut self, paths: Vec<(PathBuf, u32)>) -> Result<()> {
        self.create_directories(paths.iter().map(|(path, _)| path.clone()).collect())?;
        self.journal.requested_modes = paths
            .into_iter()
            .map(|(path, mode)| planned_path(&path, &self.deleted_files()).map(|path| (path, mode)))
            .collect::<Result<_>>()?;
        self.save()
    }
    fn deleted_files(&self) -> Vec<PathBuf> {
        self.journal
            .entries
            .iter()
            .filter(|entry| entry.before.is_some() && entry.after.is_none())
            .map(|entry| entry.target.clone())
            .collect()
    }
    pub(super) fn new_file_targets(&self) -> Vec<PathBuf> {
        self.journal
            .entries
            .iter()
            .filter(|entry| entry.before.is_none() && entry.after.is_some())
            .map(|entry| entry.target.clone())
            .collect()
    }
    pub fn conflicts(&self) -> &[PathBuf] {
        &self.journal.conflicts
    }

    pub fn conflict_files(&self) -> Vec<ConflictFile> {
        self.journal
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.journal.conflicts.contains(&entry.target))
            .map(|(i, entry)| ConflictFile {
                target: entry.target.clone(),
                backup: self.backup(i),
                staged: self.staged(i),
                directory: false,
            })
            .chain(
                self.journal
                    .conflicts
                    .iter()
                    .filter(|path| {
                        self.journal.directories.contains(path)
                            || self
                                .journal
                                .directory_changes
                                .iter()
                                .any(|entry| &entry.target == *path)
                    })
                    .map(|path| ConflictFile {
                        target: path.clone(),
                        backup: self.directory.join("before"),
                        staged: self.directory.join("after"),
                        directory: true,
                    }),
            )
            .collect()
    }

    /// Explicit resolution retains original backups and archives an external
    /// version before restoring over it. A saved decision survives interruption.
    pub fn resolve_all(&mut self, keep_external: bool) -> Result<()> {
        self.resolve_directory_conflicts(keep_external)?;
        self.save()?;
        if keep_external {
            self.journal.directories.retain(|path| {
                !self
                    .journal
                    .conflicts
                    .iter()
                    .any(|conflict| conflict.starts_with(path))
            });
            self.save()?;
        }
        for i in 0..self.journal.entries.len() {
            let target = self.journal.entries[i].target.clone();
            if !self.journal.conflicts.contains(&target) {
                continue;
            }
            if keep_external {
                self.journal.entries[i].step = Step::Restored;
                self.save()?;
                continue;
            }
            // A replacement file can temporarily hide an original directory's
            // children. Resolve that parent's file entry first; these children
            // have no current file to archive and are restored in rollback.
            if self.journal.entries.iter().any(|parent| {
                parent.before.is_none()
                    && parent.after.is_some()
                    && parent.target != target
                    && target.starts_with(&parent.target)
                    && self.removes_directory(&parent.target)
                    && fs::symlink_metadata(&parent.target).is_ok_and(|metadata| {
                        metadata.is_file() && !metadata.file_type().is_symlink()
                    })
            }) {
                continue;
            }
            durable::absolute(&target)?;
            let current = durable::fingerprint(&target)?;
            if current.is_some() {
                let copy = self
                    .directory
                    .join(format!("external-{i}-{}", durable::unique_id()));
                fs::copy(&target, &copy)?;
                fs::File::open(&copy)?.sync_all()?;
                durable::sync_dir(&self.directory)?;
                if durable::fingerprint(&copy)? != current
                    || durable::fingerprint(&target)? != current
                {
                    return Err(conflict(&target));
                }
            }
            self.journal.entries[i].override_enabled = true;
            self.journal.entries[i].override_current = current;
            self.save()?;
        }
        self.rollback()
    }
    pub fn targets(&self) -> Vec<PathBuf> {
        self.journal
            .entries
            .iter()
            .map(|e| e.target.clone())
            .chain(self.journal.directories.iter().cloned())
            .chain(self.journal.requested_directories.iter().cloned())
            .chain(
                self.journal
                    .directory_changes
                    .iter()
                    .map(|entry| entry.target.clone()),
            )
            .collect()
    }
    fn backup(&self, i: usize) -> PathBuf {
        self.directory.join("before").join(i.to_string())
    }
    fn staged(&self, i: usize) -> PathBuf {
        self.directory.join("after").join(i.to_string())
    }
    pub(super) fn save(&self) -> Result<()> {
        durable::write(
            &self.directory.join("journal.json"),
            &serde_json::to_vec_pretty(&self.journal)
                .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?,
        )
    }

    pub fn commit(&mut self, control: &Control) -> Result<()> {
        if self.journal.state != State::Prepared {
            return Err(Error::named(
                ErrorCode::Invalid,
                "transaction_not_prepared",
                "transaction is not prepared",
            ));
        }
        if let Err(error) = self.apply(control) {
            return match self.rollback() {
                Ok(()) => Err(error),
                Err(recovery) => Err(recovery),
            };
        }
        Ok(())
    }

    fn apply(&mut self, control: &Control) -> Result<()> {
        self.journal.state = State::Committing;
        self.save()?;
        control.emit(Event::Phase("committing".into()));
        for i in 0..self.journal.entries.len() {
            if self.journal.entries[i].after.is_none() {
                self.apply_file(i, control)?;
            }
        }
        self.apply_directory_removals(control)?;
        for path in self.journal.requested_directories.clone() {
            control.check()?;
            durable::absolute(&path)?;
            if path.exists() {
                return Err(conflict(&path));
            }
            // ensure_parents only examines the parent; no marker is written.
            self.ensure_parents(&path.join("unused"))?;
        }
        self.prepare_created_modes()?;
        for i in 0..self.journal.entries.len() {
            if self.journal.entries[i].after.is_some() {
                self.apply_file(i, control)?;
            }
        }
        self.apply_directory_modes(control)?;
        control.check()?;
        self.journal.state = State::Committed;
        if let Err(error) = self.save() {
            self.journal.state = State::Committing;
            return Err(error);
        }
        Ok(())
    }

    fn apply_file(&mut self, i: usize, control: &Control) -> Result<()> {
        control.check()?;
        let entry = &self.journal.entries[i];
        let target = entry.target.clone();
        durable::absolute(&target)?;
        if durable::fingerprint(&target)? != entry.before {
            return Err(conflict(&target));
        }
        if entry.before == entry.after {
            return Ok(());
        }
        if entry.after.is_some() && durable::fingerprint(&self.staged(i))? != entry.after {
            return Err(Error::named(
                ErrorCode::Invalid,
                "staged_content_changed",
                "staged content changed",
            ));
        }
        self.journal.entries[i].step = Step::Applying;
        self.save()?;
        self.ensure_parents(&target)?;
        if self.journal.entries[i].after.is_some() {
            durable::replace(&self.staged(i), &target)?;
        } else {
            fs::remove_file(&target)?;
            durable::sync_dir(target.parent().unwrap())?;
        }
        self.journal.entries[i].step = Step::Applied;
        self.save()?;
        control.emit(Event::Progress {
            label: target.display().to_string(),
            current: self
                .journal
                .entries
                .iter()
                .filter(|entry| entry.step == Step::Applied)
                .count() as u64,
            total: Some(self.journal.entries.len() as u64),
        });
        Ok(())
    }

    fn ensure_parents(&mut self, target: &Path) -> Result<()> {
        let mut parent = target.parent().unwrap();
        let mut missing = Vec::new();
        while !parent.exists() {
            missing.push(parent.to_path_buf());
            parent = parent
                .parent()
                .ok_or_else(|| Error::named(ErrorCode::Invalid, "missing_root", "missing root"))?;
        }
        for path in missing.into_iter().rev() {
            self.journal.directories.push(path.clone());
            self.save()?;
            fs::create_dir(&path)?;
            durable::sync_dir(path.parent().unwrap())?;
        }
        Ok(())
    }

    pub fn rollback(&mut self) -> Result<()> {
        if self.journal.state == State::Committed {
            return Err(Error::named(
                ErrorCode::Invalid,
                "committed_task_rollback",
                "cannot roll back committed task",
            ));
        }
        if self.journal.state == State::RolledBack {
            return Ok(());
        }
        self.journal.state = State::RollingBack;
        self.journal.conflicts.clear();
        self.save()?;
        self.restore_directory_modes()?;
        self.restore_files(false)?;
        self.remove_created_directories()?;
        self.restore_removed_directories()?;
        self.restore_files(true)?;
        if !self.journal.conflicts.is_empty() {
            self.journal.state = State::Conflict;
            self.save()?;
            return Err(Error::new(
                ErrorCode::Conflict,
                self.directory.display().to_string(),
            ));
        }
        self.journal.state = State::RolledBack;
        self.save()
    }
    fn restore_files(&mut self, existing: bool) -> Result<()> {
        for i in (0..self.journal.entries.len()).rev() {
            if self.journal.entries[i].before.is_some() != existing {
                continue;
            }
            if matches!(self.journal.entries[i].step, Step::Pending | Step::Restored) {
                continue;
            }
            let target = self.journal.entries[i].target.clone();
            let current = durable::absolute(&target).and_then(|_| durable::fingerprint(&target));
            let current = match current {
                Ok(value) => value,
                Err(_) => {
                    self.journal.conflicts.push(target);
                    continue;
                }
            };
            if current != self.journal.entries[i].before
                && current != self.journal.entries[i].after
                && !(self.journal.entries[i].override_enabled
                    && current == self.journal.entries[i].override_current)
            {
                self.journal.conflicts.push(target);
                continue;
            }
            self.journal.entries[i].step = Step::Restoring;
            self.save()?;
            if current != self.journal.entries[i].before {
                if self.journal.entries[i].before.is_some() {
                    if durable::fingerprint(&self.backup(i))? != self.journal.entries[i].before {
                        return Err(Error::named(
                            ErrorCode::Conflict,
                            "backup_changed",
                            format!("backup changed: {}", self.backup(i).display()),
                        )
                        .context(self.backup(i).display().to_string()));
                    }
                    durable::replace(&self.backup(i), &target)?;
                } else {
                    fs::remove_file(&target)?;
                    durable::sync_dir(target.parent().unwrap())?;
                }
            }
            self.journal.entries[i].step = Step::Restored;
            self.save()?;
        }
        Ok(())
    }

    fn remove_created_directories(&mut self) -> Result<()> {
        for path in self.journal.directories.clone().into_iter().rev() {
            if !self.created_mode_matches(&path) {
                self.journal.conflicts.push(path.clone());
                continue;
            }
            if durable::absolute(&path).is_err() {
                self.journal.conflicts.push(path.clone());
                continue;
            }
            match fs::remove_dir(&path) {
                Ok(()) => {
                    durable::sync_dir(path.parent().unwrap())?;
                    self.journal.directories.retain(|pending| pending != &path);
                    self.save()?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    self.journal.directories.retain(|pending| pending != &path);
                    self.save()?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                    self.journal.conflicts.push(path.clone());
                }
                Err(_) => {
                    self.journal.conflicts.push(path.clone());
                }
            }
        }
        Ok(())
    }
}

fn conflict(path: &Path) -> Error {
    Error::new(ErrorCode::Conflict, path.display().to_string())
}

/// A future descendant of a file can only be planned if that exact file is
/// included in this transaction's verified deletion set. No symlink exception.
fn planned_path(path: &Path, deleted: &[PathBuf]) -> Result<PathBuf> {
    match durable::absolute(path) {
        Ok(path) => Ok(path),
        Err(error) => {
            if !path.is_absolute()
                || path
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                return Err(error);
            }
            for parent in deleted {
                if parent != path && path.starts_with(parent) {
                    durable::absolute(parent)?;
                    if durable::fingerprint(parent)?.is_some() {
                        return Ok(parent.join(path.strip_prefix(parent).unwrap()));
                    }
                }
            }
            Err(error)
        }
    }
}

#[cfg(test)]
#[path = "transaction_tests.rs"]
mod tests;
