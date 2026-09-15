use super::*;

#[test]
fn empty_directories_commit_and_failure_rolls_them_back() {
    let f = Fixture::new();
    let mut tx = f.prepare(Vec::new());
    tx.create_directories(vec![
        f.root.join("pack/mods/common"),
        f.root.join("pack/shaderpacks"),
    ])
    .unwrap();
    tx.commit(&Control::default()).unwrap();
    assert!(f.root.join("pack/mods/common").is_dir());
    assert!(f.root.join("pack/shaderpacks").is_dir());

    f.file("source", b"new");
    let mut tx = f.prepare(vec![f.change("new-file", Some("source"))]);
    tx.create_directories(vec![f.root.join("new-tree/empty")])
        .unwrap();
    fs::remove_file(tx.staged(0)).unwrap();
    assert!(tx.commit(&Control::default()).is_err());
    assert!(!f.root.join("new-tree").exists());
    assert!(f.root.join("pack/mods/common").exists());
}

#[test]
fn recovery_retains_external_content_in_created_directory_until_resolved() {
    let f = Fixture::new();
    let mut tx = f.prepare(Vec::new());
    tx.journal.state = State::Committing;
    tx.ensure_parents(&f.root.join("new-tree/empty/unused"))
        .unwrap();
    f.file("new-tree/empty/external", b"external");
    let mut reopened = Transaction::open(&tx.directory).unwrap();
    assert_eq!(reopened.rollback().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(reopened.state(), State::Conflict);
    assert_eq!(
        fs::read(f.root.join("new-tree/empty/external")).unwrap(),
        b"external"
    );
    reopened.resolve_all(true).unwrap();
    assert_eq!(reopened.state(), State::RolledBack);
    assert!(f.root.join("new-tree/empty/external").exists());
}

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("bkmpw-txn-{}", durable::unique_id()));
        fs::create_dir_all(&root).unwrap();
        Self {
            root: fs::canonicalize(root).unwrap(),
        }
    }
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
    fn change(&self, target: &str, source: Option<&str>) -> Change {
        let target = self.root.join(target);
        Change {
            expected: durable::fingerprint(&target).unwrap(),
            target,
            source: source.map(|s| self.root.join(s)),
        }
    }
    fn prepare(&self, changes: Vec<Change>) -> Transaction {
        Transaction::prepare(&self.root.join("state"), changes, &Control::default()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn commit_creates_replaces_and_deletes_across_directories() {
    let f = Fixture::new();
    f.file("old", b"old");
    f.file("delete", b"delete");
    f.file("source", b"new");
    let mut tx = f.prepare(vec![
        f.change("old", Some("source")),
        f.change("new/nested/file", Some("source")),
        f.change("delete", None),
    ]);
    tx.commit(&Control::default()).unwrap();
    assert_eq!(fs::read(f.root.join("old")).unwrap(), b"new");
    assert_eq!(fs::read(f.root.join("new/nested/file")).unwrap(), b"new");
    assert!(!f.root.join("delete").exists());
    assert_eq!(
        Transaction::open(&tx.directory).unwrap().state(),
        State::Committed
    );
}

#[test]
fn later_failure_restores_already_written_files() {
    let f = Fixture::new();
    f.file("first", b"original");
    f.file("second", b"original");
    f.file("source", b"new");
    let mut tx = f.prepare(vec![
        f.change("first", Some("source")),
        f.change("second", Some("source")),
    ]);
    fs::remove_file(tx.staged(1)).unwrap();
    assert!(tx.commit(&Control::default()).is_err());
    assert_eq!(fs::read(f.root.join("first")).unwrap(), b"original");
    assert_eq!(fs::read(f.root.join("second")).unwrap(), b"original");
    assert_eq!(tx.state(), State::RolledBack);
}

#[test]
fn recovery_replays_interrupted_write_and_rollback_idempotently() {
    let f = Fixture::new();
    let target = f.file("target", b"original");
    f.file("source", b"new");
    let mut tx = f.prepare(vec![f.change("target", Some("source"))]);
    // Crash after the file replacement, before recording Applied.
    tx.journal.state = State::Committing;
    tx.journal.entries[0].step = Step::Applying;
    tx.save().unwrap();
    durable::replace(&tx.staged(0), &target).unwrap();
    let mut reopened = Transaction::open(&tx.directory).unwrap();
    reopened.rollback().unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"original");
    Transaction::open(&tx.directory)
        .unwrap()
        .rollback()
        .unwrap();
    assert_eq!(fs::read(target).unwrap(), b"original");
}

#[test]
fn external_edit_is_preserved_with_backup_and_staged_content() {
    let f = Fixture::new();
    let target = f.file("target", b"original");
    f.file("source", b"new");
    let mut tx = f.prepare(vec![f.change("target", Some("source"))]);
    tx.journal.state = State::Committing;
    tx.journal.entries[0].step = Step::Applied;
    tx.save().unwrap();
    fs::write(&target, b"external edit").unwrap();
    let error = tx.rollback().unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(fs::read(&target).unwrap(), b"external edit");
    assert_eq!(fs::read(tx.backup(0)).unwrap(), b"original");
    assert_eq!(fs::read(tx.staged(0)).unwrap(), b"new");
    assert_eq!(tx.conflicts(), &[target]);
}

#[test]
fn cancellation_never_publishes_prepared_content() {
    let f = Fixture::new();
    let target = f.file("target", b"original");
    f.file("source", b"new");
    let mut tx = f.prepare(vec![f.change("target", Some("source"))]);
    let control = Control::default();
    control.cancel();
    assert_eq!(tx.commit(&control).unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(fs::read(target).unwrap(), b"original");
    assert_eq!(tx.state(), State::RolledBack);
}

#[test]
fn lock_excludes_another_writer_and_releases_on_drop() {
    let f = Fixture::new();
    let paths = vec![f.root.clone()];
    let first = super::super::lock::WriteLocks::acquire(&f.root, &paths).unwrap();
    assert!(super::super::lock::WriteLocks::acquire(&f.root, &paths).is_err());
    drop(first);
    assert!(super::super::lock::WriteLocks::acquire(&f.root, &paths).is_ok());
}

#[test]
fn explicit_conflict_resolution_preserves_external_or_restores_with_archive() {
    for keep in [true, false] {
        let f = Fixture::new();
        let target = f.file("target", b"original");
        f.file("source", b"new");
        let mut tx = f.prepare(vec![f.change("target", Some("source"))]);
        tx.journal.state = State::Committing;
        tx.journal.entries[0].step = Step::Applied;
        tx.save().unwrap();
        fs::write(&target, b"external").unwrap();
        assert!(tx.rollback().is_err());
        tx.resolve_all(keep).unwrap();
        assert_eq!(
            fs::read(&target).unwrap(),
            if keep {
                b"external".as_slice()
            } else {
                b"original".as_slice()
            }
        );
        assert_eq!(fs::read(tx.backup(0)).unwrap(), b"original");
        if !keep {
            let archive = fs::read_dir(&tx.directory)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("external-")
                })
                .unwrap();
            assert_eq!(fs::read(archive).unwrap(), b"external");
        }
        assert_eq!(
            Transaction::open(&tx.directory).unwrap().state(),
            State::RolledBack
        );
    }
}
