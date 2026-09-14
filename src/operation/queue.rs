use super::{
    Control, Error, ErrorCode, Event, Result, durable,
    edit::{self, Draft},
    lock::WriteLocks,
    transaction::{State, Transaction},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    Edit(Vec<Draft>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Waiting,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    NeedsRecovery,
    NeedsCheck,
    Conflict,
}
impl Status {
    pub fn key(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::NeedsRecovery => "recovering",
            Self::NeedsCheck => "needs_check",
            Self::Conflict => "conflict",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub label: String,
    pub request: Request,
    pub status: Status,
    pub error: Option<Error>,
}
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    root: PathBuf,
    tasks: Vec<Task>,
}
struct Running {
    index: usize,
    control: Control,
    result: Receiver<Result<()>>,
    recovery: bool,
}

pub struct Queue {
    pub root: PathBuf,
    pub state: PathBuf,
    pub tasks: Vec<Task>,
    pub paused: bool,
    pub logs: Vec<Event>,
    file: PathBuf,
    running: Option<Running>,
    events: Receiver<Event>,
    sender: mpsc::Sender<Event>,
    _lease: WriteLocks,
}

impl Queue {
    pub fn open(root: &Path, state: &Path) -> Result<Self> {
        let root = durable::canonical(root)?;
        let id = root_key(&root);
        let file = state.join("queues").join(format!("{id}.json"));
        let lease = WriteLocks::acquire(state, &[state.join("queue-leases").join(&id)])?;
        let mut tasks = if file.exists() {
            let saved: Saved = serde_json::from_slice(&fs::read(&file)?)
                .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?;
            if saved.version != 1 || saved.root != root {
                return Err(Error::new(
                    ErrorCode::Invalid,
                    "queue version or root mismatch",
                ));
            }
            saved.tasks
        } else {
            Vec::new()
        };
        for task in &mut tasks {
            if task.id.is_empty() || !task.id.chars().all(|c| c.is_ascii_digit() || c == '-') {
                return Err(Error::new(ErrorCode::Invalid, "invalid task id"));
            }
            if matches!(task.status, Status::Running | Status::Cancelling) {
                task.status = Status::NeedsRecovery;
            }
        }
        let (sender, events) = mpsc::channel();
        Ok(Self {
            root,
            state: state.into(),
            tasks,
            paused: true,
            logs: Vec::new(),
            file,
            running: None,
            events,
            sender,
            _lease: lease,
        })
    }

    pub fn save(&self) -> Result<()> {
        let saved = Saved {
            version: 1,
            root: self.root.clone(),
            tasks: self.tasks.clone(),
        };
        durable::write(
            &self.file,
            &serde_json::to_vec_pretty(&saved)
                .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?,
        )
    }

    pub fn enqueue(&mut self, label: &str, request: Request) -> Result<()> {
        if self.blocked() {
            return Err(Error::new(
                ErrorCode::Conflict,
                "resolve recovery before adding changes",
            ));
        }
        let was_empty = !self.tasks.iter().any(|t| t.status == Status::Waiting);
        self.tasks.push(Task {
            id: durable::unique_id(),
            label: label.into(),
            request,
            status: Status::Waiting,
            error: None,
        });
        if let Err(e) = self.save() {
            self.tasks.pop();
            return Err(e);
        }
        // A fresh queue starts on enqueue. An explicitly paused queue stays paused.
        if was_empty && self.tasks.len() == 1 {
            self.paused = false;
        }
        Ok(())
    }

    pub fn busy(&self) -> bool {
        self.running.is_some()
    }
    pub fn blocked(&self) -> bool {
        self.tasks
            .iter()
            .any(|t| matches!(t.status, Status::Conflict | Status::NeedsRecovery))
    }
    pub fn pending(&self) -> bool {
        self.busy()
            || self.tasks.iter().any(|t| {
                matches!(
                    t.status,
                    Status::Waiting | Status::NeedsRecovery | Status::Conflict
                )
            })
    }
    pub fn resume(&mut self) -> Result<()> {
        if self.blocked() {
            return Err(Error::new(ErrorCode::Conflict, "recovery unresolved"));
        }
        self.paused = false;
        Ok(())
    }
    pub fn cancel(&mut self, index: usize) -> Result<()> {
        let task = self
            .tasks
            .get_mut(index)
            .ok_or_else(|| Error::new(ErrorCode::Invalid, "unknown task"))?;
        match task.status {
            Status::Waiting => task.status = Status::Cancelled,
            Status::Running => {
                if let Some(running) = &self.running {
                    if running.recovery {
                        return Err(Error::new(
                            ErrorCode::Busy,
                            "recovery must finish before exit",
                        ));
                    }
                    running.control.cancel();
                }
                task.status = Status::Cancelling;
            }
            _ => return Err(Error::new(ErrorCode::Invalid, "task cannot be cancelled")),
        }
        self.paused = true;
        self.save()
    }
    pub fn cancel_current(&mut self) -> Result<()> {
        let index = self
            .running
            .as_ref()
            .ok_or_else(|| Error::new(ErrorCode::Invalid, "no running task"))?
            .index;
        self.cancel(index)
    }

    pub fn retry_recovery(&mut self, index: usize) -> Result<()> {
        if self.busy() {
            return Err(Error::new(ErrorCode::Busy, "task running"));
        }
        let task = self
            .tasks
            .get_mut(index)
            .ok_or_else(|| Error::new(ErrorCode::Invalid, "unknown task"))?;
        if task.status != Status::Conflict {
            return Err(Error::new(ErrorCode::Invalid, "no recovery conflict"));
        }
        task.status = Status::NeedsRecovery;
        self.save()
    }

    pub fn poll(&mut self) -> Result<bool> {
        self.logs.extend(self.events.try_iter());
        if self.logs.len() > 2000 {
            self.logs.drain(..self.logs.len() - 2000);
        }
        let result = self
            .running
            .as_ref()
            .and_then(|r| match r.result.try_recv() {
                Ok(value) => Some(value),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(Error::new(
                    ErrorCode::Interrupted,
                    "worker disconnected",
                ))),
                Err(mpsc::TryRecvError::Empty) => None,
            });
        let changed = result.is_some();
        if let Some(result) = result {
            let running = self.running.take().unwrap();
            let task = &mut self.tasks[running.index];
            task.status = match &result {
                Ok(()) if running.recovery => Status::NeedsCheck,
                Ok(()) => Status::Completed,
                Err(_) if running.recovery => Status::Conflict,
                Err(e) => match e.code {
                    ErrorCode::Cancelled => Status::Cancelled,
                    ErrorCode::Conflict => Status::Conflict,
                    ErrorCode::Interrupted => Status::NeedsRecovery,
                    _ => Status::Failed,
                },
            };
            task.error = result.err();
            if task.status != Status::Completed {
                self.paused = true;
            }
            self.save()?;
            let task = &self.tasks[running.index];
            if matches!(
                task.status,
                Status::Completed | Status::Failed | Status::Cancelled
            ) {
                let directory = self.state.join("tasks").join(&task.id);
                // Terminal tasks cannot be rolled back again. Do not retain a
                // full pack copy after every successful metadata edit.
                let _ = fs::remove_dir_all(directory.join("workspace"));
                let _ = fs::remove_dir_all(directory.join("transactions"));
            }
        }
        if self.running.is_none() {
            if let Some(index) = self
                .tasks
                .iter()
                .position(|t| t.status == Status::NeedsRecovery)
            {
                self.start(index, true, None)?;
            } else if !self.paused && !self.blocked() {
                if let Some(index) = self.tasks.iter().position(|t| t.status == Status::Waiting) {
                    self.start(index, false, None)?;
                }
            }
        }
        Ok(changed)
    }

    fn start(&mut self, index: usize, recovery: bool, resolution: Option<bool>) -> Result<()> {
        let previous = self.tasks[index].status;
        self.tasks[index].status = Status::Running;
        if let Err(e) = self.save() {
            self.tasks[index].status = previous;
            return Err(e);
        }
        let task = self.tasks[index].clone();
        let root = self.root.clone();
        let state = self.state.clone();
        let directory = self.state.join("tasks").join(&task.id);
        let control = Control::with_events(self.sender.clone());
        let worker_control = control.clone();
        let (tx, result) = mpsc::channel();
        self.running = Some(Running {
            index,
            control,
            result,
            recovery,
        });
        std::thread::spawn(move || {
            let result = if recovery {
                if let Some(keep) = resolution {
                    resolve(&root, &state, &directory, keep)
                } else {
                    recover(&root, &state, &directory)
                }
            } else {
                match task.request {
                    Request::Edit(drafts) => {
                        edit::execute(&root, &state, &directory, &drafts, &worker_control)
                    }
                }
            };
            let result = if !recovery && result.is_err() {
                match recover(&root, &state, &directory) {
                    Ok(()) => result,
                    Err(error) => Err(Error::new(ErrorCode::Conflict, error.to_string())),
                }
            } else {
                result
            };
            let _ = tx.send(result);
        });
        Ok(())
    }

    pub fn conflict_files(&self, index: usize) -> Result<Vec<super::transaction::ConflictFile>> {
        let task = self
            .tasks
            .get(index)
            .filter(|t| t.status == Status::Conflict)
            .ok_or_else(|| Error::new(ErrorCode::Invalid, "no task conflict"))?;
        let path = self.state.join("tasks").join(&task.id).join("transactions");
        let mut files = Vec::new();
        if path.exists() {
            for item in fs::read_dir(path)? {
                files.extend(Transaction::open(&item?.path())?.conflict_files());
            }
        }
        Ok(files)
    }

    pub fn resolve(&mut self, index: usize, keep_external: bool) -> Result<()> {
        if self.busy() {
            return Err(Error::new(ErrorCode::Busy, "task running"));
        }
        self.conflict_files(index)?;
        self.start(index, true, Some(keep_external))
    }
}

