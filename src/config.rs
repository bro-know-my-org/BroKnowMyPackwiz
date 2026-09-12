use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub source: PathBuf,
    pub scan: ScanConfig,
    pub layout: LayoutConfig,
    pub install: InstallConfig,
    pub curseforge: CurseForgeConfig,
    pub release: ReleaseConfig,
}

#[derive(Debug, Clone, Default)]
pub struct ReleaseConfig {
    pub enabled: bool,
    pub template_dir: Option<PathBuf>,
    pub template_files: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub use_gitignore: bool,
    pub packwizignore: PathBuf,
}

#[derive(Debug, Clone)]
pub struct LayoutConfig {
    pub metadata_root: PathBuf,
    pub metadata_roots: Vec<PathBuf>,
    pub jar_root: PathBuf,
    pub server_meta: PathBuf,
    pub client_meta: PathBuf,
    pub common_meta: PathBuf,
    pub root_overlays: PathBuf,
    pub metadata_extension: String,
}

#[derive(Debug, Clone)]
pub struct InstallConfig {
    pub jobs: usize,
    pub retries: usize,
    pub retry_delay_seconds: u64,
    pub force: bool,
    pub split_download_min_bytes: u64,
    pub split_download_chunks: usize,
}

#[derive(Debug, Clone)]
pub struct CurseForgeConfig {
    pub api_key: Option<String>,
    pub cdn_fallback: bool,
}

impl ProjectConfig {
    pub fn load(root: &Path) -> Result<Self, String> {
        let source = root.join(".pw").join("config.toml");
        let mut config = Self::default_at(source.clone());

        let text = match fs::read_to_string(&source) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(config),
            Err(err) => return Err(format!("failed to read {}: {err}", source.display())),
        };

        let mut section = "";
        for raw_line in text.lines() {
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                section = name.trim();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!(
                    "invalid config line in {}: {raw_line}",
                    source.display()
                ));
            };
            apply_value(&mut config, section, key.trim(), value.trim())?;
        }

        Ok(config)
    }

    fn default_at(source: PathBuf) -> Self {
        Self {
            source,
            scan: ScanConfig {
                use_gitignore: true,
                packwizignore: PathBuf::from(".packwizignore"),
            },
            layout: LayoutConfig {
                metadata_root: PathBuf::from("mods"),
                metadata_roots: vec![
                    PathBuf::from("mods"),
                    PathBuf::from("resourcepacks"),
                    PathBuf::from("shaderpacks"),
                ],
                jar_root: PathBuf::from("mods"),
                server_meta: PathBuf::from("mods/server"),
                client_meta: PathBuf::from("mods/client"),
                common_meta: PathBuf::from("mods/common"),
                root_overlays: PathBuf::from("roots"),
                metadata_extension: String::from("pw.toml"),
            },
            install: InstallConfig {
                jobs: 8,
                retries: 1,
                retry_delay_seconds: 5,
                force: false,
                split_download_min_bytes: 16 * 1024 * 1024,
                split_download_chunks: 4,
            },
            curseforge: CurseForgeConfig {
                api_key: None,
                cdn_fallback: true,
            },
            release: ReleaseConfig::default(),
        }
    }
}

fn apply_value(
    config: &mut ProjectConfig,
    section: &str,
    key: &str,
    value: &str,
) -> Result<(), String> {
    match (section, key) {
        ("release", "enabled") => config.release.enabled = parse_bool(value)?,
        ("release", "template-dir") => {
            config.release.template_dir = Some(parse_string_path(value)?)
        }
        ("release", "template-files") => config.release.template_files = parse_string_list(value)?,
        ("scan", "use-gitignore") => config.scan.use_gitignore = parse_bool(value)?,
        ("scan", "packwizignore") => config.scan.packwizignore = parse_string_path(value)?,
        ("layout", "metadata-root") => {
            config.layout.metadata_root = parse_string_path(value)?;
            config.layout.metadata_roots = vec![config.layout.metadata_root.clone()];
        }
        ("layout", "metadata-roots") => {
            config.layout.metadata_roots = parse_string_list(value)?;
            config.layout.metadata_root = config
                .layout
                .metadata_roots
                .first()
                .cloned()
                .ok_or_else(|| "expected at least one path".to_string())?;
        }
        ("layout", "jar-root") => config.layout.jar_root = parse_string_path(value)?,
        ("layout", "server-meta") => config.layout.server_meta = parse_string_path(value)?,
        ("layout", "client-meta") => config.layout.client_meta = parse_string_path(value)?,
        ("layout", "common-meta") => config.layout.common_meta = parse_string_path(value)?,
        ("layout", "root-overlays") => config.layout.root_overlays = parse_string_path(value)?,
        ("layout", "metadata-extension") => config.layout.metadata_extension = parse_string(value)?,
        ("install", "jobs") => config.install.jobs = parse_usize(value)?,
        ("install", "retries") => config.install.retries = parse_usize(value)?,
        ("install", "retry-delay-seconds") => {
            config.install.retry_delay_seconds = parse_u64(value)?
        }
        ("install", "force") => config.install.force = parse_bool(value)?,
        ("install", "split-download-min-bytes") => {
            config.install.split_download_min_bytes = parse_u64(value)?
        }
        ("install", "split-download-chunks") => {
            config.install.split_download_chunks = parse_usize(value)?
        }
        ("curseforge", "api-key") => config.curseforge.api_key = Some(parse_string(value)?),
        ("curseforge", "cdn-fallback") => config.curseforge.cdn_fallback = parse_bool(value)?,
        _ => {}
    }
    Ok(())
}

