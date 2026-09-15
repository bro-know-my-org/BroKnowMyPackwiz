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
