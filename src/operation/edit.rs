use super::{
    Control, Error, ErrorCode, Result, durable, lock::WriteLocks, transaction::Transaction,
    workspace::Workspace,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Draft {
    pub relative: String,
    pub source: PathBuf,
    pub expected: Option<String>,
    pub prepared: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attachment {
    pub relative: String,
    pub source: PathBuf,
    pub expected: Option<String>,
    pub prepared: String,
    pub sha256: String,
}

pub fn document(root: &Path, relative: &str) -> Result<(DocumentMut, Option<String>)> {
    crate::operation::paths::relative(relative)?;
    let path = durable::absolute(&durable::canonical(root)?.join(relative))?;
    let expected = durable::fingerprint(&path)?;
    let text = if expected.is_some() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let parsed = text
        .parse::<DocumentMut>()
        .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?;
    if durable::fingerprint(&path)? != expected {
        return Err(Error::new(ErrorCode::Conflict, relative));
    }
    Ok((parsed, expected))
}

pub fn set(document: &mut DocumentMut, keys: &[&str], value: Value) -> Result<()> {
    let (last, parents) = keys
        .split_last()
        .ok_or_else(|| Error::named(ErrorCode::Invalid, "field_required", "empty field"))?;
    let mut table = document.as_table_mut();
    for parent in parents {
        if !table.contains_key(parent) {
            table.insert(parent, Item::Table(toml_edit::Table::new()));
        }
        table = table
            .get_mut(parent)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| {
                Error::named(
                    ErrorCode::Invalid,
                    "field_parent_not_table",
                    "field parent is not a table",
                )
            })?;
    }
    let mut value = value;
    if let Some(previous) = table.get(last).and_then(Item::as_value) {
        *value.decor_mut() = previous.decor().clone();
    }
    if let Some(item) = table.get_mut(last) {
        *item = Item::Value(value);
    } else {
        table.insert(last, Item::Value(value));
    }
    Ok(())
}

pub fn draft(
    state: &Path,
    relative: &str,
    expected: Option<String>,
    document: &DocumentMut,
) -> Result<Draft> {
    crate::operation::paths::relative(relative)?;
    let dir = state.join("drafts");
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    let source = dir.join(format!("{}.toml", durable::unique_id()));
    durable::write(&source, document.to_string().as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600))?;
    }
    let prepared = durable::fingerprint(&source)?.unwrap();
    super::artifacts::record(&source, &prepared);
    Ok(Draft {
        relative: relative.into(),
        source,
        expected,
        prepared,
    })
}

pub fn execute(
    root: &Path,
    state: &Path,
    task: &Path,
    drafts: &[Draft],
    guard: Option<&super::preview::Guard>,
    control: &Control,
) -> Result<()> {
    execute_files(root, state, task, drafts, &[], guard, control)
}