fn strip_comment(line: &str) -> &str {
    crate::pathutil::strip_comment(line)
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("expected boolean, got {value}")),
    }
}

fn parse_string_path(value: &str) -> Result<PathBuf, String> {
    parse_string(value).and_then(|value| {
        crate::pathutil::safe_slash_path(&value)?;
        Ok(PathBuf::from(value))
    })
}

fn parse_string(value: &str) -> Result<String, String> {
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return Err(format!("expected quoted string, got {value}"));
    };
    let mut chars = inner.chars();
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Ok(out)
}

fn parse_string_list(value: &str) -> Result<Vec<PathBuf>, String> {
    let text = parse_string(value)?;
    let roots = text
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            crate::pathutil::safe_slash_path(value)?;
            Ok(PathBuf::from(value))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if roots.is_empty() {
        return Err("expected at least one path".to_string());
    }
    Ok(roots)
}

fn parse_usize(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|err| format!("expected positive integer, got {value}: {err}"))
}

fn parse_u64(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|err| format!("expected positive integer, got {value}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_side_oriented() {
        let cfg = ProjectConfig::default_at(PathBuf::from(".pw/config.toml"));

        assert!(cfg.scan.use_gitignore);
        assert_eq!(
            cfg.layout.metadata_roots,
            vec![
                PathBuf::from("mods"),
                PathBuf::from("resourcepacks"),
                PathBuf::from("shaderpacks"),
            ]
        );
        assert_eq!(cfg.layout.server_meta, PathBuf::from("mods/server"));
        assert_eq!(cfg.layout.client_meta, PathBuf::from("mods/client"));
        assert_eq!(cfg.layout.common_meta, PathBuf::from("mods/common"));
        assert_eq!(cfg.layout.root_overlays, PathBuf::from("roots"));
        assert_eq!(cfg.layout.metadata_extension, "pw.toml");
        assert_eq!(cfg.install.jobs, 8);
        assert_eq!(cfg.install.retries, 1);
        assert_eq!(cfg.install.retry_delay_seconds, 5);
        assert!(!cfg.install.force);
        assert_eq!(cfg.install.split_download_min_bytes, 16 * 1024 * 1024);
        assert_eq!(cfg.install.split_download_chunks, 4);
    }

    #[test]
    fn parses_split_download_install_config() {
        let root = unique_test_dir("bkmpw-config-split-download");
        fs::create_dir_all(root.join(".pw")).unwrap();
        fs::write(
            root.join(".pw").join("config.toml"),
            "[install]\nsplit-download-min-bytes = 1024\nsplit-download-chunks = 3\n",
        )
        .unwrap();

        let config = ProjectConfig::load(&root).unwrap();

        assert_eq!(config.install.split_download_min_bytes, 1024);
        assert_eq!(config.install.split_download_chunks, 3);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_layout_paths_outside_project() {
        let root = unique_test_dir("bkmpw-config-paths");
        fs::create_dir_all(root.join(".pw")).unwrap();
        fs::write(
            root.join(".pw").join("config.toml"),
            "[layout]\nmetadata-root = \"../outside\"\n",
        )
        .unwrap();

        let result = ProjectConfig::load(&root);

        assert!(result.is_err());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn keeps_hash_inside_quoted_config_strings() {
        let root = unique_test_dir("bkmpw-config-hash");
        fs::create_dir_all(root.join(".pw")).unwrap();
        fs::write(
            root.join(".pw").join("config.toml"),
            "[curseforge]\napi-key = \"abc#def\"\n",
        )
        .unwrap();

        let config = ProjectConfig::load(&root).unwrap();

        assert_eq!(config.curseforge.api_key.as_deref(), Some("abc#def"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_escaped_backslash_before_quote() {
        assert_eq!(parse_string(r#""a\\\"b""#).unwrap(), "a\\\"b");
    }

    #[test]
    fn metadata_roots_updates_legacy_primary_root() {
        let root = unique_test_dir("bkmpw-config-roots");
        fs::create_dir_all(root.join(".pw")).unwrap();
        fs::write(
            root.join(".pw").join("config.toml"),
            "[layout]\nmetadata-roots = \"plugins,resourcepacks\"\n",
        )
        .unwrap();

        let config = ProjectConfig::load(&root).unwrap();

        assert_eq!(config.layout.metadata_root, PathBuf::from("plugins"));
        assert_eq!(
            config.layout.metadata_roots,
            vec![PathBuf::from("plugins"), PathBuf::from("resourcepacks")]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_errors_when_config_path_is_unreadable() {
        let root = unique_test_dir("bkmpw-config-unreadable");
        fs::create_dir_all(root.join(".pw").join("config.toml")).unwrap();

        let result = ProjectConfig::load(&root);

        assert!(result.is_err());

        let _ = fs::remove_dir_all(root);
    }

    fn unique_test_dir(prefix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "{}-{}",
            prefix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }
}
