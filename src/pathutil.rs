use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub fn to_slash(path: &Path) -> String {
    let mut out = String::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if !out.is_empty() {
                    out.push('/');
                }
                out.push_str(&part.to_string_lossy());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.is_empty() {
                    out.push('/');
                }
                out.push_str("..");
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    out
}

pub fn normalize_slash(input: &str) -> String {
    let input = input.replace('\\', "/");
    let mut out: Vec<&str> = Vec::new();
    for part in input.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if out.last().is_none_or(|last| *last == "..") {
                    out.push("..");
                } else {
                    out.pop();
                }
            }
            value => out.push(value),
        }
    }
    out.join("/")
}

pub fn safe_filename(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        return Err(format!("unsafe filename: {value}"));
    }
    Ok(value.to_string())
}

pub fn safe_slash_path(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        return Err(format!("unsafe relative path: {value}"));
    }

    let mut parts = Vec::new();
    for part in value.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(format!("unsafe relative path: {value}"));
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

pub fn is_under_slash(rel: &str, parent: &str) -> bool {
    let rel = normalize_slash(rel);
    let parent = normalize_slash(parent);
    let parent = parent.trim_matches('/');
    !parent.is_empty()
        && (rel == parent
            || rel
                .strip_prefix(parent)
                .is_some_and(|rest| rest.starts_with('/')))
}

pub fn strip_comment(line: &str) -> &str {
    let mut escaped = false;
    let mut in_string = false;
    for (idx, ch) in line.char_indices() {
        match ch {
            '\\' if in_string => escaped = !escaped,
            '"' if !escaped => {
                in_string = !in_string;
                escaped = false;
            }
            '#' if !in_string => return &line[..idx],
            _ => escaped = false,
        }
    }
    line
}

pub fn relative_slash(root: &Path, path: &Path) -> Result<String, String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|err| format!("failed to relativize {}: {err}", path.display()))?;
    Ok(to_slash(rel))
}

pub fn join_slash(root: &Path, slash_path: &str) -> PathBuf {
    slash_path.split('/').filter(|part| !part.is_empty()).fold(
        root.to_path_buf(),
        |mut path, part| {
            path.push(part);
            path
        },
    )
}

pub fn write_atomic(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    write_atomic_operation(path, bytes).map_err(|error| error.detail)
}

pub fn write_atomic_operation(
    path: &Path,
    bytes: impl AsRef<[u8]>,
) -> crate::operation::Result<()> {
    use crate::operation::{Error, ErrorCode};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            Error::named(
                ErrorCode::Failed,
                "create_directory_failed",
                format!("failed to create {}: {err}", parent.display()),
            )
            .context(format!("{}: {err}", parent.display()))
        })?;
    }
    let tmp = temp_sibling(path, COUNTER.fetch_add(1, Ordering::Relaxed));
    fs::write(&tmp, bytes.as_ref()).map_err(|err| {
        Error::named(
            ErrorCode::Failed,
            "write_file_failed",
            format!("failed to write {}: {err}", tmp.display()),
        )
        .context(format!("{}: {err}", tmp.display()))
    })?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(first_err) if path.exists() => {
            fs::remove_file(path).map_err(|err| {
                Error::named(
                    ErrorCode::Failed,
                    "remove_file_failed",
                    format!("failed to remove {}: {err}", path.display()),
                )
                .context(format!("{}: {err}", path.display()))
            })?;
            fs::rename(&tmp, path).map_err(|err| {
                let _ = fs::remove_file(&tmp);
                Error::named(
                    ErrorCode::Failed,
                    "replace_file_failed",
                    format!(
                        "failed to replace {}: {err}; initial rename error: {first_err}",
                        path.display()
                    ),
                )
                .context(format!("{}: {err}; {first_err}", path.display()))
            })
        }
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(Error::named(
                ErrorCode::Failed,
                "replace_file_failed",
                format!("failed to replace {}: {err}", path.display()),
            )
            .context(format!("{}: {err}", path.display())))
        }
    }
}

fn temp_sibling(path: &Path, counter: u64) -> PathBuf {
    let mut extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map_or(String::new(), |value| format!("{value}."));
    extension.push_str("bkmpw-tmp-");
    extension.push_str(&std::process::id().to_string());
    extension.push('-');
    extension.push_str(&counter.to_string());
    path.with_extension(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_slash_preserves_leading_parent_segments() {
        assert_eq!(normalize_slash("../../foo"), "../../foo");
        assert_eq!(normalize_slash("foo/../bar"), "bar");
        assert_eq!(normalize_slash("../foo/../bar"), "../bar");
    }

    #[test]
    fn is_under_slash_normalizes_backslash_parent() {
        assert!(is_under_slash("mods/common/foo.pw", r"mods\common"));
    }
}
