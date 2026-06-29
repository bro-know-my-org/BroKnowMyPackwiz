use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_CONFIG: &str = r#"[scan]
use-gitignore = true
packwizignore = ".packwizignore"

[layout]
metadata-root = "mods"
metadata-roots = "mods,resourcepacks,shaderpacks"
jar-root = "mods"
server-meta = "mods/server"
client-meta = "mods/client"
common-meta = "mods/common"
metadata-extension = "pw"

[install]
jobs = 8
retries = 1
retry-delay-seconds = 5
force = false

[curseforge]
api-key = ""
cdn-fallback = true
"#;

const DEFAULT_PACK: &str = r#"name = "bro-know-my-packwiz-pack"
pack-format = "packwiz:1.1.0"

[index]
file = "index.toml"
hash-format = "sha256"
hash = ""

[versions]
minecraft = ""
"#;

const DEFAULT_INDEX: &str = "hash-format = \"sha256\"\n";

#[derive(Debug, Clone)]
pub struct InitResult {
    pub created: Vec<PathBuf>,
    pub kept: Vec<PathBuf>,
}

pub fn init_pack(root: &Path) -> Result<InitResult, String> {
    let mut result = InitResult {
        created: Vec::new(),
        kept: Vec::new(),
    };

    ensure_dir(root, &mut result)?;
    ensure_dir(&root.join(".pw"), &mut result)?;
    ensure_dir(&root.join("mods"), &mut result)?;
    ensure_dir(&root.join("mods").join("server"), &mut result)?;
    ensure_dir(&root.join("mods").join("client"), &mut result)?;
    ensure_dir(&root.join("mods").join("common"), &mut result)?;
    ensure_dir(&root.join("resourcepacks"), &mut result)?;
    ensure_dir(&root.join("shaderpacks"), &mut result)?;

    write_if_missing(
        &root.join(".pw").join("config.toml"),
        DEFAULT_CONFIG,
        &mut result,
    )?;
    write_if_missing(&root.join("pack.toml"), DEFAULT_PACK, &mut result)?;
    write_if_missing(&root.join("index.toml"), DEFAULT_INDEX, &mut result)?;
    write_if_missing(&root.join(".packwizignore"), "", &mut result)?;

    Ok(result)
}

fn ensure_dir(path: &Path, result: &mut InitResult) -> Result<(), String> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_dir() {
            return Err(format!("expected directory at {}", path.display()));
        }
        result.kept.push(path.to_path_buf());
        return Ok(());
    }
    fs::create_dir_all(path)
        .map_err(|err| format!("failed to create {}: {err}", path.display()))?;
    result.created.push(path.to_path_buf());
    Ok(())
}

fn write_if_missing(path: &Path, text: &str, result: &mut InitResult) -> Result<(), String> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_file() {
            return Err(format!("expected file at {}", path.display()));
        }
        result.kept.push(path.to_path_buf());
        return Ok(());
    }
    fs::write(path, text).map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    result.created.push(path.to_path_buf());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn creates_default_pack_roots() {
        let root = temp_root("init-roots");

        init_pack(&root).unwrap();

        assert!(root.join("mods").is_dir());
        assert!(root.join("resourcepacks").is_dir());
        assert!(root.join("shaderpacks").is_dir());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_existing_file_where_directory_is_expected() {
        let root = temp_root("init-dir-type");
        fs::write(root.join("mods"), b"not a dir").unwrap();

        let result = init_pack(&root);

        assert!(result.is_err());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_existing_directory_where_file_is_expected() {
        let root = temp_root("init-file-type");
        fs::create_dir_all(root.join(".pw").join("config.toml")).unwrap();

        let result = init_pack(&root);

        assert!(result.is_err());

        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(prefix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "{}-{}",
            prefix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
