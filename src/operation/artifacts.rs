//! Ownership of files produced by a preview before a durable queue takes over.
use super::durable;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type Files = Arc<Mutex<BTreeMap<PathBuf, String>>>;
thread_local! { static ACTIVE: RefCell<Vec<Files>> = const { RefCell::new(Vec::new()) }; }

#[derive(Default)]
pub struct Lease {
    files: Files,
}
impl Lease {
    /// Called only after the queue has durably recorded ownership.
    pub fn release(&mut self) {
        self.files.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        for (path, expected) in self.files.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            if durable::absolute(path).is_ok()
                && durable::fingerprint(path).ok().flatten().as_ref() == Some(expected)
            {
                let _ = fs::remove_file(path);
            }
        }
    }
}

pub struct Scope {
    lease: Option<Lease>,
}
impl Scope {
    pub fn new() -> Self {
        let lease = Lease::default();
        ACTIVE.with(|active| active.borrow_mut().push(lease.files.clone()));
        Self { lease: Some(lease) }
    }
    pub fn finish(mut self) -> Lease {
        self.lease.take().unwrap()
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            active.borrow_mut().pop();
        });
    }
}

pub fn record(path: &Path, expected: &str) {
    ACTIVE.with(|active| {
        if let Some(files) = active.borrow().last() {
            if let Ok(path) = durable::absolute(path) {
                files
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(path)
                    .or_insert_with(|| expected.into());
            }
        }
    });
}

pub fn record_payload(path: &Path, sha256: &str, permissions: &fs::Permissions) {
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        permissions.mode()
    };
    #[cfg(not(unix))]
    let mode = u32::from(permissions.readonly());
    record(path, &format!("{sha256}:{mode}"));
}

pub fn remove_temporary(path: &Path) {
    let allowed = ACTIVE.with(|active| {
        let active = active.borrow();
        let Some(files) = active.last() else {
            return true;
        };
        let Ok(path) = durable::absolute(path) else {
            return false;
        };
        let files = files.lock().unwrap_or_else(|e| e.into_inner());
        files.get(&path).is_some_and(|expected| {
            durable::fingerprint(&path).ok().flatten().as_ref() == Some(expected)
        })
    });
    if allowed {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::{Control, edit, transfer};

    #[test]
    fn abandoned_preview_cleans_drafts_and_payloads_but_preserves_external_changes() {
        let base = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(&base).unwrap();
        let scope = Scope::new();
        let doc = "name = 'Test'".parse().unwrap();
        let draft = edit::draft(&base, "mods/a.pw.toml", None, &doc).unwrap();
        let changed = edit::draft(&base, "mods/b.pw.toml", None, &doc).unwrap();
        let source = base.join("source");
        let payload = base.join("payload");
        fs::write(&source, "payload").unwrap();
        transfer::copy(&source, &payload, &Control::default()).unwrap();
        fs::write(&changed.source, "external change").unwrap();
        let lease = scope.finish();
        assert!(draft.source.exists() && payload.exists());
        drop(lease);
        assert!(!draft.source.exists() && !payload.exists());
        assert!(changed.source.exists() && source.exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn releasing_ownership_keeps_files_for_the_queue() {
        let base = std::env::temp_dir().join(durable::unique_id());
        let scope = Scope::new();
        let draft = edit::draft(
            &base,
            "mods/a.pw.toml",
            None,
            &"name = 'Test'".parse().unwrap(),
        )
        .unwrap();
        let mut lease = scope.finish();
        lease.release();
        drop(lease);
        assert!(draft.source.exists());
        fs::remove_dir_all(base).unwrap();
    }
}
