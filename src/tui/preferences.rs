use super::i18n::Language;
use crate::operation::{Error, ErrorCode, Result, durable};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub language: Option<Language>,
    pub recent: Vec<PathBuf>,
    pub editor: Option<String>,
}

impl Preferences {
    fn file() -> Result<PathBuf> {
        directories::ProjectDirs::from("org", "bro-know-my", "bkmpw")
            .map(|dirs| dirs.config_dir().join("tui.json"))
            .ok_or_else(|| {
                Error::named(
                    ErrorCode::Invalid,
                    "user_config_unavailable",
                    "user configuration directory unavailable",
                )
            })
    }
    pub fn load() -> Result<Self> {
        let path = Self::file()?;
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self) -> Result<()> {
        durable::write(
            &Self::file()?,
            &serde_json::to_vec_pretty(self)
                .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?,
        )
    }
    pub fn remember(&mut self, root: &Path) -> Result<()> {
        let root = durable::canonical(root)?;
        self.recent.retain(|p| p != &root);
        self.recent.insert(0, root);
        self.recent.truncate(12);
        self.save()
    }
}
