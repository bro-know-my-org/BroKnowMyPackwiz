use std::path::PathBuf;

use crate::config::ProjectConfig;

#[derive(Debug, Clone)]
pub struct PackLayout {
    pub metadata_root: PathBuf,
    pub metadata_roots: Vec<PathBuf>,
    pub jar_root: PathBuf,
    pub server_meta: PathBuf,
    pub client_meta: PathBuf,
    pub common_meta: PathBuf,
    pub metadata_extension: String,
}

impl PackLayout {
    pub fn from_config(config: &ProjectConfig) -> Self {
        Self {
            metadata_root: normalize_pathbuf(&config.layout.metadata_root),
            metadata_roots: config
                .layout
                .metadata_roots
                .iter()
                .map(|path| normalize_pathbuf(path))
                .collect(),
            jar_root: normalize_pathbuf(&config.layout.jar_root),
            server_meta: normalize_pathbuf(&config.layout.server_meta),
            client_meta: normalize_pathbuf(&config.layout.client_meta),
            common_meta: normalize_pathbuf(&config.layout.common_meta),
            metadata_extension: config.layout.metadata_extension.clone(),
        }
    }
}

fn normalize_pathbuf(path: &std::path::Path) -> PathBuf {
    PathBuf::from(crate::pathutil::normalize_slash(&path.to_string_lossy()))
}
