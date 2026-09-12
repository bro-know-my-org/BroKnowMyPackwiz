use std::fs;
use std::path::Path;

use crate::config::ProjectConfig;
use crate::curseforge;
use crate::install;
use crate::layout::PackLayout;
use crate::metadata::{ModMetadata, Side};
use crate::packinfo::PackInfo;
use crate::pathutil::join_slash;
use crate::scan::{ScanReport, is_metadata_file};
use crate::zipstore::ZipStore;

#[derive(Debug, Clone)]
struct CfManifestFile {
    project_id: u64,
    file_id: u64,
}

pub fn export_curseforge(root: &Path, output: &Path, target_side: &Side) -> Result<usize, String> {
    reject_unknown_side(target_side)?;
    let config = ProjectConfig::load(root)?;
    let layout = PackLayout::from_config(&config);
    let report = ScanReport::build(root, &config, &layout)?;
    let pack = PackInfo::load(root)?;
    let mut cf_files = Vec::new();
    let mut cf_override_targets = std::collections::BTreeSet::new();
    let mut managed_runtime_jars = std::collections::BTreeSet::new();
    let mut overrides = std::collections::BTreeSet::new();
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let output_rel = output_rel_in_root(root, output);
    let output_abs = output.canonicalize().ok();

    for entry in &report.metadata {
        let metadata = ModMetadata::load(&join_slash(root, &entry.path))?;
        let declared_side = side_from_directory_or_metadata(entry.side_hint, &metadata);
        let target = metadata
            .filename
            .as_deref()
            .map(|filename| install::resolve_pack_file_path(&entry.path, filename, &layout))
            .transpose()?;
        if metadata.optional && !metadata.option_default {
            if let Some(target) = target {
                cf_override_targets.insert(target);
            }
            continue;
        }
        if !declared_side.installs_on(target_side) {
            if let Some(target) = target {
                cf_override_targets.insert(target);
            }
            continue;
        }

        if let Some((project_id, file_id)) =
            export_curseforge_ids(&metadata, &config, &pack, &entry.path)?
        {
            cf_files.push(CfManifestFile {
                project_id,
                file_id,
            });
            if let Some(target) = target {
                cf_override_targets.insert(target);
            }
        } else if let Some(target) = target {
            managed_runtime_jars.insert(target);
        }
    }

    for rel in &report.included {
        if skip_override(rel, &layout) || output_rel.as_deref() == Some(rel.as_str()) {
            continue;
        }
        if is_runtime_jar(rel, &layout) {
            if !managed_runtime_jars.contains(rel) {
                continue;
            }
        } else if cf_override_targets.contains(rel) {
            continue;
        }
        if !override_side(rel, &layout).installs_on(target_side) {
            continue;
        }
        overrides.insert(rel.clone());
    }
    // Managed non-CF files must be bundled even when runtime files are ignored.
    for target in &managed_runtime_jars {
        reject_symlink(root, target)?;
        let target_path = join_slash(root, target);
        if output_rel
            .as_ref()
            .is_some_and(|out| out.eq_ignore_ascii_case(target))
            || output_abs
                .as_ref()
                .is_some_and(|out| target_path.canonicalize().ok().as_ref() == Some(out))
        {
            return Err(format!(
                "output path collides with required override file: {target}"
            ));
        }
        if !target_path.is_file() {
            return Err(format!("missing required override file: {target}"));
        }
        overrides.insert(target.clone());
    }
    cf_files.sort_by(|a, b| {
        a.project_id
            .cmp(&b.project_id)
            .then(a.file_id.cmp(&b.file_id))
    });

    let temp_output = crate::export_server::temp_zip_path(output);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_output)
        .map_err(|err| format!("failed to create {}: {err}", temp_output.display()))?;
    let result = (|| {
        let mut zip = ZipStore::new(file);
        zip.add_bytes("manifest.json", manifest_json(&pack, &cf_files).as_bytes())?;
        for rel in &overrides {
            reject_symlink(root, rel)?;
            zip.add_file(&format!("overrides/{rel}"), &join_slash(root, rel))?;
        }
        zip.finish()?;
        crate::export_server::replace_output(&temp_output, output)?;
        Ok(cf_files.len())
    })();
    crate::export_server::cleanup_temp_on_error(result, &temp_output)
}

