use super::{Error, ErrorCode, Result, durable};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    Init,
    Refresh,
    Inspect,
    List,
    Scan,
    Check,
    PreparePack,
    PrepareServer,
    Modlist,
    ExportClient,
    ExportServer,
    ExportServerInstaller,
    ExportCurseForge,
    DownloadFiles,
    Sync,
    InstallLocal,
    Remove,
    Hash,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Output {
    None,
    File,
    Directory,
}
impl Kind {
    pub fn command(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Refresh => "refresh",
            Self::Inspect => "inspect",
            Self::List => "list",
            Self::Scan => "scan",
            Self::Check => "check",
            Self::PreparePack => "prepare-pack",
            Self::PrepareServer => "prepare-server",
            Self::Modlist => "modlist",
            Self::ExportClient => "export-client",
            Self::ExportServer => "export-server",
            Self::ExportServerInstaller => "export-server-installer",
            Self::ExportCurseForge => "export-curseforge",
            Self::DownloadFiles => "download-files",
            Self::Sync => "sync",
            Self::InstallLocal => "install-local",
            Self::Remove => "remove",
            Self::Hash => "hash",
        }
    }
    pub fn readonly(self) -> bool {
        matches!(
            self,
            Self::Inspect | Self::List | Self::Scan | Self::Check | Self::Hash
        )
    }
    pub fn output(self) -> Output {
        match self {
            Self::ExportClient
            | Self::ExportServer
            | Self::ExportServerInstaller
            | Self::ExportCurseForge => Output::File,
            Self::PrepareServer | Self::Modlist | Self::Sync | Self::InstallLocal => {
                Output::Directory
            }
            _ => Output::None,
        }
    }
    pub fn installation(self) -> bool {
        matches!(self, Self::DownloadFiles | Self::Sync | Self::InstallLocal)
    }
    pub fn default_output(self) -> Option<&'static str> {
        match self {
            Self::ExportClient => Some("client-full.zip"),
            Self::ExportServer => Some("server-pack.zip"),
            Self::ExportServerInstaller => Some("server-installer.zip"),
            Self::ExportCurseForge => Some("curseforge-export.zip"),
            Self::PrepareServer => Some(".bkmpw/server-pack"),
            Self::Modlist => Some("docs/generated"),
            Self::Sync => Some(""),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub guard: Option<super::preview::Guard>,
    pub kind: Kind,
    pub output: Option<PathBuf>,
    pub input: Option<PathBuf>,
    pub side: String,
    pub root_dir: String,
    pub algorithm: String,
    pub names: Vec<String>,
    pub jobs: Option<usize>,
    pub retries: Option<usize>,
    pub retry_delay_seconds: Option<u64>,
    pub force: bool,
    pub cleanup: Option<bool>,
}
impl Request {
    pub fn new(kind: Kind) -> Self {
        Self {
            guard: None,
            kind,
            output: None,
            input: None,
            side: "both".into(),
            root_dir: String::new(),
            algorithm: "sha256".into(),
            names: Vec::new(),
            jobs: None,
            retries: None,
            retry_delay_seconds: None,
            force: false,
            cleanup: None,
        }
    }
    pub fn target(&self, root: &Path) -> Result<Option<PathBuf>> {
        if self.kind.output() == Output::None {
            return Ok(None);
        }
        let path = self
            .output
            .clone()
            .or_else(|| self.kind.default_output().map(PathBuf::from))
            .ok_or_else(|| invalid("output_required"))?;
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        durable::canonical(&path).map(Some)
    }
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.side.as_str(), "client" | "server" | "both") {
            return Err(invalid("unknown_side"));
        }
        if self.jobs == Some(0) {
            return Err(invalid("jobs_must_be_positive"));
        }
        if !self.root_dir.is_empty() {
            crate::operation::paths::relative(&self.root_dir)?;
        }
        if self.kind == Kind::Remove {
            if self.names.is_empty() {
                return Err(invalid("remove_selection_required"));
            }
            for name in &self.names {
                crate::operation::paths::relative(name)?;
            }
        }
        if self.kind == Kind::Hash
            && (self.input.is_none()
                || !matches!(
                    self.algorithm.as_str(),
                    "sha1" | "sha256" | "sha512" | "murmur2"
                ))
        {
            return Err(invalid("hash_input_required"));
        }
        Ok(())
    }
    /// All filesystem arguments supplied here have already been mapped to the
    /// private workspaces; only read-only hash input may be outside them.
    pub fn arguments(&self, root: &Path, target: Option<&Path>) -> Result<Vec<Vec<OsString>>> {
        self.validate()?;
        if self.kind == Kind::Hash {
            let input = self.input.as_ref().unwrap();
            let input = durable::canonical(&if input.is_absolute() {
                input.clone()
            } else {
                root.join(input)
            })?;
            if !input.is_file() {
                return Err(invalid("hash_input_required"));
            }
            return Ok(vec![vec![
                self.algorithm.clone().into(),
                input.into_os_string(),
            ]]);
        }
        if self.kind == Kind::Remove {
            return Ok(self
                .names
                .iter()
                .map(|name| vec![root.as_os_str().into(), name.into()])
                .collect());
        }
        let mut args = vec![root.as_os_str().into()];
        if self.kind.output() != Output::None {
            args.push(
                target
                    .ok_or_else(|| invalid("output_required"))?
                    .as_os_str()
                    .into(),
            );
        }
        if matches!(
            self.kind,
            Kind::Sync | Kind::InstallLocal | Kind::ExportCurseForge
        ) {
            args.push(self.side.clone().into());
        }
        if self.kind == Kind::ExportClient && !self.root_dir.is_empty() {
            args.push(self.root_dir.clone().into());
        }
        if self.kind.installation() {
            if let Some(jobs) = self.jobs {
                args.push(jobs.to_string().into());
            }
            if self.force {
                args.push("--force".into());
            }
            if let Some(retries) = self.retries {
                args.extend(["--retries".into(), retries.to_string().into()]);
            }
            if let Some(delay) = self.retry_delay_seconds {
                args.extend(["--retry-delay-seconds".into(), delay.to_string().into()]);
            }
            if self.kind == Kind::InstallLocal {
                if let Some(cleanup) = self.cleanup {
                    args.push(if cleanup { "--cleanup" } else { "--no-cleanup" }.into());
                }
            }
        }
        Ok(vec![args])
    }
}
fn invalid(key: &str) -> Error {
    Error::key(ErrorCode::Invalid, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installation_and_export_arguments_keep_paths_and_options_separate() {
        let mut request = Request::new(Kind::InstallLocal);
        request.jobs = Some(3);
        request.retries = Some(2);
        request.cleanup = Some(false);
        let args = request
            .arguments(
                Path::new("pack with spaces"),
                Some(Path::new("target with spaces")),
            )
            .unwrap();
        assert_eq!(
            args[0],
            vec![
                OsString::from("pack with spaces"),
                "target with spaces".into(),
                "both".into(),
                "3".into(),
                "--retries".into(),
                "2".into(),
                "--no-cleanup".into()
            ]
        );
        request.jobs = Some(0);
        assert!(request.validate().is_err());
        let mut request = Request::new(Kind::ExportClient);
        request.root_dir = "../outside".into();
        assert!(request.validate().is_err());
    }
}
