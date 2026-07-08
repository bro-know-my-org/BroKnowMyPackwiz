use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const RUN_DIR: &str = ".bkmpw/runs";
const TRASH_DIR: &str = ".bkmpw/trash";
const INSTALL_MARKER: &str = "bkmpw-tmp-run-";
const DOWNLOAD_MARKER: &str = "bkmpw-download-tmp-run-";
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

static RUN_ID: LazyLock<String> = LazyLock::new(|| {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_nanos());
    format!("{:x}{:x}", std::process::id(), nanos)
});

#[derive(Debug, Clone)]
pub struct StaleTempCleanup {
    pub removed: usize,
    pub warnings: Vec<String>,
}

pub struct RunGuard {
    stop: Option<mpsc::Sender<()>>,
    heartbeat: Option<thread::JoinHandle<()>>,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = self.heartbeat.take();
    }
}

pub fn run_id() -> &'static str {
    &RUN_ID
}

pub fn install_temp_marker() -> &'static str {
    INSTALL_MARKER
}

pub fn download_temp_marker() -> &'static str {
    DOWNLOAD_MARKER
}

pub fn start_run(root: &Path) -> Result<RunGuard, String> {
    let run_dir = root.join(RUN_DIR);
    fs::create_dir_all(&run_dir)
        .map_err(|err| format!("failed to create {}: {err}", run_dir.display()))?;
    let lock_path = run_dir.join(format!("{}.lock", run_id()));
    write_lock(&lock_path)?;

    let (stop, stop_rx) = mpsc::channel();
    let heartbeat_path = lock_path.clone();
    let heartbeat = thread::spawn(move || {
        while stop_rx.recv_timeout(HEARTBEAT_INTERVAL).is_err() {
            if let Err(err) = write_lock(&heartbeat_path) {
                eprintln!("warn: failed to refresh temp cleanup run lock: {err}");
            }
        }
    });

    Ok(RunGuard {
        stop: Some(stop),
        heartbeat: Some(heartbeat),
    })
}

pub fn cleanup_stale_temp_files(
    root: &Path,
    managed_roots: &[PathBuf],
    stale_after: Duration,
) -> StaleTempCleanup {
    let mut result = StaleTempCleanup {
        removed: 0,
        warnings: Vec::new(),
    };
    let mut roots = BTreeSet::new();
    for managed_root in managed_roots {
        roots.insert(managed_root.clone());
    }

    for managed_root in roots {
        if managed_root.has_root() {
            result.warnings.push(format!(
                "skipping absolute managed root during stale temp cleanup: {}",
                managed_root.display()
            ));
            continue;
        }
        let full_root = root.join(&managed_root);
        visit_temp_files(root, &full_root, stale_after, &mut result);
    }
    result
}

fn write_lock(path: &Path) -> Result<(), String> {
    fs::write(
        path,
        format!("run_id={}\npid={}\n", run_id(), std::process::id()),
    )
    .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn visit_temp_files(
    pack_root: &Path,
    current: &Path,
    stale_after: Duration,
    result: &mut StaleTempCleanup,
) {
    let entries = match fs::read_dir(current) {
        Ok(entries) => entries,
        Err(err) => {
            result.warnings.push(format!(
                "failed to read temp cleanup directory {}: {err}",
                current.display()
            ));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                result.warnings.push(format!(
                    "failed to read temp cleanup entry in {}: {err}",
                    current.display()
                ));
                continue;
            }
        };
        let path = entry.path();
        if is_symlink(&path) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                result.warnings.push(format!(
                    "failed to read temp cleanup file type for {}: {err}",
                    path.display()
                ));
                continue;
            }
        };
        if file_type.is_dir() {
            visit_temp_files(pack_root, &path, stale_after, result);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(run_id) = temp_run_id(&path) else {
            continue;
        };
        if !is_stale_enough(&path, stale_after) {
            continue;
        }
        if run_lock_is_active(pack_root, run_id, stale_after) {
            continue;
        }
        if let Err(err) = quarantine_and_remove(pack_root, &path, run_id) {
            result.warnings.push(err);
        } else {
            result.removed += 1;
        }
    }
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

