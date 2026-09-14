use super::{Error, ErrorCode, Result};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub fn unique_id() -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    )
}

pub fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::new(ErrorCode::Invalid, "missing parent"))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".bkmpw-{}.tmp", unique_id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        sync_dir(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub fn replace(source: &Path, target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| Error::new(ErrorCode::Invalid, "missing parent"))?;
    let temp = parent.join(format!(".bkmpw-{}.tmp", unique_id()));
    let result = (|| {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        std::io::copy(&mut File::open(source)?, &mut output)?;
        output.sync_all()?;
        fs::set_permissions(&temp, fs::metadata(source)?.permissions())?;
        fs::rename(&temp, target)?;
        sync_dir(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub fn fingerprint(path: &Path) -> Result<Option<String>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(Error::new(
            ErrorCode::Invalid,
            format!("not a regular file: {}", path.display()),
        ));
    }
    let hash = crate::sha256::sha256_file_hex(path).map_err(Error::from)?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode()
    };
    #[cfg(not(unix))]
    let mode = u32::from(meta.permissions().readonly());
    Ok(Some(format!("{hash}:{mode}")))
}

/// Resolve existing ancestors and reject symlinks, including a missing leaf's parents.
pub fn absolute(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut output = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => continue,
            Component::ParentDir => {
                if !output.pop() {
                    return Err(Error::new(ErrorCode::Invalid, "invalid parent path"));
                }
            }
            _ => output.push(part.as_os_str()),
        }
        match fs::symlink_metadata(&output) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(Error::new(
                    ErrorCode::Invalid,
                    format!("symlink: {}", output.display()),
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    // Canonicalizing the nearest existing ancestor also normalizes Windows drive spelling.
    let mut ancestor = output.as_path();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        missing.push(
            ancestor
                .file_name()
                .ok_or_else(|| Error::new(ErrorCode::Invalid, "invalid path"))?
                .to_os_string(),
        );
        ancestor = ancestor
            .parent()
            .ok_or_else(|| Error::new(ErrorCode::Invalid, "invalid parent"))?;
    }
    let mut canonical = fs::canonicalize(ancestor)?;
    for name in missing.into_iter().rev() {
        canonical.push(name);
    }
    Ok(canonical)
}

pub fn user_state() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("org", "bro-know-my", "bkmpw")
        .ok_or_else(|| Error::new(ErrorCode::Invalid, "user directories unavailable"))?;
    let path = dirs
        .state_dir()
        .unwrap_or(dirs.data_local_dir())
        .to_path_buf();
    fs::create_dir_all(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}
