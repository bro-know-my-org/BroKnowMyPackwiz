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

pub fn document(root: &Path, relative: &str) -> Result<(DocumentMut, Option<String>)> {
    crate::pathutil::safe_slash_path(relative).map_err(Error::from)?;
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
        .ok_or_else(|| Error::new(ErrorCode::Invalid, "empty field"))?;
    let mut table = document.as_table_mut();
    for parent in parents {
        if !table.contains_key(parent) {
            table.insert(parent, Item::Table(toml_edit::Table::new()));
        }
        table = table
            .get_mut(parent)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| Error::new(ErrorCode::Invalid, "field parent is not a table"))?;
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
    crate::pathutil::safe_slash_path(relative).map_err(Error::from)?;
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
    let _lock = WriteLocks::acquire(state, &[root.to_path_buf()])?;
    if let Some(guard) = guard {
        guard.validate(root, control)?;
    }
    let workspace = Workspace::create(root, &task.join("workspace"), control)?;
    if let Some(guard) = guard {
        guard.validate(&workspace.staged, control)?;
    }
    for draft in drafts {
        control.check()?;
        crate::pathutil::safe_slash_path(&draft.relative).map_err(Error::from)?;
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
    let config = crate::config::ProjectConfig::load(&workspace.staged).map_err(Error::from)?;
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
