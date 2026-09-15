use super::{
    Control, Error, ErrorCode, Result,
    command::{Output, Request},
    durable,
    lock::WriteLocks,
    process,
    transaction::{Change, Transaction},
    workspace::Workspace,
};
use std::{fs, path::Path, process::Command};

/// Execute the existing CLI only against private paths, then publish one batch.
pub fn execute(
    root: &Path,
    state: &Path,
    task: &Path,
    request: &Request,
    control: &Control,
) -> Result<()> {
    let executable = std::env::current_exe()?;
    execute_with(root, state, task, request, control, |root, target| {
        for (index, arguments) in request.arguments(root, target)?.into_iter().enumerate() {
            let mut command = Command::new(&executable);
            command.arg(request.kind.command()).args(arguments);
            process::run(
                command,
                root,
                &task.join("logs").join(index.to_string()),
                control,
            )?;
        }
        Ok(())
    })
}

fn execute_with(
    root: &Path,
    state: &Path,
    task: &Path,
    request: &Request,
    control: &Control,
    run: impl FnOnce(&Path, Option<&Path>) -> Result<()>,
) -> Result<()> {
    request.validate()?;
    control.check()?;
    let root = durable::canonical(root)?;
    let target = request.target(&root)?;
    let mut paths = vec![root.clone()];
    paths.extend(target.iter().cloned());
    let _locks = WriteLocks::acquire(state, &paths)?;
    if let Some(guard) = &request.guard {
        guard.validate(&root, control)?;
    }
    if request.kind.readonly() {
        return run(&root, target.as_deref());
    }
    let stage = task.join("workspace");
    // An ancestor output must share its snapshot with the contained pack.
    let ancestor = target
        .as_ref()
        .filter(|path| request.kind.output() == Output::Directory && root.starts_with(path));
    let original = ancestor.unwrap_or(&root);
    let primary = Workspace::create(original, &stage.join("root"), control)?;
    let mapped_root = primary.staged.join(root.strip_prefix(original).unwrap());
    let mut external = None;
    let mut file_before = None;
    let mut file_permissions = None;
    let mapped_target = match target.as_ref() {
        None => None,
        Some(path) if path.starts_with(original) => {
            Some(primary.staged.join(path.strip_prefix(original).unwrap()))
        }
        Some(path) if request.kind.output() == Output::Directory => {
            let workspace = Workspace::create(path, &stage.join("external"), control)?;
            let mapped = workspace.staged.clone();
            external = Some(workspace);
            Some(mapped)
        }
        Some(path) => {
            durable::absolute(path)?;
            file_before = durable::fingerprint(path)?;
            if file_before.is_some() {
                file_permissions = Some(fs::metadata(path)?.permissions());
            }
            let directory = stage.join("output");
            fs::create_dir_all(&directory)?;
            Some(
                directory.join(
                    path.file_name()
                        .ok_or_else(|| Error::key(ErrorCode::Invalid, "output_required"))?,
                ),
            )
        }
    };
    run(&mapped_root, mapped_target.as_deref())?;
    control.check()?;
    let mut changes = primary.changes(control)?;
    let mut directories = primary.new_directories(control)?;
    let mut directory_changes = primary.directory_changes(control)?;
    if let Some(workspace) = external {
        changes.extend(workspace.changes(control)?);
        directories.extend(workspace.new_directories(control)?);
        directory_changes.extend(workspace.directory_changes(control)?);
    }
    if let (Some(target), Some(mapped)) = (&target, &mapped_target) {
        if request.kind.output() == Output::File && !target.starts_with(original) {
            if durable::fingerprint(mapped)?.is_none() {
                return Err(Error::key(ErrorCode::Failed, "output_missing"));
            }
            if let Some(permissions) = file_permissions {
                fs::set_permissions(mapped, permissions)?;
            }
            changes.push(Change {
                target: target.clone(),
                expected: file_before,
                source: Some(mapped.clone()),
            });
        }
    }
    let mut transaction = Transaction::prepare(task, changes, control)?;
    transaction.create_directories(directories)?;
    transaction.change_directories(directory_changes)?;
    transaction.commit(control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::command::Kind;
    use std::path::PathBuf;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let base = std::env::temp_dir().join(durable::unique_id());
            fs::create_dir_all(base.join("pack")).unwrap();
            fs::write(base.join("pack/metadata"), "old").unwrap();
            Self(fs::canonicalize(base).unwrap())
        }
        fn run(
            &self,
            request: &Request,
            run: impl FnOnce(&Path, Option<&Path>) -> Result<()>,
        ) -> Result<()> {
            execute_with(
                &self.0.join("pack"),
                &self.0.join("state"),
                &self.0.join("task"),
                request,
                &Control::default(),
                run,
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn initialization_can_publish_into_a_previously_missing_root() {
        let fixture = Fixture::new();
        fs::remove_dir_all(fixture.0.join("pack")).unwrap();
        fixture
            .run(&Request::new(Kind::Init), |root, _| {
                fs::create_dir_all(root.join("mods/common"))?;
                fs::write(root.join("pack.toml"), "name = 'New'")?;
                Ok(())
            })
            .unwrap();
        assert!(fixture.0.join("pack/mods/common").is_dir());
        assert!(fixture.0.join("pack/pack.toml").is_file());
    }

    #[test]
    fn replacing_external_output_removes_stale_empty_directories() {
        let fixture = Fixture::new();
        let output = fixture.0.join("output");
        fs::create_dir_all(output.join("stale/empty")).unwrap();
        fs::write(output.join("stale/old.txt"), "old").unwrap();
        let mut request = Request::new(Kind::PrepareServer);
        request.output = Some(output.clone());
        fixture
            .run(&request, |_, target| {
                let target = target.unwrap();
                fs::remove_dir_all(target)?;
                fs::create_dir_all(target)?;
                fs::write(target.join("new.txt"), "new")?;
                Ok(())
            })
            .unwrap();
        assert!(!output.join("stale").exists());
        assert_eq!(fs::read_to_string(output.join("new.txt")).unwrap(), "new");
    }

    #[test]
    fn removal_guard_blocks_execution_after_metadata_changes() {
        let fixture = Fixture::new();
        let root = fixture.0.join("pack");
        fs::create_dir(root.join("mods")).unwrap();
        fs::write(root.join("mods/a.pw.toml"), "name = 'A'").unwrap();
        let mut request = Request::new(Kind::Remove);
        request.names = vec!["mods/a.pw.toml".into()];
        request.guard =
            Some(crate::operation::preview::Guard::capture(&root, &Control::default()).unwrap());
        fs::write(root.join("mods/a.pw.toml"), "name = 'External'").unwrap();
        let result = fixture.run(&request, |_, _| panic!("stale deletion must not run"));
        assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
    }

    #[test]
    fn initializes_empty_directory_skeleton_without_marker_files() {
        let fixture = Fixture::new();
        fixture
            .run(&Request::new(Kind::Init), |root, _| {
                fs::create_dir_all(root.join("mods/common"))?;
                fs::create_dir_all(root.join("shaderpacks"))?;
                Ok(())
            })
            .unwrap();
        assert!(fixture.0.join("pack/mods/common").is_dir());
        assert_eq!(
            fs::read_dir(fixture.0.join("pack/mods/common"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn failure_publishes_neither_pack_nor_external_output() {
        let fixture = Fixture::new();
        let output = fixture.0.join("export.zip");
        fs::write(&output, "previous").unwrap();
        let mut request = Request::new(Kind::ExportClient);
        request.output = Some(output.clone());
        let result = fixture.run(&request, |root, target| {
            fs::write(root.join("metadata"), "new")?;
            fs::write(target.unwrap(), "partial")?;
            Err(Error::key(ErrorCode::Failed, "injected"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(output).unwrap(), "previous");
        assert_eq!(
            fs::read_to_string(fixture.0.join("pack/metadata")).unwrap(),
            "old"
        );
    }

    #[test]
    fn external_directory_cleanup_and_pack_change_publish_together() {
        let fixture = Fixture::new();
        let output = fixture.0.join("installed");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("managed.jar"), "old").unwrap();
        fs::write(output.join("manual.jar"), "manual").unwrap();
        let mut request = Request::new(Kind::Sync);
        request.output = Some(output.clone());
        fixture
            .run(&request, |root, target| {
                fs::write(root.join("metadata"), "new")?;
                let target = target.unwrap();
                fs::remove_file(target.join("managed.jar"))?;
                fs::write(target.join("new.jar"), "payload")?;
                Ok(())
            })
            .unwrap();
        assert!(!output.join("managed.jar").exists());
        assert_eq!(
            fs::read_to_string(output.join("manual.jar")).unwrap(),
            "manual"
        );
        assert_eq!(
            fs::read_to_string(output.join("new.jar")).unwrap(),
            "payload"
        );
        assert_eq!(
            fs::read_to_string(fixture.0.join("pack/metadata")).unwrap(),
            "new"
        );
    }

    #[test]
    fn external_output_conflict_prevents_all_publication() {
        let fixture = Fixture::new();
        let output = fixture.0.join("export.zip");
        let mut request = Request::new(Kind::ExportClient);
        request.output = Some(output.clone());
        let result = fixture.run(&request, |root, target| {
            fs::write(root.join("metadata"), "new")?;
            fs::write(target.unwrap(), "archive")?;
            fs::write(&output, "external")?;
            Ok(())
        });
        assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(fs::read_to_string(output).unwrap(), "external");
        assert_eq!(
            fs::read_to_string(fixture.0.join("pack/metadata")).unwrap(),
            "old"
        );
    }
}
