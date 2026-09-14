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
