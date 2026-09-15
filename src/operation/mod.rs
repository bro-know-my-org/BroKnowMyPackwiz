pub mod download;
pub mod durable;
pub mod edit;
pub mod lock;
pub mod manifest;
pub mod preview;
pub mod queue;
pub mod transaction;
pub mod transfer;
pub mod workspace;

use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    Io,
    Invalid,
    Conflict,
    Cancelled,
    Busy,
    Interrupted,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Error {
    pub code: ErrorCode,
    pub detail: String,
}

impl Error {
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::new(ErrorCode::Io, error.to_string())
    }
}
impl From<String> for Error {
    fn from(error: String) -> Self {
        Self::new(ErrorCode::Failed, error)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.detail)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Event {
    Phase(String),
    Progress {
        label: String,
        current: u64,
        total: Option<u64>,
    },
    Log(String),
}

#[derive(Clone, Default)]
pub struct Control {
    cancelled: Arc<AtomicBool>,
    pub events: Option<Sender<Event>>,
}

impl Control {
    pub fn with_events(events: Sender<Event>) -> Self {
        Self {
            events: Some(events),
            ..Self::default()
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(Error::new(ErrorCode::Cancelled, "cancelled"))
        } else {
            Ok(())
        }
    }
    pub fn emit(&self, event: Event) {
        if let Some(tx) = &self.events {
            let _ = tx.send(event);
        }
    }
}
