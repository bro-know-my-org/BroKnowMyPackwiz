use crate::operation::Result;
use crate::{config::ProjectConfig, layout::PackLayout, metadata::ModMetadata, scan::ScanReport};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub source: String,
    pub side: String,
    pub present: bool,
    pub metadata: ModMetadata,
}

pub fn load(root: &Path) -> Result<Vec<Entry>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let config = ProjectConfig::load_operation(root)?;
    let layout = PackLayout::from_config(&config);
    let report = ScanReport::build_operation(root, &config, &layout)?;
    report
        .metadata
        .into_iter()
        .map(|file| {
            let metadata = ModMetadata::load_operation(&root.join(&file.path))?;
            let present = metadata
                .filename
                .as_ref()
                .map(|name| {
                    crate::install::resolve_pack_file_path_operation(&file.path, name, &layout)
                        .map(|path| root.join(path).is_file())
                })
                .transpose()?
                .unwrap_or(false);
            let source = if metadata.curseforge_project_id.is_some() {
                "CurseForge"
            } else if metadata.github_project.is_some() {
                "GitHub"
            } else if metadata.download_url.is_some() {
                "URL"
            } else {
                "Local"
            }
            .to_string();
            Ok(Entry {
                name: metadata.name.clone().unwrap_or_else(|| file.path.clone()),
                side: crate::install::side_from_directory_or_metadata(file.side_hint, &metadata)
                    .as_str()
                    .to_string(),
                path: file.path,
                source,
                present,
                metadata,
            })
        })
        .collect()
}
