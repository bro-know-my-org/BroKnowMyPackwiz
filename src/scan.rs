use std::fs;
use std::path::Path;

use crate::config::ProjectConfig;
use crate::ignore::IgnoreSet;
use crate::layout::PackLayout;
use crate::pathutil::{join_slash, relative_slash};

#[derive(Debug, Clone)]
pub struct ScanReport {
    pub included: Vec<String>,
    pub excluded: Vec<ExcludedFile>,
    pub metadata: Vec<MetadataFile>,
    pub jars: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ExcludedFile {
    pub path: String,
    pub reason: ExcludeReason,
}

#[derive(Debug, Clone)]
pub enum ExcludeReason {
    Gitignore,
    Packwizignore,
}

impl ExcludeReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Gitignore => "gitignore",
            Self::Packwizignore => "packwizignore",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MetadataFile {
    pub path: String,
    pub side_hint: SideHint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideHint {
    Server,
    Client,
    Common,
    Unknown,
}

impl SideHint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Client => "client",
            Self::Common => "common",
            Self::Unknown => "unknown",
        }
    }
}

impl ScanReport {
    pub fn build(root: &Path, config: &ProjectConfig, layout: &PackLayout) -> Result<Self, String> {
        let gitignore = if config.scan.use_gitignore {
            GitignoreSet::load_root(root)?
        } else {
            GitignoreSet::empty()
        };
        let packwizignore = IgnoreSet::load_result(&join_slash(
            root,
            &config.scan.packwizignore.to_string_lossy(),
        ))?;

        let mut report = Self {
            included: Vec::new(),
            excluded: Vec::new(),
            metadata: Vec::new(),
            jars: Vec::new(),
        };

        walk(root, root, &gitignore, &packwizignore, layout, &mut report)?;
        report.included.sort();
        report.excluded.sort_by(|a, b| a.path.cmp(&b.path));
        report.metadata.sort_by(|a, b| a.path.cmp(&b.path));
        report.jars.sort();
        Ok(report)
    }
}

fn walk(
    root: &Path,
    current: &Path,
    gitignore: &GitignoreSet,
    packwizignore: &IgnoreSet,
    layout: &PackLayout,
    report: &mut ScanReport,
) -> Result<(), String> {
    let entries = fs::read_dir(current)
        .map_err(|err| format!("failed to read {}: {err}", current.display()))?;

    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to read file type for {}: {err}", path.display()))?;
        let is_dir = file_type.is_dir();
        let rel = relative_slash(root, &path)?;

        if rel == ".git" || rel == "target" {
            continue;
        }

        let packwiz_included = packwizignore.is_explicitly_included(&rel, is_dir);
        if gitignore.is_ignored(&rel, is_dir) && !packwiz_included {
            report.excluded.push(ExcludedFile {
                path: rel,
                reason: ExcludeReason::Gitignore,
            });
            continue;
        }

        if packwizignore.is_ignored(&rel, is_dir) {
            report.excluded.push(ExcludedFile {
                path: rel,
                reason: ExcludeReason::Packwizignore,
            });
            continue;
        }

        if file_type.is_symlink() {
            return Err(format!("refusing to scan symlink: {}", path.display()));
        }

        if is_dir {
            let child_gitignore = gitignore.with_directory(root, &path)?;
            walk(root, &path, &child_gitignore, packwizignore, layout, report)?;
            continue;
        }

        report.included.push(rel.clone());
        if is_metadata_file(&rel, layout) {
            report.metadata.push(MetadataFile {
                side_hint: side_hint(&rel, layout),
                path: rel,
            });
        } else if rel.ends_with(".jar") && is_under(&rel, &layout.jar_root.to_string_lossy()) {
            report.jars.push(rel);
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct GitignoreSet {
    scopes: Vec<GitignoreScope>,
}

#[derive(Debug, Clone)]
struct GitignoreScope {
    base: String,
    rules: IgnoreSet,
}

impl GitignoreSet {
    fn empty() -> Self {
        Self { scopes: Vec::new() }
    }

    fn load_root(root: &Path) -> Result<Self, String> {
        let rules = IgnoreSet::load_result(&root.join(".gitignore"))?;
        if rules.is_empty() {
            Ok(Self::empty())
        } else {
            Ok(Self {
                scopes: vec![GitignoreScope {
                    base: String::new(),
                    rules,
                }],
            })
        }
    }

    fn with_directory(&self, root: &Path, dir: &Path) -> Result<Self, String> {
        let rules = IgnoreSet::load_result(&dir.join(".gitignore"))?;
        if rules.is_empty() {
            return Ok(self.clone());
        }

        let mut scopes = self.scopes.clone();
        scopes.push(GitignoreScope {
            base: relative_slash(root, dir)?,
            rules,
        });
        Ok(Self { scopes })
    }

    fn is_ignored(&self, rel_path: &str, is_dir: bool) -> bool {
        let mut ignored = false;
        for scope in &self.scopes {
            let Some(scoped_path) = scope.strip_base(rel_path) else {
                continue;
            };
            if scope.rules.is_ignored(scoped_path, is_dir) {
                ignored = true;
            } else if scope.rules.is_explicitly_included(scoped_path, is_dir) {
                ignored = false;
            }
        }
        ignored
    }
}

impl GitignoreScope {
    fn strip_base<'a>(&self, rel_path: &'a str) -> Option<&'a str> {
        if self.base.is_empty() {
            return Some(rel_path);
        }
        rel_path.strip_prefix(&self.base)?.strip_prefix('/')
    }
}