pub fn execute_files(
    root: &Path,
    state: &Path,
    task: &Path,
    drafts: &[Draft],
    files: &[Attachment],
    guard: Option<&super::preview::Guard>,
    control: &Control,
) -> Result<()> {
    let _lock = WriteLocks::acquire(state, &[root.to_path_buf()])?;
    if let Some(guard) = guard {
        guard.validate(root, control)?;
    }
    if drafts.is_empty() && files.is_empty() {
        return control.check();
    }
    let mut targets = std::collections::BTreeSet::new();
    for path in drafts
        .iter()
        .map(|d| &d.relative)
        .chain(files.iter().map(|f| &f.relative))
    {
        crate::operation::paths::relative(path)?;
        if !targets.insert(path.to_lowercase()) {
            return Err(Error::key(ErrorCode::Conflict, "duplicate_batch_target"));
        }
    }
    let workspace = Workspace::create(root, &task.join("workspace"), control)?;
    if let Some(guard) = guard {
        guard.validate(&workspace.staged, control)?;
    }
    for draft in drafts {
        control.check()?;
        crate::operation::paths::relative(&draft.relative)?;
        if durable::fingerprint(&root.join(&draft.relative))? != draft.expected
            || durable::fingerprint(&draft.source)?.as_ref() != Some(&draft.prepared)
        {
            return Err(Error::new(ErrorCode::Conflict, &draft.relative));
        }
        let text = fs::read_to_string(&draft.source)?;
        text.parse::<DocumentMut>()
            .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?;
        let target = workspace.staged.join(&draft.relative);
        // Preserve target permissions; the private draft's 0600 mode is not a product setting.
        let permissions = fs::metadata(&target).ok().map(|m| m.permissions());
        durable::write(&target, text.as_bytes())?;
        if let Some(permissions) = permissions {
            fs::set_permissions(&target, permissions)?;
        }
    }
    for file in files {
        control.check()?;
        if durable::fingerprint(&root.join(&file.relative))? != file.expected
            || durable::fingerprint(&file.source)?.as_ref() != Some(&file.prepared)
        {
            return Err(Error::new(ErrorCode::Conflict, &file.relative));
        }
        let target = workspace.staged.join(&file.relative);
        let permissions = fs::metadata(&target).ok().map(|m| m.permissions());
        if target.exists() {
            fs::remove_file(&target)?;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let hashes = super::transfer::copy(&file.source, &target, control)?;
        hashes.verify("sha256", &file.sha256)?;
        if durable::fingerprint(&file.source)?.as_ref() != Some(&file.prepared) {
            return Err(Error::new(ErrorCode::Conflict, &file.relative));
        }
        if let Some(permissions) = permissions {
            fs::set_permissions(&target, permissions)?;
        }
    }
    super::manifest::record(&workspace.staged, files, control)?;
    let config = crate::config::ProjectConfig::load_operation(&workspace.staged)?;
    let layout = crate::layout::PackLayout::from_config(&config);
    crate::refresh::refresh(&workspace.staged, &config, &layout).map_err(Error::from)?;
    let changes = workspace.changes(control)?;
    let mut txn = Transaction::prepare(task, changes, control)?;
    txn.commit(control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_and_payload_commit_together_and_failed_verification_publishes_neither() {
        let base = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(base.join("pack/mods")).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let root = base.join("pack");
        let state = base.join("state");
        fs::write(root.join("pack.toml"), "name = \"Pack\"\n").unwrap();
        fs::write(
            root.join("mods/a.pw.toml"),
            "name = \"Old\"\nfilename = \"a.jar\"\n",
        )
        .unwrap();
        fs::write(root.join("mods/a.jar"), b"old payload").unwrap();
        let source = base.join("new.jar");
        fs::write(&source, b"new payload").unwrap();
        let control = Control::default();
        let guard = super::super::preview::Guard::capture(&root, &control).unwrap();
        let (mut doc, expected) = document(&root, "mods/a.pw.toml").unwrap();
        set(&mut doc, &["name"], Value::from("New")).unwrap();
        let drafts = [draft(&state, "mods/a.pw.toml", expected, &doc).unwrap()];
        let mut file = Attachment {
            relative: "mods/a.jar".into(),
            source: source.clone(),
            expected: durable::fingerprint(&root.join("mods/a.jar")).unwrap(),
            prepared: durable::fingerprint(&source).unwrap().unwrap(),
            sha256: "wrong".into(),
        };
        assert!(
            execute_files(
                &root,
                &state,
                &base.join("failed"),
                &drafts,
                &[file.clone()],
                Some(&guard),
                &control
            )
            .is_err()
        );
        assert!(
            fs::read_to_string(root.join("mods/a.pw.toml"))
                .unwrap()
                .contains("Old")
        );
        assert_eq!(fs::read(root.join("mods/a.jar")).unwrap(), b"old payload");
        assert!(!root.join("index.toml").exists());
        file.sha256 = super::super::transfer::hash(&source, &control)
            .unwrap()
            .sha256;
        execute_files(
            &root,
            &state,
            &base.join("success"),
            &drafts,
            &[file],
            Some(&guard),
            &control,
        )
        .unwrap();
        assert!(
            fs::read_to_string(root.join("mods/a.pw.toml"))
                .unwrap()
                .contains("New")
        );
        assert_eq!(fs::read(root.join("mods/a.jar")).unwrap(), b"new payload");
        assert!(root.join("index.toml").exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn editing_preserves_unrelated_fields_and_comments() {
        let mut doc: DocumentMut = "# owner note\nname = 'Hello' # name note\nunknown = 42\n[download]\nurl = 'https://example.invalid/a'\n".parse().unwrap();
        set(&mut doc, &["name"], Value::from("世界")).unwrap();
        set(&mut doc, &["pin"], Value::from(true)).unwrap();
        let text = doc.to_string();
        assert!(text.contains("# owner note"));
        assert!(text.contains("# name note"));
        assert!(text.contains("unknown = 42"));
        assert!(text.contains("url = 'https://example.invalid/a'"));
        assert_eq!(doc["pin"].as_bool(), Some(true));
    }
}
