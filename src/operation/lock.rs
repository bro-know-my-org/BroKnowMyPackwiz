use super::{Error, ErrorCode, Result, durable};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

pub struct WriteLocks {
    _files: Vec<File>,
}

impl WriteLocks {
    pub fn acquire(state: &Path, paths: &[PathBuf]) -> Result<Self> {
        let mut paths = paths
            .iter()
            .map(|p| durable::absolute(p))
            .collect::<Result<Vec<_>>>()?;
        paths.sort();
        paths.dedup();
        let dir = state.join("locks");
        fs::create_dir_all(&dir)?;
        let mut files = Vec::new();
        for path in paths {
            let digest = Sha256::digest(path.to_string_lossy().as_bytes());
            let name = digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
                + ".lock";
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(dir.join(name))?;
            file.try_lock_exclusive()
                .map_err(|err| Error::new(ErrorCode::Busy, format!("{}: {err}", path.display())))?;
            files.push(file);
        }
        Ok(Self { _files: files })
    }
}