pub fn is_metadata_file(rel: &str, layout: &PackLayout) -> bool {
    let extension = layout.metadata_extension.trim_start_matches('.');
    let configured_suffix = format!(".{extension}");
    let configured_toml_suffix = format!(".{extension}.toml");
    (rel.ends_with(".pw")
        || rel.ends_with(".pw.toml")
        || rel.ends_with(&configured_suffix)
        || rel.ends_with(&configured_toml_suffix))
        && layout
            .metadata_roots
            .iter()
            .any(|root| crate::pathutil::is_under_slash(rel, &root.to_string_lossy()))
}

fn side_hint(rel: &str, layout: &PackLayout) -> SideHint {
    if crate::pathutil::is_under_slash(rel, &layout.server_meta.to_string_lossy()) {
        SideHint::Server
    } else if crate::pathutil::is_under_slash(rel, &layout.client_meta.to_string_lossy()) {
        SideHint::Client
    } else if crate::pathutil::is_under_slash(rel, &layout.common_meta.to_string_lossy()) {
        SideHint::Common
    } else if is_direct_child(rel, &layout.metadata_root.to_string_lossy()) {
        SideHint::Common
    } else if layout.metadata_roots.iter().any(|root| {
        normalize_layout_path(root) != normalize_layout_path(&layout.metadata_root)
            && crate::pathutil::is_under_slash(rel, &root.to_string_lossy())
    }) {
        SideHint::Client
    } else {
        SideHint::Unknown
    }
}

fn normalize_layout_path(path: &Path) -> String {
    crate::pathutil::normalize_slash(&path.to_string_lossy())
}

