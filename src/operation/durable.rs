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
    #[cfg(test)]
    super::faults::check(super::faults::Point::WriteBefore, path)?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::named(ErrorCode::Invalid, "missing_parent", "missing parent"))?;
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
        #[cfg(test)]
        super::faults::check(super::faults::Point::WritePublished, path)?;
        sync_dir(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub fn replace(source: &Path, target: &Path) -> Result<()> {
    #[cfg(test)]
    super::faults::check(super::faults::Point::ReplaceBefore, target)?;
    let parent = target
        .parent()
        .ok_or_else(|| Error::named(ErrorCode::Invalid, "missing_parent", "missing parent"))?;
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
        #[cfg(test)]
        super::faults::check(super::faults::Point::ReplacePublished, target)?;
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
        return Err(Error::named(
            ErrorCode::Invalid,
            "not_regular_file",
            format!("not a regular file: {}", path.display()),
        )
        .context(path.display().to_string()));
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
                    return Err(Error::named(
                        ErrorCode::Invalid,
                        "invalid_parent_path",
                        "invalid parent path",
                    ));
                }
            }
            _ => output.push(part.as_os_str()),
        }
        match fs::symlink_metadata(&output) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(Error::named(
                    ErrorCode::Invalid,
                    "symlink_rejected",
                    format!("symlink: {}", output.display()),
                )
                .context(output.display().to_string()));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    canonical(&output)
}

/// Resolve a user-selected root or lock identity, including system directory aliases.
/// Transaction targets still use `absolute` to reject symlinks inside that root.
pub fn canonical(path: &Path) -> Result<PathBuf> {
    let output = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    // Canonicalizing the nearest existing ancestor also normalizes Windows drive spelling.
    let mut ancestor = output.as_path();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        missing.push(
            ancestor
                .file_name()
                .ok_or_else(|| Error::named(ErrorCode::Invalid, "invalid_path", "invalid path"))?
                .to_os_string(),
        );
        ancestor = ancestor
            .parent()
            .ok_or_else(|| Error::named(ErrorCode::Invalid, "invalid_parent", "invalid parent"))?;
    }
    let mut canonical = fs::canonicalize(ancestor)?;
    for name in missing.into_iter().rev() {
        canonical.push(name);
    }
    Ok(canonical)
}

pub fn user_state() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("org", "bro-know-my", "bkmpw").ok_or_else(|| {
        Error::named(
            ErrorCode::Invalid,
            "user_directories_unavailable",
            "user directories unavailable",
        )
    })?;
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
