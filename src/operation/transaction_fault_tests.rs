use super::*;
use crate::operation::faults::{Point, Scope};
use std::{cell::Cell, io, rc::Rc};

fn batch(f: &Fixture) -> Transaction {
    for name in ["a", "b", "c"] {
        f.file(name, name.as_bytes());
    }
    f.file("source", b"new");
    f.prepare(
        ["a", "b", "c", "new"]
            .iter()
            .map(|name| f.change(name, Some("source")))
            .collect(),
    )
}

fn assert_original(f: &Fixture) {
    for name in ["a", "b", "c"] {
        assert_eq!(fs::read(f.root.join(name)).unwrap(), name.as_bytes());
    }
    assert!(!f.root.join("new").exists());
}

#[test]
fn each_commit_write_boundary_recovers_from_a_single_io_failure() {
    let f = Fixture::new();
    let mut tx = batch(&f);
    let count = Rc::new(Cell::new(0));
    let seen = count.clone();
    let scope = Scope::new(move |_, _| {
        seen.set(seen.get() + 1);
        Ok(())
    });
    tx.commit(&Control::default()).unwrap();
    drop(scope);
    assert!(count.get() > 10);

    // Includes failure after the final journal rename, before directory fsync.
    for fail_at in 0..count.get() {
        for kind in [io::ErrorKind::StorageFull, io::ErrorKind::PermissionDenied] {
            let f = Fixture::new();
            let mut tx = batch(&f);
            let at = Cell::new(0);
            let scope = Scope::new(move |_, _| {
                let current = at.get();
                at.set(current + 1);
                if current == fail_at {
                    Err(kind.into())
                } else {
                    Ok(())
                }
            });
            assert_eq!(
                tx.commit(&Control::default()).unwrap_err().code,
                ErrorCode::Io
            );
            drop(scope);
            assert_original(&f);
            let mut reopened = Transaction::open(&tx.directory).unwrap();
            assert_eq!(reopened.state(), State::RolledBack);
            reopened.rollback().unwrap();
            assert_original(&f);
        }
    }
}

#[test]
fn backup_io_failure_never_changes_original_files() {
    for point in [Point::BackupBefore, Point::BackupCopied] {
        let f = Fixture::new();
        f.file("target", b"original");
        f.file("source", b"new");
        let scope = Scope::new(move |current, _| {
            if current == point {
                Err(io::ErrorKind::StorageFull.into())
            } else {
                Ok(())
            }
        });
        let result = Transaction::prepare(
            &f.root.join("state"),
            vec![f.change("target", Some("source"))],
            &Control::default(),
        );
        assert_eq!(result.err().unwrap().code, ErrorCode::Io);
        drop(scope);
        assert_eq!(fs::read(f.root.join("target")).unwrap(), b"original");
    }
}

#[test]
fn persistent_io_failure_keeps_backups_for_later_recovery() {
    let f = Fixture::new();
    let mut tx = batch(&f);
    let target = f.root.join("a");
    let failed = Cell::new(false);
    let scope = Scope::new(move |point, path| {
        if point == Point::ReplacePublished && path == target {
            failed.set(true);
        }
        if failed.get() {
            Err(io::ErrorKind::StorageFull.into())
        } else {
            Ok(())
        }
    });
    assert_eq!(
        tx.commit(&Control::default()).unwrap_err().code,
        ErrorCode::Io
    );
    assert_eq!(fs::read(f.root.join("a")).unwrap(), b"new");
    for (i, name) in ["a", "b", "c"].iter().enumerate() {
        assert_eq!(fs::read(tx.backup(i)).unwrap(), name.as_bytes());
    }
    drop(scope);
    let mut reopened = Transaction::open(&tx.directory).unwrap();
    reopened.rollback().unwrap();
    assert_original(&f);
}

#[test]
fn cancellation_before_final_check_rolls_back_but_after_commit_marker_keeps_success() {
    let f = Fixture::new();
    let mut tx = batch(&f);
    let mut control = Control::default();
    let cancel = control.clone();
    let target = f.root.join("new").display().to_string();
    control.checkpoint = Some(std::sync::Arc::new(move |event| {
        if matches!(event, super::super::super::Event::Progress { label, .. } if label == &target) {
            cancel.cancel();
        }
    }));
    assert_eq!(tx.commit(&control).unwrap_err().code, ErrorCode::Cancelled);
    assert_original(&f);

    let f = Fixture::new();
    let mut tx = batch(&f);
    let control = Control::default();
    let cancel = control.clone();
    let journal = tx.directory.join("journal.json");
    let scope = Scope::new(move |point, path| {
        if point == Point::WritePublished && path == journal {
            let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
            if value["state"] == "Committed" {
                cancel.cancel();
            }
        }
        Ok(())
    });
    tx.commit(&control).unwrap();
    drop(scope);
    assert_eq!(control.check().unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(
        Transaction::open(&tx.directory).unwrap().state(),
        State::Committed
    );
    for name in ["a", "b", "c", "new"] {
        assert_eq!(fs::read(f.root.join(name)).unwrap(), b"new");
    }
}

#[test]
fn crash_after_restoring_a_file_can_resume_recovery_again() {
    let f = Fixture::new();
    let mut tx = batch(&f);
    // Apply all file changes without publishing the transaction commit marker.
    tx.journal.state = State::Committing;
    tx.save().unwrap();
    for i in 0..tx.journal.entries.len() {
        tx.apply_file(i, &Control::default()).unwrap();
    }
    let target = f.root.join("b");
    let scope = Scope::new(move |point, path| {
        assert!(
            !(point == Point::ReplacePublished && path == target),
            "simulated process crash"
        );
        Ok(())
    });
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tx.rollback())).is_err());
    drop(scope);
    assert_eq!(fs::read(f.root.join("a")).unwrap(), b"new");
    assert_eq!(fs::read(f.root.join("b")).unwrap(), b"b");
    assert_eq!(fs::read(f.root.join("c")).unwrap(), b"c");
    assert!(!f.root.join("new").exists());
    let mut reopened = Transaction::open(&tx.directory).unwrap();
    assert_eq!(reopened.state(), State::RollingBack);
    reopened.rollback().unwrap();
    assert_original(&f);
    Transaction::open(&tx.directory)
        .unwrap()
        .rollback()
        .unwrap();
    assert_original(&f);
}
