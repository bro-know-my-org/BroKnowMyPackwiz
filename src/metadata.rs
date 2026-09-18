use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Side {
    Client,
    Server,
    Both,
    Unknown(String),
}

impl Side {
    pub fn parse(value: &str) -> Self {
        match value {
            "client" => Self::Client,
            "server" => Self::Server,
            "both" | "common" | "" => Self::Both,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
            Self::Both => "both",
            Self::Unknown(value) => value,
        }
    }

    pub fn installs_on(&self, target: &Side) -> bool {
        match (self, target) {
            (Self::Both, Self::Client | Self::Server | Self::Both) => true,
            (Self::Client, Self::Client | Self::Both) => true,
            (Self::Server, Self::Server | Self::Both) => true,
            (Self::Unknown(_), _) | (_, Self::Unknown(_)) => false,
            (Self::Client, Self::Server) | (Self::Server, Self::Client) => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModMetadata {
    pub name: Option<String>,
    pub filename: Option<String>,
    pub side: Option<Side>,
    pub preserve: bool,
    pub pin: bool,
    pub optional: bool,
    pub option_default: bool,
    pub download_url: Option<String>,
    pub download_mode: Option<String>,
    pub download_hash_format: Option<String>,
    pub download_hash: Option<String>,
    pub curseforge_project_id: Option<u64>,
    pub curseforge_file_id: Option<u64>,
    pub export_curseforge_project_id: Option<u64>,
    pub export_curseforge_file_id: Option<u64>,
    pub export_curseforge_latest: bool,
    pub github_project: Option<String>,
    pub github_tag: Option<String>,
    pub github_asset: Option<String>,
}

impl ModMetadata {
    pub fn updates_via_curseforge(&self) -> bool {
        self.download_mode.as_deref() == Some("metadata:curseforge")
            || (self.curseforge_project_id.is_some() && self.github_project.is_none())
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        Self::load_operation(path).map_err(|error| error.detail)
    }

    pub fn load_operation(path: &Path) -> crate::operation::Result<Self> {
        let text = fs::read_to_string(path).map_err(|err| {
            crate::operation::Error::named(
                crate::operation::ErrorCode::Failed,
                "read_file_failed",
                format!("failed to read {}: {err}", path.display()),
            )
            .context(format!("{}: {err}", path.display()))
        })?;
        Ok(Self::parse(&text))
    }

    pub fn parse(text: &str) -> Self {
        let mut name = None;
        let mut filename = None;
        let mut side = None;
        let mut preserve = false;
        let mut pin = false;
        let mut optional = false;
        let mut option_default = false;
        let mut download_url = None;
        let mut download_mode = None;
        let mut download_hash_format = None;
        let mut download_hash = None;
        let mut curseforge_project_id = None;
        let mut curseforge_file_id = None;
        let mut export_curseforge_project_id = None;
        let mut export_curseforge_file_id = None;
        let mut export_curseforge_latest = false;
        let mut github_project = None;
        let mut github_tag = None;
        let mut github_asset = None;
        let mut section = "";
        for raw in text.lines() {
            let line = strip_comment(raw).trim();
            if let Some(name) = line.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                section = name.trim();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match (section, key) {
                ("", "name") => name = parse_string(value),
                ("", "filename") => filename = parse_string(value),
                ("", "side") => side = parse_string(value).map(|value| Side::parse(&value)),
                ("", "preserve") => preserve = parse_bool(value).unwrap_or(false),
                ("", "pin") => pin = parse_bool(value).unwrap_or(false),
                ("download", "url") => download_url = parse_string(value),
                ("download", "mode") => download_mode = parse_string(value),
                ("download", "hash-format") => download_hash_format = parse_string(value),
                ("download", "hash") => download_hash = parse_string(value),
                ("update.curseforge", "project-id") => curseforge_project_id = parse_u64(value),
                ("update.curseforge", "file-id") => curseforge_file_id = parse_u64(value),
                ("export.curseforge", "project-id") => {
                    export_curseforge_project_id = parse_u64(value)
                }
                ("export.curseforge", "file-id") => export_curseforge_file_id = parse_u64(value),
                ("export.curseforge", "latest") => {
                    export_curseforge_latest = parse_bool(value).unwrap_or(false)
                }
                ("update.github", "project") => github_project = parse_string(value),
                ("update.github", "tag") => github_tag = parse_string(value),
                ("update.github", "asset") => github_asset = parse_string(value),
                ("option", "optional") => optional = parse_bool(value).unwrap_or(false),
                ("option", "default") => option_default = parse_bool(value).unwrap_or(false),
                _ => {}
            }
        }
        Self {
            name,
            filename,
            side,
            preserve,
            pin,
            optional,
            option_default,
            download_url,
            download_mode,
            download_hash_format,
            download_hash,
            curseforge_project_id,
            curseforge_file_id,
            export_curseforge_project_id,
            export_curseforge_file_id,
            export_curseforge_latest,
            github_project,
            github_tag,
            github_asset,
        }
    }
}

fn strip_comment(line: &str) -> &str {
    crate::pathutil::strip_comment(line)
}

fn parse_string(value: &str) -> Option<String> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.replace("\\\"", "\"").replace("\\\\", "\\"))
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_u64(value: &str) -> Option<u64> {
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_existing_side_without_inference() {
        let meta = ModMetadata::parse("name = \"Bad upstream side\"\nside = \"client\"\n");

        assert_eq!(meta.side, Some(Side::Client));
        assert_eq!(meta.name.as_deref(), Some("Bad upstream side"));
    }

    #[test]
    fn parses_download_hash_and_preserve() {
        let meta = ModMetadata::parse(
            "preserve = true\npin = true\n[download]\nhash-format = \"sha256\"\nhash = \"abc\"\n",
        );

        assert!(meta.preserve);
        assert!(meta.pin);
        assert_eq!(meta.download_url, None);
        assert_eq!(meta.download_mode, None);
        assert_eq!(meta.download_hash_format.as_deref(), Some("sha256"));
        assert_eq!(meta.download_hash.as_deref(), Some("abc"));
    }

    #[test]
    fn parses_optional_metadata() {
        let meta = ModMetadata::parse("[option]\noptional = true\ndefault = false\n");

        assert!(meta.optional);
        assert!(!meta.option_default);
    }

    #[test]
    fn parses_curseforge_metadata() {
        let meta = ModMetadata::parse(
            "name = \"Always Eat\"\n\
             filename = \"AlwaysEat.jar\"\n\
             [download]\n\
             hash-format = \"sha1\"\n\
             hash = \"abc\"\n\
             mode = \"metadata:curseforge\"\n\
             [update]\n\
             [update.curseforge]\n\
             file-id = 6498183\n\
             project-id = 1259229\n",
        );

        assert_eq!(meta.download_mode.as_deref(), Some("metadata:curseforge"));
        assert_eq!(meta.curseforge_file_id, Some(6498183));
        assert_eq!(meta.curseforge_project_id, Some(1259229));
    }

    #[test]
    fn parses_github_metadata() {
        let meta = ModMetadata::parse(
            "[update]\n\
             [update.github]\n\
             project = \"owner/repo\"\n\
             tag = \"latest\"\n\
             asset = \"neoforge\"\n",
        );

        assert_eq!(meta.github_project.as_deref(), Some("owner/repo"));
        assert_eq!(meta.github_tag.as_deref(), Some("latest"));
        assert_eq!(meta.github_asset.as_deref(), Some("neoforge"));
    }

    #[test]
    fn parses_export_curseforge_metadata() {
        let meta = ModMetadata::parse(
            "[download]\n\
             url = \"https://example.invalid/core.jar\"\n\
             [export.curseforge]\n\
             project-id = 123456\n\
             file-id = 789012\n\
             latest = true\n",
        );

        assert_eq!(meta.export_curseforge_project_id, Some(123456));
        assert_eq!(meta.export_curseforge_file_id, Some(789012));
        assert!(meta.export_curseforge_latest);
    }

    #[test]
    fn keeps_hash_inside_quoted_strings() {
        let meta = ModMetadata::parse(
            "name = \"A # B\"\n[download]\nurl = \"https://example.invalid/file.jar#sha256\"\n",
        );

        assert_eq!(meta.name.as_deref(), Some("A # B"));
        assert_eq!(
            meta.download_url.as_deref(),
            Some("https://example.invalid/file.jar#sha256")
        );
    }
}
