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
        Self::parse(&text).map_err(|detail| {
            crate::operation::Error::named(
                crate::operation::ErrorCode::Failed,
                "metadata_parse_failed",
                format!("{}: {detail}", path.display()),
            )
            .context(path.display().to_string())
        })
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let doc = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|err| format!("invalid metadata TOML: {err}"))?;
        let get = |section: &str, key: &str| -> Option<&toml_edit::Item> {
            let mut item = doc.as_item();
            for part in section.split('.') {
                if part.is_empty() {
                    break;
                }
                item = item.get(part)?;
            }
            item.get(key)
        };
        let string = |section: &str, key: &str| {
            get(section, key).and_then(|v| v.as_str().map(str::to_string))
        };
        let boolv =
            |section: &str, key: &str| get(section, key).and_then(|v| v.as_bool()).unwrap_or(false);
        let int = |section: &str, key: &str| {
            get(section, key)
                .and_then(|v| v.as_integer())
                .and_then(|v| u64::try_from(v).ok())
        };
        Ok(Self {
            name: string("", "name"),
            filename: string("", "filename"),
            side: string("", "side").map(|value| Side::parse(&value)),
            preserve: boolv("", "preserve"),
            pin: boolv("", "pin"),
            optional: boolv("option", "optional"),
            option_default: boolv("option", "default"),
            download_url: string("download", "url"),
            download_mode: string("download", "mode"),
            download_hash_format: string("download", "hash-format"),
            download_hash: string("download", "hash"),
            curseforge_project_id: int("update.curseforge", "project-id"),
            curseforge_file_id: int("update.curseforge", "file-id"),
            export_curseforge_project_id: int("export.curseforge", "project-id"),
            export_curseforge_file_id: int("export.curseforge", "file-id"),
            export_curseforge_latest: boolv("export.curseforge", "latest"),
            github_project: string("update.github", "project"),
            github_tag: string("update.github", "tag"),
            github_asset: string("update.github", "asset"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_existing_side_without_inference() {
        let meta = ModMetadata::parse("name = \"Bad upstream side\"\nside = \"client\"\n").unwrap();

        assert_eq!(meta.side, Some(Side::Client));
        assert_eq!(meta.name.as_deref(), Some("Bad upstream side"));
    }

    #[test]
    fn parses_download_hash_and_preserve() {
        let meta = ModMetadata::parse(
            "preserve = true\npin = true\n[download]\nhash-format = \"sha256\"\nhash = \"abc\"\n",
        )
        .unwrap();

        assert!(meta.preserve);
        assert!(meta.pin);
        assert_eq!(meta.download_url, None);
        assert_eq!(meta.download_mode, None);
        assert_eq!(meta.download_hash_format.as_deref(), Some("sha256"));
        assert_eq!(meta.download_hash.as_deref(), Some("abc"));
    }

    #[test]
    fn parses_optional_metadata() {
        let meta = ModMetadata::parse("[option]\noptional = true\ndefault = false\n").unwrap();

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
        )
        .unwrap();

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
        )
        .unwrap();

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
        )
        .unwrap();

        assert_eq!(meta.export_curseforge_project_id, Some(123456));
        assert_eq!(meta.export_curseforge_file_id, Some(789012));
        assert!(meta.export_curseforge_latest);
    }

    #[test]
    fn keeps_hash_inside_quoted_strings() {
        let meta = ModMetadata::parse(
            "name = \"A # B\"\n[download]\nurl = \"https://example.invalid/file.jar#sha256\"\n",
        )
        .unwrap();

        assert_eq!(meta.name.as_deref(), Some("A # B"));
        assert_eq!(
            meta.download_url.as_deref(),
            Some("https://example.invalid/file.jar#sha256")
        );
    }
}
