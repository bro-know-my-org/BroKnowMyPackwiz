//! Localized operation adapters over the CLI's unchanged path validators.
use super::{Error, ErrorCode, Result};

pub fn filename(value: &str) -> Result<String> {
    crate::pathutil::safe_filename(value)
        .map_err(|legacy| Error::named(ErrorCode::Failed, "unsafe_filename", legacy).context(value))
}

pub fn relative(value: &str) -> Result<String> {
    crate::pathutil::safe_slash_path(value).map_err(|legacy| {
        Error::named(ErrorCode::Failed, "unsafe_relative_path", legacy).context(value)
    })
}

pub fn relativize(root: &std::path::Path, path: &std::path::Path) -> Result<String> {
    crate::pathutil::relative_slash(root, path).map_err(|legacy| {
        Error::named(ErrorCode::Failed, "path_outside_root", legacy).context(format!(
            "{}; {}",
            path.display(),
            root.display()
        ))
    })
}

pub fn github_project(value: &str) -> Result<String> {
    crate::github::normalize_project(value).map_err(|legacy| {
        Error::named(ErrorCode::Invalid, "invalid_github_project", legacy).context(value)
    })
}

pub fn metadata_slug(value: &str) -> Result<String> {
    crate::ops::safe_slug(value).map_err(|legacy| {
        Error::named(ErrorCode::Invalid, "invalid_metadata_name", legacy).context(value)
    })
}
