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
    let hash = crate::sha256::sha256_file_hex_operation(path)?;
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
    #[cfg(windows)]
    validate_windows_prefix(path)?;
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut output = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Prefix(_) => {
                // A Windows drive/UNC prefix is not a filesystem path until
                // its root separator has been appended.
                output.push(part.as_os_str());
                continue;
            }
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
                // macOS exposes standard system directories through aliases.
                // Resolve only these root aliases; pack-internal links remain
                // rejected before any transaction or cleanup can follow them.
                #[cfg(target_os = "macos")]
                if ["/var", "/tmp", "/etc"]
                    .iter()
                    .any(|alias| output == Path::new(alias))
                {
                    let resolved = fs::canonicalize(&output)?;
                    if resolved == Path::new("/private").join(output.file_name().unwrap()) {
                        output = resolved;
                        continue;
                    }
                }
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
    #[cfg(windows)]
    validate_windows_prefix(path)?;
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

#[cfg(windows)]
fn validate_windows_prefix(path: &Path) -> Result<()> {
    use std::path::Prefix;
    let mut parts = path.components();
    if let Some(Component::Prefix(prefix)) = parts.next() {
        if !matches!(
            prefix.kind(),
            Prefix::Disk(_)
                | Prefix::UNC(_, _)
                | Prefix::VerbatimDisk(_)
                | Prefix::VerbatimUNC(_, _)
        ) || !matches!(parts.next(), Some(Component::RootDir))
        {
            return Err(Error::named(
                ErrorCode::Invalid,
                "invalid_path",
                "invalid path",
            ));
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn rejects_drive_relative_and_device_namespace_paths() {
        for path in [
            r"C:child",
            r"C:",
            r"\\.\PhysicalDrive0",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1\file",
        ] {
            assert_eq!(
                absolute(Path::new(path)).unwrap_err().code,
                ErrorCode::Invalid
            );
            assert_eq!(
                canonical(Path::new(path)).unwrap_err().code,
                ErrorCode::Invalid
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn absolute_accepts_exact_macos_system_aliases() {
        for alias in ["/var", "/tmp", "/etc"] {
            let path = Path::new(alias);
            let expected = Path::new("/private").join(path.file_name().unwrap());
            assert_eq!(absolute(path).unwrap(), expected);
            let child = unique_id();
            assert_eq!(absolute(&path.join(&child)).unwrap(), expected.join(child));
        }
    }

    #[test]
    fn absolute_accepts_system_temp_and_canonical_roots_with_missing_children() {
        let root = std::env::temp_dir().join(unique_id());
        fs::create_dir_all(&root).unwrap();
        let canonical_root = fs::canonicalize(&root).unwrap();
        for base in [&root, &canonical_root] {
            assert_eq!(absolute(base).unwrap(), canonical_root);
            assert_eq!(
                absolute(&base.join("missing/child")).unwrap(),
                canonical_root.join("missing/child")
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn absolute_still_rejects_links_inside_a_pack() {
        let root = std::env::temp_dir().join(unique_id());
        fs::create_dir_all(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        let error = absolute(&root.join("link/missing")).unwrap_err();
        assert_eq!(error.message.as_deref(), Some("symlink_rejected"));
        fs::remove_dir_all(root).unwrap();
    }
}
