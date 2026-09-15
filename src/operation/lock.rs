use super::{Error, ErrorCode, Result, durable};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

pub struct WriteLocks {
    _files: Vec<Held>,
}

struct Held(File);
impl Drop for Held {
    fn drop(&mut self) {
        // A concurrent process spawn may briefly inherit the open file
        // description before exec closes it. Release ownership explicitly.
        let _ = FileExt::unlock(&self.0);
    }
}

pub fn for_command(command: &str, args: &[String]) -> Result<Option<WriteLocks>> {
    if !matches!(
        command,
        "init"
            | "refresh"
            | "pin"
            | "unpin"
            | "remove"
            | "rm"
            | "update"
            | "add-url"
            | "add-file"
            | "add-curseforge"
            | "add-github"
            | "add-resourcepack"
            | "add-shaderpack"
            | "download-files"
            | "sync"
            | "install-local"
            | "install-files"
            | "install-files-headless"
            | "install-files-retry"
            | "export-client"
            | "export-server"
            | "export-server-installer"
            | "export-curseforge"
            | "prepare-pack"
            | "prepare-server"
            | "modlist"
    ) {
        return Ok(None);
    }
    let root = args
        .first()
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let mut paths = vec![root];
    if matches!(
        command,
        "sync"
            | "install-local"
            | "prepare-server"
            | "modlist"
            | "export-client"
            | "export-server"
            | "export-server-installer"
            | "export-curseforge"
    ) {
        if let Some(target) = args.get(1) {
            paths.push(PathBuf::from(target));
        }
    }
    WriteLocks::acquire(&durable::user_state()?, &paths).map(Some)
}

impl WriteLocks {
    pub fn acquire(state: &Path, paths: &[PathBuf]) -> Result<Self> {
        let mut paths = paths
            .iter()
            .map(|p| durable::canonical(p))
            .collect::<Result<Vec<_>>>()?;
        paths.sort();
        paths.dedup();
        let dir = state.join("locks");
        fs::create_dir_all(&dir)?;
        // Serialize only lock registration, never the work itself. Sidecar paths
        // remain readable on Windows while the corresponding OS lock is held.
        let registry = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(durable::absolute(&dir.join("registry"))?)?;
        registry.lock_exclusive()?;
        let _registry = Held(registry);
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .is_none_or(|extension| extension != "lock")
            {
                continue;
            }
            let path = durable::absolute(&entry.path())?;
            let file = OpenOptions::new().read(true).write(true).open(&path)?;
            if file.try_lock_exclusive().is_ok() {
                FileExt::unlock(&file)?;
                continue;
            }
            let sidecar = durable::absolute(&path.with_extension("json"))?;
            let held: PathBuf = fs::read(&sidecar)
                .ok()
                .filter(|bytes| bytes.len() < 65536)
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .ok_or_else(|| {
                    Error::named(
                        ErrorCode::Busy,
                        "active_lock_metadata_unavailable",
                        "active lock metadata unavailable",
                    )
                })?;
            if paths.iter().any(|requested| overlaps(requested, &held)) {
                return Err(Error::new(ErrorCode::Busy, format!("{}", held.display())));
            }
        }
        let mut files = Vec::new();
        for path in paths {
            let digest = Sha256::digest(path.to_string_lossy().as_bytes());
            let name = digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
                + ".lock";
            let lock_path = durable::absolute(&dir.join(name))?;
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&lock_path)?;
            file.try_lock_exclusive()
                .map_err(|err| Error::new(ErrorCode::Busy, format!("{}: {err}", path.display())))?;
            let file = Held(file);
            let sidecar = durable::absolute(&lock_path.with_extension("json"))?;
            durable::write(
                &sidecar,
                &serde_json::to_vec(&path)
                    .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?,
            )?;
            files.push(file);
        }
        Ok(Self { _files: files })
    }
}

fn overlaps(a: &Path, b: &Path) -> bool {
    #[cfg(any(windows, target_os = "macos"))]
    {
        let a = a.to_string_lossy().replace('\\', "/").to_lowercase();
        let b = b.to_string_lossy().replace('\\', "/").to_lowercase();
        a == b
            || a.starts_with(&(b.trim_end_matches('/').to_string() + "/"))
            || b.starts_with(&(a.trim_end_matches('/').to_string() + "/"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        a.starts_with(b) || b.starts_with(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn releasing_owner_unlocks_even_while_a_duplicate_handle_exists() {
        let path = std::env::temp_dir().join(durable::unique_id());
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.lock_exclusive().unwrap();
        let held = Held(file);
        let duplicate = held.0.try_clone().unwrap();
        drop(held);
        let next = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        next.try_lock_exclusive().unwrap();
        FileExt::unlock(&next).unwrap();
        drop(duplicate);
        drop(next);
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn ancestor_and_descendant_writers_conflict_but_siblings_can_run() {
        let base = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(&base).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let state = base.join("state");
        let root = base.join("pack");
        let child = root.join("mods/a.jar");
        let lock = WriteLocks::acquire(&state, &[root.clone(), child.clone()]).unwrap();
        assert!(WriteLocks::acquire(&state, &[child.clone()]).is_err());
        assert!(WriteLocks::acquire(&state, &[base.clone()]).is_err());
        let sibling = WriteLocks::acquire(&state, &[base.join("other-pack")]).unwrap();
        drop(sibling);
        drop(lock);
        let child_lock = WriteLocks::acquire(&state, &[child]).unwrap();
        assert!(WriteLocks::acquire(&state, &[root.clone()]).is_err());
        drop(child_lock);
        drop(WriteLocks::acquire(&state, &[root]).unwrap());
        fs::remove_dir_all(base).unwrap();
    }
}
