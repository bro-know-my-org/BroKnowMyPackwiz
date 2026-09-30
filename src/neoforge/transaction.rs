//! Write-ahead snapshots: no instance file changes before a durable ready marker.
use super::Result;
use crate::sha256::sha256_file_hex as hash;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const CONTROL: &str = ".bkmpw-neoforge";
pub struct Lock {
    _file: fs::File,
}
pub fn lock(root: &Path) -> Result<Lock> {
    let dir = root.join(CONTROL);
    safe_path(&dir)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("lock");
    safe_path(&path)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.try_lock_exclusive()
        .map_err(|e| format!("another loader operation is active: {e}"))?;
    Ok(Lock { _file: file })
}
pub struct Change {
    pub path: PathBuf,
    pub source: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    before: Option<String>,
    after: String,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    root: PathBuf,
    entries: Vec<Entry>,
}
pub fn safe_path(path: &Path) -> Result<()> {
    let mut cursor = Some(path);
    while let Some(p) = cursor {
        match fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(format!(
                    "symlink/reparse path is not supported: {}",
                    p.display()
                ));
            }
            Ok(m) if cfg!(windows) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        return Err(format!("reparse path: {}", p.display()));
                    }
                }
                let _ = m;
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("{}: {e}", p.display()));
            }
            _ => {}
        }
        cursor = p.parent();
    }
    Ok(())
}
fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    let _ = path;
    Ok(())
}
fn marker(dir: &Path, name: &str) -> Result<()> {
    let path = dir.join(name);
    if path.exists() {
        return Ok(());
    }
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    sync_dir(dir)
}
fn copy_synced(src: &Path, dst: &Path) -> Result<()> {
    fs::copy(src, dst).map_err(|e| format!("{} → {}: {e}", src.display(), dst.display()))?;
    fs::File::open(dst)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}