fn temp_run_id(path: &Path) -> Option<&str> {
    let name = path.file_name()?.to_str()?;
    parse_run_id_after(name, INSTALL_MARKER).or_else(|| parse_run_id_after(name, DOWNLOAD_MARKER))
}

fn parse_run_id_after<'a>(name: &'a str, marker: &str) -> Option<&'a str> {
    let rest = name.split_once(marker)?.1;
    let (run_id, _) = rest.split_once('-')?;
    if run_id.is_empty() || !run_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(run_id)
}

fn is_stale_enough(path: &Path, stale_after: Duration) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age >= stale_after)
}

fn run_lock_is_active(root: &Path, run_id: &str, stale_after: Duration) -> bool {
    let lock_path = root.join(RUN_DIR).join(format!("{run_id}.lock"));
    fs::metadata(lock_path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < stale_after)
}

fn quarantine_and_remove(root: &Path, path: &Path, run_id: &str) -> Result<(), String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|err| format!("failed to relativize {}: {err}", path.display()))?;
    let trash_path = root.join(TRASH_DIR).join(run_id).join(rel);
    if let Some(parent) = trash_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    fs::rename(path, &trash_path).map_err(|err| {
        format!(
            "failed to quarantine stale temp file {}: {err}",
            path.display()
        )
    })?;
    fs::remove_file(&trash_path).map_err(|err| {
        format!(
            "failed to remove quarantined temp file {}: {err}",
            trash_path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_current_temp_run_ids_only() {
        assert_eq!(
            temp_run_id(Path::new("foo.jar.bkmpw-tmp-run-abcd-1")),
            Some("abcd")
        );
        assert_eq!(
            temp_run_id(Path::new("foo.jar.bkmpw-download-tmp-run-123f-1")),
            Some("123f")
        );
        assert_eq!(temp_run_id(Path::new("foo.jar.bkmpw-tmp-123-1")), None);
        assert_eq!(temp_run_id(Path::new("foo.jar.bkmpw-tmp-run-xyz-1")), None);
    }

    #[test]
    fn cleanup_removes_stale_temp_without_active_lock() {
        let root = temp_root("cleanup-stale-temp");
        fs::create_dir_all(root.join("mods")).unwrap();
        let temp = root.join("mods").join("foo.jar.bkmpw-tmp-run-abcdef-0");
        fs::write(&temp, b"partial").unwrap();
        fs::write(root.join("mods").join("foo.jar"), b"real").unwrap();

        let result =
            cleanup_stale_temp_files(&root, &[PathBuf::from("mods")], Duration::from_secs(0));

        assert_eq!(result.removed, 1);
        assert!(result.warnings.is_empty());
        assert!(!temp.exists());
        assert!(root.join("mods").join("foo.jar").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_skips_temp_with_active_lock() {
        let root = temp_root("cleanup-active-lock");
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::create_dir_all(root.join(RUN_DIR)).unwrap();
        fs::write(root.join(RUN_DIR).join("abcdef.lock"), b"live").unwrap();
        let temp = root.join("mods").join("foo.jar.bkmpw-tmp-run-abcdef-0");
        fs::write(&temp, b"partial").unwrap();

        let result = cleanup_stale_temp_files(
            &root,
            &[PathBuf::from("mods")],
            Duration::from_secs(60 * 60),
        );

        assert_eq!(result.removed, 0);
        assert!(result.warnings.is_empty());
        assert!(temp.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_skips_absolute_managed_roots() {
        let root = temp_root("cleanup-absolute-root");
        let absolute = std::env::temp_dir();

        let result = cleanup_stale_temp_files(&root, &[absolute.clone()], Duration::from_secs(0));

        assert_eq!(result.removed, 0);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains(&absolute.display().to_string()));

        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("bkmpw-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