fn resolve(root: &Path, state: &Path, directory: &Path, keep: bool) -> Result<()> {
    let _locks = WriteLocks::acquire(state, &[root.to_path_buf()])?;
    let path = directory.join("transactions");
    if path.exists() {
        for item in fs::read_dir(path)? {
            let mut txn = Transaction::open(&item?.path())?;
            if txn.state() == State::Conflict {
                txn.resolve_all(keep)?;
            } else if txn.state() != State::Committed {
                txn.rollback()?;
            }
        }
    }
    Ok(())
}

pub fn recover(root: &Path, state: &Path, directory: &Path) -> Result<()> {
    let _locks = WriteLocks::acquire(state, &[root.to_path_buf()])?;
    let transactions = directory.join("transactions");
    if transactions.exists() {
        for item in fs::read_dir(transactions)? {
            let mut txn = Transaction::open(&item?.path())?;
            if txn.state() != State::Committed {
                txn.rollback()?;
            }
        }
    }
    Ok(())
}

pub fn root_key(root: &Path) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(root.to_string_lossy().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopened_queue_waits_for_confirmation_and_edits_transactionally() {
        let base = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(base.join("pack/mods")).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let root = base.join("pack");
        let state = base.join("state");
        fs::write(root.join("pack.toml"), "name = \"Test\"\n").unwrap();
        fs::write(
            root.join("mods/a.pw.toml"),
            "name = \"A\"\nfilename = \"a.jar\"\n",
        )
        .unwrap();
        let (mut doc, expected) = edit::document(&root, "mods/a.pw.toml").unwrap();
        edit::set(&mut doc, &["pin"], toml_edit::Value::from(true)).unwrap();
        let draft = edit::draft(&state, "mods/a.pw.toml", expected, &doc).unwrap();
        let mut queue = Queue::open(&root, &state).unwrap();
        queue.enqueue("pin", Request::Edit(vec![draft])).unwrap();
        drop(queue);
        let mut queue = Queue::open(&root, &state).unwrap();
        queue.poll().unwrap();
        assert!(queue.paused && !queue.busy());
        assert!(
            !fs::read_to_string(root.join("mods/a.pw.toml"))
                .unwrap()
                .contains("pin")
        );
        queue.resume().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while queue.pending() {
            queue.poll().unwrap();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(queue.tasks[0].status, Status::Completed);
        assert!(
            fs::read_to_string(root.join("mods/a.pw.toml"))
                .unwrap()
                .contains("pin = true")
        );
        assert!(root.join("index.toml").is_file());
        drop(queue);
        fs::remove_dir_all(base).unwrap();
    }
}