fn reject_unknown_side(side: &Side) -> Result<(), String> {
    match side {
        Side::Unknown(value) => Err(format!("unsupported side: {value}")),
        Side::Client | Side::Server | Side::Both => Ok(()),
    }
}

fn output_rel_in_root(root: &Path, output: &Path) -> Option<String> {
    let root_abs = root.canonicalize().ok()?;
    let output_abs = if let Ok(path) = output.canonicalize() {
        path
    } else {
        let cwd = std::env::current_dir().ok()?;
        let output = if output.is_absolute() {
            output.to_path_buf()
        } else {
            cwd.join(output)
        };
        let parent = output.parent()?.canonicalize().ok()?;
        parent.join(output.file_name()?)
    };
    output_abs
        .strip_prefix(root_abs)
        .ok()
        .map(crate::pathutil::to_slash)
}

fn override_side(rel: &str, layout: &PackLayout) -> Side {
    if crate::pathutil::is_under_slash(rel, &layout.server_meta.to_string_lossy()) {
        Side::Server
    } else if crate::pathutil::is_under_slash(rel, &layout.client_meta.to_string_lossy())
        || crate::pathutil::is_under_slash(rel, "resourcepacks")
        || crate::pathutil::is_under_slash(rel, "shaderpacks")
    {
        Side::Client
    } else {
        Side::Both
    }
}

fn reject_symlink(root: &Path, rel: &str) -> Result<(), String> {
    crate::export_server::reject_symlink(root, rel)
}

fn side_from_directory_or_metadata(hint: crate::scan::SideHint, metadata: &ModMetadata) -> Side {
    match hint {
        crate::scan::SideHint::Server => Side::Server,
        crate::scan::SideHint::Client => Side::Client,
        crate::scan::SideHint::Common => Side::Both,
        crate::scan::SideHint::Unknown => metadata.side.clone().unwrap_or(Side::Both),
    }
}

fn export_curseforge_ids(
    metadata: &ModMetadata,
    config: &ProjectConfig,
    pack: &PackInfo,
    rel: &str,
) -> Result<Option<(u64, u64)>, String> {
    if metadata.export_curseforge_latest {
        let project_id = export_curseforge_latest_project_id(metadata, rel)?;
        let api_key = curseforge::api_key(config)
            .ok_or_else(|| format!("{rel}: export.curseforge.latest needs CURSEFORGE_API_KEY"))?;
        let file_id = curseforge::latest_file_id(
            &api_key,
            project_id,
            pack.minecraft.as_deref(),
            pack.loader_name(),
        )?;
        return Ok(Some((project_id, file_id)));
    }

    if let (Some(project_id), Some(file_id)) = (
        metadata.export_curseforge_project_id,
        metadata.export_curseforge_file_id,
    ) {
        return Ok(Some((project_id, file_id)));
    }

    if metadata.download_mode.as_deref() == Some("metadata:curseforge")
        && let (Some(project_id), Some(file_id)) =
            (metadata.curseforge_project_id, metadata.curseforge_file_id)
    {
        return Ok(Some((project_id, file_id)));
    }

    Ok(None)
}

fn export_curseforge_latest_project_id(metadata: &ModMetadata, rel: &str) -> Result<u64, String> {
    metadata
        .export_curseforge_project_id
        .or(metadata.curseforge_project_id)
        .ok_or_else(|| format!("{rel}: export.curseforge.latest needs project-id"))
}

fn skip_override(rel: &str, layout: &PackLayout) -> bool {
    rel == "pack.toml"
        || rel == "index.toml"
        || rel == ".pw/config.toml"
        || is_metadata_file(rel, layout)
}

fn is_runtime_jar(rel: &str, layout: &PackLayout) -> bool {
    rel.ends_with(".jar")
        && crate::pathutil::is_under_slash(rel, &layout.jar_root.to_string_lossy())
}

