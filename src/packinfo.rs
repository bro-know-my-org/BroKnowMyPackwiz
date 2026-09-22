use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct PackInfo {
    pub name: Option<String>,
    pub author: Option<String>,
    pub version: Option<String>,
    pub minecraft: Option<String>,
    pub neoforge: Option<String>,
    pub forge: Option<String>,
    pub fabric: Option<String>,
    pub quilt: Option<String>,
}

impl PackInfo {
    pub fn load(root: &Path) -> Result<Self, String> {
        Self::load_operation(root).map_err(|error| error.detail)
    }

    pub fn load_operation(root: &Path) -> crate::operation::Result<Self> {
        let path = root.join("pack.toml");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => {
                return Err(crate::operation::Error::named(
                    crate::operation::ErrorCode::Failed,
                    "pack_info_read_failed",
                    format!("failed to read {}: {err}", path.display()),
                )
                .context(format!("{}: {err}", path.display())));
            }
        };
        Self::parse(&text).map_err(|error| {
            crate::operation::Error::named(
                crate::operation::ErrorCode::Failed,
                "pack_info_parse_failed",
                format!("{}: {}", path.display(), error.detail),
            )
            .context(path.display().to_string())
        })
    }

    fn parse(text: &str) -> crate::operation::Result<Self> {
        use crate::operation::{Error, ErrorCode};
        let doc = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|err| Error::new(ErrorCode::Failed, format!("invalid pack.toml: {err}")))?;
        let string = |section: &str, key: &str| -> Option<String> {
            let parent = if section.is_empty() {
                Some(doc.as_item())
            } else {
                doc.get(section)
            };
            parent?.get(key)?.as_str().map(str::to_string)
        };
        Ok(Self {
            name: string("", "name"),
            author: string("", "author"),
            version: string("", "version"),
            minecraft: string("versions", "minecraft"),
            neoforge: string("versions", "neoforge"),
            forge: string("versions", "forge"),
            fabric: string("versions", "fabric"),
            quilt: string("versions", "quilt"),
        })
    }

    pub fn primary_loader_id(&self) -> Option<String> {
        self.neoforge
            .as_ref()
            .map(|version| format!("neoforge-{version}"))
            .or_else(|| {
                self.forge
                    .as_ref()
                    .map(|version| format!("forge-{version}"))
            })
            .or_else(|| {
                self.fabric
                    .as_ref()
                    .map(|version| format!("fabric-{version}"))
            })
            .or_else(|| {
                self.quilt
                    .as_ref()
                    .map(|version| format!("quilt-{version}"))
            })
    }

    pub fn loader_name(&self) -> Option<&'static str> {
        if self.neoforge.is_some() {
            Some("neoforge")
        } else if self.forge.is_some() {
            Some("forge")
        } else if self.fabric.is_some() {
            Some("fabric")
        } else if self.quilt.is_some() {
            Some("quilt")
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fabric_and_quilt_loader_ids_preserve_existing_loader_priority() {
        for loader in ["fabric", "quilt"] {
            let info = PackInfo::parse(&format!("[versions]\n{loader} = \"0.1\"")).unwrap();
            assert_eq!(info.loader_name(), Some(loader));
            assert_eq!(info.primary_loader_id(), Some(format!("{loader}-0.1")));
        }
        let info = PackInfo::parse(
            "[versions]\nneoforge = \"21\"\nforge = \"52\"\nfabric = \"0.16\"\nquilt = \"0.27\"",
        )
        .unwrap();
        assert_eq!(info.loader_name(), Some("neoforge"));
        assert_eq!(info.primary_loader_id().as_deref(), Some("neoforge-21"));
    }

    #[test]
    fn parses_pack_versions() {
        let info = PackInfo::parse(
            "name = \"Pack\"\nauthor = \"Me\"\nversion = \"1\"\n[versions]\nminecraft = \"1.21.1\"\nneoforge = \"21.1.228\"\n",
        ).unwrap();

        assert_eq!(info.name.as_deref(), Some("Pack"));
        assert_eq!(info.minecraft.as_deref(), Some("1.21.1"));
        assert_eq!(
            info.primary_loader_id().as_deref(),
            Some("neoforge-21.1.228")
        );
        assert_eq!(info.loader_name(), Some("neoforge"));
    }

    #[test]
    fn keeps_hash_inside_pack_strings() {
        let info =
            PackInfo::parse("name = \"A # Pack\"\n[versions]\nminecraft = \"1.21.1\"\n").unwrap();

        assert_eq!(info.name.as_deref(), Some("A # Pack"));
    }
}