fn is_under(rel: &str, parent: &str) -> bool {
    let parent = crate::pathutil::normalize_slash(parent);
    rel == parent
        || rel
            .strip_prefix(&parent)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn is_direct_child(rel: &str, parent: &str) -> bool {
    let Some(rest) = rel.strip_prefix(parent) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('/') else {
        return false;
    };
    !rest.contains('/')
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use crate::config::{CurseForgeConfig, InstallConfig, LayoutConfig, ProjectConfig, ScanConfig};

    use super::*;

    #[test]
    fn direct_mods_metadata_is_common_compat() {
        let config = ProjectConfig {
            source: PathBuf::from(".pw/config.toml"),
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
                metadata_extension: String::from("pw"),
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
        };
        let layout = PackLayout::from_config(&config);

        assert_eq!(side_hint("mods/foo.pw", &layout), SideHint::Common);
        assert_eq!(side_hint("mods/foo.pw.toml", &layout), SideHint::Common);
        assert_eq!(side_hint("resourcepacks/foo.pw", &layout), SideHint::Client);
        assert_eq!(side_hint("shaderpacks/foo.pw", &layout), SideHint::Client);
        assert!(is_metadata_file("mods/foo.pw.toml", &layout));
        assert!(is_metadata_file("resourcepacks/foo.pw.toml", &layout));

        let mut toml_layout = layout.clone();
        toml_layout.metadata_extension = "pw.toml".to_string();
        assert!(is_metadata_file("mods/foo.pw", &toml_layout));
        assert!(is_metadata_file("mods/foo.pw.toml", &toml_layout));
    }

    #[test]
    fn ignored_symlink_does_not_block_scan() {
        let root = temp_root("scan-ignored-symlink");
        fs::write(root.join(".gitignore"), "ignored.link\n").unwrap();
        fs::write(root.join(".packwizignore"), "").unwrap();
        let target = root.join("target.txt");
        fs::write(&target, b"target").unwrap();
        let link = root.join("ignored.link");
        if create_file_symlink(&target, &link).is_err() {
            let _ = fs::remove_dir_all(root);
            return;
        }
        let config = test_config();
        let layout = PackLayout::from_config(&config);

        let report = ScanReport::build(&root, &config, &layout).unwrap();

        assert!(
            report
                .excluded
                .iter()
                .any(|item| item.path == "ignored.link")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn packwiz_allowlist_overrides_gitignore() {
        let root = temp_root("scan-packwiz-allowlist-overrides-gitignore");
        fs::write(root.join(".gitignore"), "icon.png\nPCL/\nmods/*.jar\n").unwrap();
        fs::write(
            root.join(".packwizignore"),
            "/*\n!/icon.png\n!/PCL/\n!/PCL/**\n!/mods/\n!/mods/**\nmods/*.jar\n",
        )
        .unwrap();
        fs::write(root.join("icon.png"), b"icon").unwrap();
        fs::create_dir_all(root.join("PCL")).unwrap();
        fs::write(root.join("PCL").join("Logo.png"), b"logo").unwrap();
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods").join("manual.jar"), b"manual").unwrap();
        let config = test_config();
        let layout = PackLayout::from_config(&config);

        let report = ScanReport::build(&root, &config, &layout).unwrap();

        assert!(report.included.contains(&"icon.png".to_string()));
        assert!(report.included.contains(&"PCL/Logo.png".to_string()));
        assert!(!report.included.contains(&"mods/manual.jar".to_string()));
        assert!(report.excluded.iter().any(|item| {
            item.path == "mods/manual.jar" && matches!(item.reason, ExcludeReason::Packwizignore)
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nested_gitignore_applies_to_descendants() {
        let root = temp_root("scan-nested-gitignore");
        fs::write(root.join(".gitignore"), "/*\n!/config/\n!/.packwizignore\n").unwrap();
        fs::write(root.join(".packwizignore"), "").unwrap();
        fs::create_dir_all(root.join("config").join("spark")).unwrap();
        fs::write(root.join("config").join(".gitignore"), "/*\n!/.gitignore\n").unwrap();
        fs::write(
            root.join("config").join("spark").join("heap-test.hprof"),
            b"heap",
        )
        .unwrap();
        let config = test_config();
        let layout = PackLayout::from_config(&config);

        let report = ScanReport::build(&root, &config, &layout).unwrap();

        assert!(
            !report
                .included
                .contains(&"config/spark/heap-test.hprof".to_string())
        );
        assert!(report.excluded.iter().any(|item| {
            item.path == "config/spark" && matches!(item.reason, ExcludeReason::Gitignore)
        }));

        let _ = fs::remove_dir_all(root);
    }

    fn test_config() -> ProjectConfig {
        ProjectConfig {
            source: PathBuf::from(".pw/config.toml"),
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
                metadata_extension: String::from("pw"),
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
        }
    }

    #[cfg(unix)]
    fn create_file_symlink(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_file_symlink(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    fn temp_root(prefix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "{}-{}",
            prefix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