fn manifest_json(pack: &PackInfo, files: &[CfManifestFile]) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"minecraft\": {\n");
    out.push_str("    \"version\": \"");
    out.push_str(&json_escape(pack.minecraft.as_deref().unwrap_or("")));
    out.push_str("\",\n");
    out.push_str("    \"modLoaders\": [");
    if let Some(loader) = pack.primary_loader_id() {
        out.push_str("{\"id\":\"");
        out.push_str(&json_escape(&loader));
        out.push_str("\",\"primary\":true}");
    }
    out.push_str("]\n");
    out.push_str("  },\n");
    out.push_str("  \"manifestType\": \"minecraftModpack\",\n");
    out.push_str("  \"manifestVersion\": 1,\n");
    out.push_str("  \"name\": \"");
    out.push_str(&json_escape(
        pack.name.as_deref().unwrap_or("Minecraft Modpack"),
    ));
    out.push_str("\",\n");
    out.push_str("  \"version\": \"");
    out.push_str(&json_escape(pack.version.as_deref().unwrap_or("")));
    out.push_str("\",\n");
    out.push_str("  \"author\": \"");
    out.push_str(&json_escape(pack.author.as_deref().unwrap_or("")));
    out.push_str("\",\n");
    out.push_str("  \"files\": [\n");
    for (idx, file) in files.iter().enumerate() {
        let comma = if idx + 1 == files.len() { "" } else { "," };
        out.push_str("    {\"projectID\":");
        out.push_str(&file.project_id.to_string());
        out.push_str(",\"fileID\":");
        out.push_str(&file.file_id.to_string());
        out.push_str(",\"required\":true}");
        out.push_str(comma);
        out.push('\n');
    }
    out.push_str("  ],\n");
    out.push_str("  \"overrides\": \"overrides\"\n");
    out.push_str("}\n");
    out
}

fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            ch if ch <= '\u{001f}' => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::scan::SideHint;

    use super::*;

    #[test]
    fn export_curseforge_ids_prefers_export_section() {
        let metadata = ModMetadata::parse(
            "[download]\n\
             mode = \"metadata:curseforge\"\n\
             [update.curseforge]\n\
             project-id = 1\n\
             file-id = 2\n\
             [export.curseforge]\n\
             project-id = 3\n\
             file-id = 4\n",
        );

        let config = ProjectConfig::load(Path::new(".")).unwrap();
        let pack = PackInfo::default();

        assert_eq!(
            export_curseforge_ids(&metadata, &config, &pack, "mods/core.pw").unwrap(),
            Some((3, 4))
        );
    }

    #[test]
    fn export_latest_can_reuse_update_curseforge_project_id() {
        let metadata = ModMetadata::parse(
            "[download]\n\
             mode = \"metadata:curseforge\"\n\
             [update.curseforge]\n\
             project-id = 123456\n\
             file-id = 1\n\
             [export.curseforge]\n\
             latest = true\n",
        );
        assert_eq!(
            export_curseforge_latest_project_id(&metadata, "mods/core.pw").unwrap(),
            123456
        );
    }

    #[test]
    fn directory_side_overrides_metadata_side() {
        let metadata = ModMetadata::parse("side = \"server\"\n");

        assert_eq!(
            side_from_directory_or_metadata(SideHint::Client, &metadata),
            Side::Client
        );
    }

    #[test]
    fn override_side_uses_directory_placement() {
        let layout = PackLayout {
            metadata_root: PathBuf::from("mods"),
            metadata_roots: vec![PathBuf::from("mods")],
            jar_root: PathBuf::from("mods"),
            server_meta: PathBuf::from("mods/server"),
            client_meta: PathBuf::from("mods/client"),
            common_meta: PathBuf::from("mods/common"),
            root_overlays: PathBuf::from("roots"),
            metadata_extension: "pw".to_string(),
        };

        assert_eq!(
            override_side("mods/server/config.toml", &layout),
            Side::Server
        );
        assert_eq!(
            override_side("mods/client/config.toml", &layout),
            Side::Client
        );
        assert_eq!(
            override_side("resourcepacks/options.txt", &layout),
            Side::Client
        );
        assert_eq!(override_side("options.txt", &layout), Side::Both);
    }

    #[test]
    fn export_mapping_excludes_runtime_jar_from_overrides() {
        let root = unique_test_dir("bkmpw-export-cf");
        create_pack(&root);
        fs::create_dir_all(root.join("mods/common")).unwrap();
        fs::write(root.join("mods/core.jar"), b"jar").unwrap();
        fs::write(
            root.join("mods/common/core.pw"),
            "name = \"Core\"\n\
             filename = \"core.jar\"\n\
             [download]\n\
             url = \"https://github.com/owner/repo/releases/download/v1/core.jar\"\n\
             hash-format = \"sha256\"\n\
             hash = \"abc\"\n\
             [export.curseforge]\n\
             project-id = 123456\n\
             file-id = 789012\n",
        )
        .unwrap();

        let output = root.join("out.zip");
        let count = export_curseforge(&root, &output, &Side::Both).unwrap();
        let bytes = fs::read(&output).unwrap();
        let zip_text = String::from_utf8_lossy(&bytes);

        assert_eq!(count, 1);
        assert!(zip_text.contains("\"projectID\":123456"));
        assert!(!zip_text.contains("overrides/mods/core.jar"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_server_excludes_client_metadata_runtime_jar_from_overrides() {
        let root = unique_test_dir("bkmpw-export-server-excludes-client-jar");
        create_pack(&root);
        fs::create_dir_all(root.join("mods/client")).unwrap();
        fs::write(
            root.join("mods/client/client-only.pw.toml"),
            "filename = \"client-only.jar\"\n",
        )
        .unwrap();
        fs::write(root.join("mods/client-only.jar"), b"jar").unwrap();

        let output = root.join("server.zip");
        export_curseforge(&root, &output, &Side::Server).unwrap();
        let bytes = fs::read(&output).unwrap();
        let zip_text = String::from_utf8_lossy(&bytes);

        assert!(!zip_text.contains("overrides/mods/client-only.jar"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_excludes_unmanaged_runtime_jar_from_overrides() {
        let root = unique_test_dir("bkmpw-export-excludes-unmanaged-jar");
        create_pack(&root);
        fs::write(root.join("mods").join("private-dev.jar"), b"jar").unwrap();
        fs::write(root.join("options.txt"), b"options").unwrap();

        let output = root.join("out.zip");
        export_curseforge(&root, &output, &Side::Both).unwrap();
        let bytes = fs::read(&output).unwrap();
        let zip_text = String::from_utf8_lossy(&bytes);

        assert!(zip_text.contains("overrides/options.txt"));
        assert!(!zip_text.contains("overrides/mods/private-dev.jar"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_keeps_metadata_runtime_jar_without_curseforge_mapping_as_override() {
        let root = unique_test_dir("bkmpw-export-keeps-local-jar");
        create_pack(&root);
        fs::write(root.join(".packwizignore"), "mods/*.jar\n").unwrap();
        fs::create_dir_all(root.join("mods/common")).unwrap();
        fs::write(root.join("mods").join("local.jar"), b"jar").unwrap();
        fs::write(
            root.join("mods/common/local.pw.toml"),
            "filename = \"local.jar\"\n\
             [download]\n\
             url = \"https://example.invalid/local.jar\"\n\
             hash-format = \"sha256\"\n\
             hash = \"abc\"\n",
        )
        .unwrap();

        let output = root.join("out.zip");
        export_curseforge(&root, &output, &Side::Both).unwrap();
        let bytes = fs::read(&output).unwrap();
        let zip_text = String::from_utf8_lossy(&bytes);

        assert!(zip_text.contains("overrides/mods/local.jar"));

        let result = export_curseforge(&root, &root.join("mods/local.jar"), &Side::Both);
        assert!(result.unwrap_err().contains("output path collides"));
        assert_eq!(fs::read(root.join("mods/local.jar")).unwrap(), b"jar");

        let result = export_curseforge(&root, &root.join("mods/LOCAL.jar"), &Side::Both);
        assert!(result.unwrap_err().contains("output path collides"));
        assert_eq!(fs::read(root.join("mods/local.jar")).unwrap(), b"jar");

        fs::remove_file(&output).unwrap();
        fs::hard_link(root.join("mods/local.jar"), &output).unwrap();
        export_curseforge(&root, &output, &Side::Both).unwrap();
        assert_eq!(fs::read(root.join("mods/local.jar")).unwrap(), b"jar");
        assert!(
            String::from_utf8_lossy(&fs::read(&output).unwrap())
                .contains("overrides/mods/local.jar")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn ignored_managed_override_rejects_symlink_parent_before_replacing_output() {
        let root = unique_test_dir("bkmpw-export-cf-symlink-parent");
        create_pack(&root);
        fs::create_dir_all(root.join("private")).unwrap();
        fs::write(root.join("private/local.jar"), "private").unwrap();
        std::os::unix::fs::symlink(root.join("private"), root.join("mods/link")).unwrap();
        fs::write(root.join(".packwizignore"), "mods/link\nprivate\n").unwrap();
        fs::write(
            root.join("mods/local.pw.toml"),
            "filename = \"mods/link/local.jar\"\n",
        )
        .unwrap();
        let output = root.join("out.zip");
        fs::write(&output, "previous-artifact").unwrap();
        let result = export_curseforge(&root, &output, &Side::Both);
        assert!(result.unwrap_err().contains("symlink"));
        assert_eq!(fs::read_to_string(output).unwrap(), "previous-artifact");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn managed_runtime_jar_override_survives_deselected_optional_duplicate_target() {
        let root = unique_test_dir("bkmpw-export-keeps-duplicate-local-jar");
        create_pack(&root);
        fs::create_dir_all(root.join("mods/common")).unwrap();
        fs::write(root.join("mods").join("local.jar"), b"jar").unwrap();
        fs::write(
            root.join("mods/common/local.pw.toml"),
            "filename = \"local.jar\"\n\
             [download]\n\
             url = \"https://example.invalid/local.jar\"\n\
             hash-format = \"sha256\"\n\
             hash = \"abc\"\n",
        )
        .unwrap();
        fs::write(
            root.join("mods/common/optional-local.pw.toml"),
            "filename = \"local.jar\"\n\
             [option]\n\
             optional = true\n\
             default = false\n",
        )
        .unwrap();

        let output = root.join("out.zip");
        export_curseforge(&root, &output, &Side::Both).unwrap();
        let bytes = fs::read(&output).unwrap();
        let zip_text = String::from_utf8_lossy(&bytes);

        assert!(zip_text.contains("overrides/mods/local.jar"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_excludes_existing_output_zip_from_overrides() {
        let root = unique_test_dir("bkmpw-export-excludes-output");
        create_pack(&root);
        fs::write(root.join("options.txt"), b"options").unwrap();
        let output = root.join("curseforge-export.zip");
        fs::write(&output, b"old zip").unwrap();

        export_curseforge(&root, &output, &Side::Both).unwrap();
        let bytes = fs::read(&output).unwrap();
        let zip_text = String::from_utf8_lossy(&bytes);

        assert!(zip_text.contains("overrides/options.txt"));
        assert!(!zip_text.contains("overrides/curseforge-export.zip"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn output_rel_normalizes_existing_output_path() {
        let root = unique_test_dir("bkmpw-export-output-rel");
        fs::create_dir_all(root.join("sub")).unwrap();
        let output = root.join("curseforge-export.zip");
        fs::write(&output, b"old").unwrap();

        let rel = output_rel_in_root(
            &root,
            &root.join("sub").join("..").join("curseforge-export.zip"),
        );

        assert_eq!(rel.as_deref(), Some("curseforge-export.zip"));

        let _ = fs::remove_dir_all(root);
    }

    fn create_pack(root: &Path) {
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(
            root.join("pack.toml"),
            "name = \"Test Pack\"\n\
             author = \"Tester\"\n\
             version = \"1.0.0\"\n\
             pack-format = \"packwiz:1.1.0\"\n\
             [versions]\n\
             minecraft = \"1.21.1\"\n\
             neoforge = \"21.1.0\"\n",
        )
        .unwrap();
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
