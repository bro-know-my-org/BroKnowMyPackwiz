//! Merge only files actually installed by this task; leave unrelated jars unowned.
use super::{Control, Error, ErrorCode, Result, durable, edit::Attachment};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};

pub fn record(root: &Path, files: &[Attachment], control: &Control) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let path = root.join("packwiz.json");
    let mut manifest = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes).map_err(|e| invalid(e.to_string()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            json!({"format":"bkmpw:1","side":"both","files":[]})
        }
        Err(error) => return Err(error.into()),
    };
    if manifest.get("format").and_then(Value::as_str) != Some("bkmpw:1") {
        return Err(invalid("unsupported_manifest_format"));
    }
    let items = manifest
        .get_mut("files")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid("invalid_manifest_files"))?;
    let mut paths = BTreeSet::new();
    for item in items.iter_mut() {
        let path = item
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("invalid_manifest_path"))?;
        let path = crate::pathutil::safe_slash_path(path).map_err(Error::from)?;
        if !paths.insert(path.to_lowercase()) {
            return Err(invalid("duplicate_manifest_path"));
        }
        item["path"] = path.into();
    }
    for file in files {
        control.check()?;
        let existing = items.iter().position(|item| {
            item.get("path")
                .and_then(Value::as_str)
                .is_some_and(|p| p.eq_ignore_ascii_case(&file.relative))
        });
        if let Some(index) = existing {
            items[index]["path"] = file.relative.clone().into();
            items[index]["sha256"] = file.sha256.clone().into();
        } else {
            items.push(json!({"name":file.relative,"path":file.relative,"sha256":file.sha256}));
        }
    }
    let permissions = fs::metadata(&path).ok().map(|m| m.permissions());
    durable::write(
        &path,
        &serde_json::to_vec_pretty(&manifest).map_err(|e| invalid(e.to_string()))?,
    )?;
    if let Some(permissions) = permissions {
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}
fn invalid(detail: impl Into<String>) -> Error {
    Error::new(ErrorCode::Invalid, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_installs_retain_old_records_and_do_not_adopt_manual_jars() {
        let root = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods/manual.jar"), b"manual").unwrap();
        fs::write(root.join("packwiz.json"),serde_json::to_vec(&json!({"format":"bkmpw:1","side":"client","extra":42,"files":[{"path":"mods/old.jar","name":"Old","sha256":"old"}]})).unwrap()).unwrap();
        record(
            &root,
            &[Attachment {
                relative: "mods/new.jar".into(),
                source: root.join("unused"),
                expected: None,
                prepared: String::new(),
                sha256: "new".into(),
            }],
            &Control::default(),
        )
        .unwrap();
        let manifest: Value =
            serde_json::from_slice(&fs::read(root.join("packwiz.json")).unwrap()).unwrap();
        assert_eq!(manifest["files"].as_array().unwrap().len(), 2);
        assert_eq!(manifest["extra"], 42);
        assert_eq!(manifest["side"], "client");
        let paths = crate::install::read_manifest_paths(&root).unwrap();
        assert!(paths.contains("mods/old.jar") && paths.contains("mods/new.jar"));
        assert!(!paths.contains("mods/manual.jar"));
        fs::remove_dir_all(root).unwrap();
    }
}
