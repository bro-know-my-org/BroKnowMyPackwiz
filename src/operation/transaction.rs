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
}
#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    version: u32,
    state: State,
    entries: Vec<Entry>,
    directories: Vec<PathBuf>,
    #[serde(default)]
    requested_directories: Vec<PathBuf>,
    conflicts: Vec<PathBuf>,
}

pub struct Transaction {
    pub directory: PathBuf,
    journal: Journal,
}

impl Transaction {
    pub fn prepare(state: &Path, changes: Vec<Change>, control: &Control) -> Result<Self> {
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
                conflicts: Vec::new(),
            },
        };
        txn.save()?;
        let mut targets = BTreeSet::new();
        control.emit(Event::Phase("preparing".into()));
        for change in changes {
            control.check()?;
            let target = durable::absolute(&change.target)?;
            if !targets.insert(target.clone()) {
                return Err(Error::new(
                    ErrorCode::Invalid,
                    "duplicate transaction target",
                ));
            }
            let before = durable::fingerprint(&target)?;
            if before != change.expected {
                return Err(conflict(&target));
            }
            let i = txn.journal.entries.len();
            if before.is_some() {
                fs::copy(&target, txn.backup(i))?;
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
                        return Err(Error::new(ErrorCode::Invalid, "missing staged source"));
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
        txn.save()?;
        durable::sync_dir(txn.directory.parent().unwrap())?;
        Ok(txn)
    }

    pub fn open(directory: &Path) -> Result<Self> {
        let journal: Journal = serde_json::from_slice(&fs::read(directory.join("journal.json"))?)
            .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?;
        if journal.version != 1 {
            return Err(Error::new(
                ErrorCode::Invalid,
                "unsupported transaction version",
            ));
        }
        for entry in &journal.entries {
            if !entry.target.is_absolute() {
                return Err(Error::new(
                    ErrorCode::Invalid,
                    "non-absolute transaction target",
                ));
            }
        }
        if journal
            .directories
            .iter()
            .chain(&journal.requested_directories)
            .any(|path| !path.is_absolute())
        {
            return Err(Error::new(
                ErrorCode::Invalid,
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
            return Err(Error::new(
                ErrorCode::Invalid,
                "transaction is not prepared",
            ));
        }
        let mut directories = BTreeSet::new();
        for path in paths {
            let path = durable::absolute(&path)?;
            if path.exists() {
                return Err(conflict(&path));
            }
            directories.insert(path);
        }
        self.journal.requested_directories = directories.into_iter().collect();
        self.save()
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
            })
            .collect()
    }

    /// Explicit resolution retains original backups and archives an external
    /// version before restoring over it. A saved decision survives interruption.
    pub fn resolve_all(&mut self, keep_external: bool) -> Result<()> {
        if keep_external {
            self.journal
                .directories
                .retain(|path| !self.journal.conflicts.contains(path));
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
            .collect()
    }
    fn backup(&self, i: usize) -> PathBuf {
        self.directory.join("before").join(i.to_string())
    }
    fn staged(&self, i: usize) -> PathBuf {
        self.directory.join("after").join(i.to_string())
    }
    fn save(&self) -> Result<()> {
        durable::write(
            &self.directory.join("journal.json"),
            &serde_json::to_vec_pretty(&self.journal)
                .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?,
        )
    }

    pub fn commit(&mut self, control: &Control) -> Result<()> {
        if self.journal.state != State::Prepared {
            return Err(Error::new(
                ErrorCode::Invalid,
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
        for path in self.journal.requested_directories.clone() {
            control.check()?;
            durable::absolute(&path)?;
            if path.exists() {
                return Err(conflict(&path));
            }
            // ensure_parents only examines the parent; no marker is written.
            self.ensure_parents(&path.join("unused"))?;
        }
        for i in 0..self.journal.entries.len() {
            control.check()?;
            let entry = &self.journal.entries[i];
            let target = entry.target.clone();
            durable::absolute(&target)?;
            if durable::fingerprint(&target)? != entry.before {
                return Err(conflict(&target));
            }
            if entry.before == entry.after {
                continue;
            }
            if entry.after.is_some() && durable::fingerprint(&self.staged(i))? != entry.after {
                return Err(Error::new(ErrorCode::Invalid, "staged content changed"));
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
                current: (i + 1) as u64,
                total: Some(self.journal.entries.len() as u64),
            });
        }
        control.check()?;
        self.journal.state = State::Committed;
        if let Err(error) = self.save() {
            self.journal.state = State::Committing;
            return Err(error);
        }
        Ok(())
    }

    fn ensure_parents(&mut self, target: &Path) -> Result<()> {
        let mut parent = target.parent().unwrap();
        let mut missing = Vec::new();
        while !parent.exists() {
            missing.push(parent.to_path_buf());
            parent = parent
                .parent()
                .ok_or_else(|| Error::new(ErrorCode::Invalid, "missing root"))?;
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
            return Err(Error::new(
                ErrorCode::Invalid,
                "cannot roll back committed task",
            ));
        }
        if self.journal.state == State::RolledBack {
            return Ok(());
        }
        self.journal.state = State::RollingBack;
        self.journal.conflicts.clear();
        self.save()?;
        for i in (0..self.journal.entries.len()).rev() {
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
                        return Err(Error::new(
                            ErrorCode::Conflict,
                            format!("backup changed: {}", self.backup(i).display()),
                        ));
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
        if !self.journal.conflicts.is_empty() {
            self.journal.state = State::Conflict;
            self.save()?;
            return Err(Error::new(
                ErrorCode::Conflict,
                format!("{}", self.directory.display()),
            ));
        }
        for path in self.journal.directories.iter().rev() {
            match fs::remove_dir(path) {
                Ok(()) => durable::sync_dir(path.parent().unwrap())?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                    self.journal.conflicts.push(path.clone());
                }
                Err(e) => return Err(e.into()),
            }
        }
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
}

fn conflict(path: &Path) -> Error {
    Error::new(ErrorCode::Conflict, path.display().to_string())
}

#[cfg(test)]
#[path = "transaction_tests.rs"]
mod tests;
