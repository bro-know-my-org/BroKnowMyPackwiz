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
        Ok(Self::parse(&text))
    }

    fn parse(text: &str) -> Self {
        let mut info = Self::default();
        let mut section = "";
        for raw in text.lines() {
            let line = crate::pathutil::strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
            {
                section = name.trim();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = parse_string(value.trim());
            match (section, key) {
                ("", "name") => info.name = value,
                ("", "author") => info.author = value,
                ("", "version") => info.version = value,
                ("versions", "minecraft") => info.minecraft = value,
                ("versions", "neoforge") => info.neoforge = value,
                ("versions", "forge") => info.forge = value,
                ("versions", "fabric") => info.fabric = value,
                ("versions", "quilt") => info.quilt = value,
                _ => {}
            }
        }
        info
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

fn parse_string(value: &str) -> Option<String> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.replace("\\\"", "\"").replace("\\\\", "\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fabric_and_quilt_loader_ids_preserve_existing_loader_priority() {
        for loader in ["fabric", "quilt"] {
            let info = PackInfo::parse(&format!("[versions]\n{loader} = \"0.1\""));
            assert_eq!(info.loader_name(), Some(loader));
            assert_eq!(info.primary_loader_id(), Some(format!("{loader}-0.1")));
        }
        let info = PackInfo::parse(
            "[versions]\nneoforge = \"21\"\nforge = \"52\"\nfabric = \"0.16\"\nquilt = \"0.27\"",
        );
        assert_eq!(info.loader_name(), Some("neoforge"));
        assert_eq!(info.primary_loader_id().as_deref(), Some("neoforge-21"));
    }

    #[test]
    fn parses_pack_versions() {
        let info = PackInfo::parse(
            "name = \"Pack\"\nauthor = \"Me\"\nversion = \"1\"\n[versions]\nminecraft = \"1.21.1\"\nneoforge = \"21.1.228\"\n",
        );

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
        let info = PackInfo::parse("name = \"A # Pack\"\n[versions]\nminecraft = \"1.21.1\"\n");

        assert_eq!(info.name.as_deref(), Some("A # Pack"));
    }
}