fn replace(src: &Path, dst: &Path) -> Result<()> {
    safe_path(dst)?;
    let parent = dst.parent().ok_or("missing file parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = dst.with_file_name(format!(
        "{}.bkmpw-loader-tmp",
        dst.file_name().ok_or("missing filename")?.to_string_lossy()
    ));
    safe_path(&temp)?;
    // A previous interrupted write may have left this owned temporary file.
    if temp.exists() {
        fs::remove_file(&temp).map_err(|e| e.to_string())?;
    }
    copy_synced(src, &temp)?;
    if dst.is_file() {
        fs::set_permissions(
            &temp,
            fs::metadata(dst).map_err(|e| e.to_string())?.permissions(),
        )
        .map_err(|e| e.to_string())?;
    }
    match fs::rename(&temp, dst) {
        Ok(()) => {}
        Err(_) if cfg!(windows) && dst.is_file() => {
            // The original is already in the durable snapshot. Recovery also
            // handles interruption between removal and rename on Windows.
            fs::remove_file(dst).map_err(|e| e.to_string())?;
            fs::rename(&temp, dst).map_err(|e| e.to_string())?;
        }
        Err(e) => return Err(format!("{}: {e}", dst.display())),
    }
    sync_dir(parent)
}
fn prepare(root: &Path, changes: &[Change]) -> Result<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let dir = root.join(CONTROL).join(format!("old_{stamp}"));
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    fs::create_dir(dir.join("files")).map_err(|e| e.to_string())?;
    let mut journal = Journal {
        root: root.to_owned(),
        entries: vec![],
    };
    for (i, change) in changes.iter().enumerate() {
        if !change.path.is_absolute() || journal.entries.iter().any(|e| e.path == change.path) {
            return Err("duplicate or non-absolute transaction path".into());
        }
        safe_path(&change.path)?;
        safe_path(&change.source)?;
        let before = if change.path.exists() {
            if !change.path.is_file() {
                return Err(format!("not a regular file: {}", change.path.display()));
            }
            let old = hash(&change.path)?;
            copy_synced(&change.path, &dir.join("files").join(i.to_string()))?;
            if hash(&dir.join("files").join(i.to_string()))? != old {
                return Err("file changed during backup".into());
            }
            Some(old)
        } else {
            None
        };
        journal.entries.push(Entry {
            path: change.path.clone(),
            before,
            after: hash(&change.source)?,
        });
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("journal.json"))
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(&journal).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    sync_dir(&dir.join("files"))?;
    marker(&dir, "ready")?;
    sync_dir(&root.join(CONTROL))?;
    Ok(dir)
}
pub fn apply(root: &Path, changes: &[Change], verify: impl Fn() -> Result<()>) -> Result<PathBuf> {
    let dir = prepare(root, changes)?;
    let journal: Journal = serde_json::from_value(super::json(&dir.join("journal.json"))?)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        verify()?;
        for (change, entry) in changes.iter().zip(&journal.entries) {
            let current = if change.path.exists() {
                Some(hash(&change.path)?)
            } else {
                None
            };
            if current != entry.before || hash(&change.source)? != entry.after {
                return Err(format!(
                    "file changed since plan: {}",
                    change.path.display()
                ));
            }
            replace(&change.source, &change.path)?;
            if hash(&change.path)? != entry.after {
                return Err("post-write hash mismatch".into());
            }
        }
        verify()?;
        marker(&dir, "committed")
    })();
    if let Err(e) = result {
        return match restore(root, &dir) {
            Ok(()) => Err(format!(
                "{e}; original installation restored; backup {}",
                dir.display()
            )),
            Err(r) => Err(format!(
                "{e}; recovery incomplete: {r}; run loader recover; backup {}",
                dir.display()
            )),
        };
    }
    Ok(dir)
}
fn restore(root: &Path, dir: &Path) -> Result<()> {
    safe_path(dir)?;
    let journal: Journal = serde_json::from_value(super::json(&dir.join("journal.json"))?)
        .map_err(|e| e.to_string())?;
    if journal.root != root {
        return Err("backup belongs to another instance".into());
    }
    // Validate every path before restoring anything. Refuse to destroy edits
    // made after upgrade; the user can preserve/reconcile those edits first.
    for (i, entry) in journal.entries.iter().enumerate() {
        safe_path(&entry.path)?;
        if let Some(before) = &entry.before {
            let backup = dir.join("files").join(i.to_string());
            safe_path(&backup)?;
            if hash(&backup)? != *before {
                return Err(format!("damaged backup {}", backup.display()));
            }
        }
        if entry.path.exists() {
            let current = hash(&entry.path)?;
            if current != entry.after && Some(&current) != entry.before.as_ref() {
                return Err(format!(
                    "modified since upgrade: {}; preserve edits before rollback",
                    entry.path.display()
                ));
            }
        }
    }
    marker(dir, "restoring")?;
    for (i, entry) in journal.entries.iter().enumerate().rev() {
        if entry.before.is_some() {
            replace(&dir.join("files").join(i.to_string()), &entry.path)?;
        } else if entry.path.exists() {
            fs::remove_file(&entry.path).map_err(|e| e.to_string())?;
            sync_dir(entry.path.parent().ok_or("missing parent")?)?;
        }
    }
    marker(dir, "rolled-back")
}
pub fn backups(root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = fs::read_dir(root.join(CONTROL))
        .map_err(|e| e.to_string())?
        .map(|e| e.map(|e| e.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>>>()?;
    dirs.retain(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("old_"))
            && p.is_dir()
    });
    dirs.sort();
    Ok(dirs)
}
pub fn recover(root: &Path) -> Result<Vec<PathBuf>> {
    let mut recovered = vec![];
    for dir in backups(root)?.into_iter().rev() {
        if dir.join("ready").exists()
            && !dir.join("rolled-back").exists()
            && (!dir.join("committed").exists() || dir.join("restoring").exists())
        {
            restore(root, &dir)?;
            recovered.push(dir);
        }
    }
    Ok(recovered)
}
pub fn rollback(root: &Path) -> Result<PathBuf> {
    recover(root)?;
    let dir = backups(root)?
        .into_iter()
        .rev()
        .find(|p| p.join("committed").exists() && !p.join("rolled-back").exists())
        .ok_or("no committed upgrade to roll back")?;
    restore(root, &dir)?;
    Ok(dir)
}
pub fn clean(root: &Path, name: &str) -> Result<()> {
    crate::pathutil::safe_filename(name)?;
    if !name.starts_with("old_") {
        return Err("only named old_ loader backups may be cleaned".into());
    }
    let dir = root.join(CONTROL).join(name);
    safe_path(&dir)?;
    if dir.join("ready").exists()
        && !dir.join("rolled-back").exists()
        && (!dir.join("committed").exists() || dir.join("restoring").exists())
    {
        return Err("recover interrupted transaction before cleaning".into());
    }
    // Keep history coherent: remove only the oldest backup, or a rolled-back one.
    let oldest = backups(root)?
        .into_iter()
        .find(|p| p.join("committed").exists() && !p.join("rolled-back").exists());
    if dir.join("committed").exists()
        && !dir.join("rolled-back").exists()
        && oldest.as_ref() != Some(&dir)
    {
        return Err("clean committed backups oldest first".into());
    }
    fs::remove_dir_all(dir).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, Vec<Change>) {
        let root = std::env::temp_dir().join(format!(
            "bkmpw transaction spaces {}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("run.sh"), "old").unwrap();
        fs::write(root.join("new"), "new").unwrap();
        let changes = vec![
            Change {
                path: root.join("run.sh"),
                source: root.join("new"),
            },
            Change {
                path: root.join("library"),
                source: root.join("new"),
            },
        ];
        (root, changes)
    }
    #[test]
    fn commit_rollback_preserves_unmanaged_and_backup() {
        let (root, changes) = fixture();
        let _lock = lock(&root).unwrap();
        fs::write(root.join("manual.jar"), "keep").unwrap();
        let dir = apply(&root, &changes, || Ok(())).unwrap();
        assert_eq!(super::super::read(&root.join("run.sh")).unwrap(), "new");
        rollback(&root).unwrap();
        assert!(!root.join("library").exists());
        assert_eq!(super::super::read(&root.join("run.sh")).unwrap(), "old");
        assert_eq!(
            super::super::read(&root.join("manual.jar")).unwrap(),
            "keep"
        );
        assert!(dir.exists());
        clean(&root, dir.file_name().unwrap().to_str().unwrap()).unwrap();
        drop(_lock);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failure_and_interrupted_switch_restore_from_disk() {
        let (root, changes) = fixture();
        let _lock = lock(&root).unwrap();
        let calls = std::cell::Cell::new(0);
        assert!(
            apply(&root, &changes, || {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    Err("switch failure".into())
                } else {
                    Ok(())
                }
            })
            .unwrap_err()
            .contains("restored")
        );
        let dir = prepare(&root, &changes).unwrap();
        replace(&changes[0].source, &changes[0].path).unwrap();
        assert_eq!(recover(&root).unwrap(), vec![dir]);
        assert_eq!(super::super::read(&root.join("run.sh")).unwrap(), "old");
        assert!(recover(&root).unwrap().is_empty());
        drop(_lock);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn refuses_to_overwrite_post_upgrade_user_edits() {
        let (root, changes) = fixture();
        let _lock = lock(&root).unwrap();
        apply(&root, &changes, || Ok(())).unwrap();
        fs::write(root.join("run.sh"), "user edit").unwrap();
        assert!(rollback(&root).unwrap_err().contains("modified since"));
        drop(_lock);
        fs::remove_dir_all(root).unwrap();
    }
}
