use std::fs;
use std::path::Path;

use crate::config::ProjectConfig;
use crate::curseforge;
use crate::github;
use crate::install;
use crate::layout::PackLayout;
use crate::metadata::ModMetadata;
use crate::ops;
use crate::packinfo::PackInfo;
use crate::pathutil::join_slash;
use crate::refresh;
use crate::scan::ScanReport;

#[derive(Debug, Clone, Default)]
pub struct UpdateOptions {
    pub minecraft_version: Option<String>,
    pub loader: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateResult {
    pub updated: Vec<String>,
    pub unchanged: Vec<String>,
    pub skipped: Vec<String>,
}

pub fn update_all(root: &Path, options: UpdateOptions) -> Result<UpdateResult, String> {
    let config = ProjectConfig::load(root)?;
    let layout = PackLayout::from_config(&config);
    let report = ScanReport::build(root, &config, &layout)?;
    let mut result = UpdateResult::default();
    for entry in report.metadata {
        let path = join_slash(root, &entry.path);
        update_metadata_path(root, &config, &layout, &path, &options, &mut result)?;
    }
    refresh::refresh(root, &config, &layout)?;
    Ok(result)
}

pub fn update_one(root: &Path, name: &str, options: UpdateOptions) -> Result<UpdateResult, String> {
    let config = ProjectConfig::load(root)?;
    let layout = PackLayout::from_config(&config);
    let path = ops::find_metadata(root, &config, &layout, name)?;
    let mut result = UpdateResult::default();
    update_metadata_path(root, &config, &layout, &path, &options, &mut result)?;
    refresh::refresh(root, &config, &layout)?;
    Ok(result)
}

fn update_metadata_path(
    root: &Path,
    config: &ProjectConfig,
    layout: &PackLayout,
    path: &Path,
    options: &UpdateOptions,
    result: &mut UpdateResult,
) -> Result<(), String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let metadata = ModMetadata::parse(&text);
    let rel = display_path(root, path);
    if metadata.pin {
        result.skipped.push(format!("{rel}: pinned"));
        return Ok(());
    }
    if metadata.download_mode.as_deref() == Some("metadata:curseforge") {
        update_curseforge(
            root, layout, path, &text, &metadata, config, options, result, rel,
        )
    } else if metadata.github_project.is_some() {
        update_github(root, layout, path, &text, &metadata, result, rel)
    } else {
        result.skipped.push(format!("{rel}: no update provider"));
        Ok(())
    }
}

fn update_curseforge(
    root: &Path,
    layout: &PackLayout,
    path: &Path,
    text: &str,
    metadata: &ModMetadata,
    config: &ProjectConfig,
    options: &UpdateOptions,
    result: &mut UpdateResult,
    rel: String,
) -> Result<(), String> {
    let project_id = metadata
        .curseforge_project_id
        .ok_or_else(|| format!("{rel}: missing update.curseforge.project-id"))?;
    let api_key = curseforge::api_key(config)
        .ok_or_else(|| format!("{rel}: CurseForge update needs CURSEFORGE_API_KEY"))?;
    let pack = PackInfo::load(root)?;
    let latest_file_id = curseforge::latest_file_id(
        &api_key,
        project_id,
        options
            .minecraft_version
            .as_deref()
            .or(pack.minecraft.as_deref()),
        options.loader.as_deref().or(pack.loader_name()),
    )?;
    let latest = curseforge::get_file_info(&api_key, project_id, latest_file_id)?;
    let latest_filename = crate::pathutil::safe_filename(&latest.filename)?;
    reject_manual_target_collision(
        root,
        layout,
        path,
        metadata,
        &latest_filename,
        &latest.hash,
        &latest.hash_format,
    )?;
    if metadata.curseforge_file_id == Some(latest.file_id)
        && metadata.filename.as_deref() == Some(latest_filename.as_str())
        && metadata.download_hash.as_deref() == Some(latest.hash.as_str())
    {
        result.unchanged.push(rel);
        return Ok(());
    }
    let updated = set_top_level_string(text, "filename", &latest_filename);
    let updated = set_section_string(&updated, "download", "hash-format", &latest.hash_format);
    let updated = set_section_string(&updated, "download", "hash", &latest.hash);
    let updated = set_section_string(&updated, "download", "mode", "metadata:curseforge");
    let updated = set_section_u64(&updated, "update.curseforge", "project-id", project_id);
    let updated = set_section_u64(&updated, "update.curseforge", "file-id", latest.file_id);
    crate::pathutil::write_atomic(path, updated)?;
    result.updated.push(format!(
        "{rel}: CurseForge {} -> {}",
        metadata
            .curseforge_file_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_string()),
        latest.file_id
    ));
    Ok(())
}

fn update_github(
    root: &Path,
    layout: &PackLayout,
    path: &Path,
    text: &str,
    metadata: &ModMetadata,
    result: &mut UpdateResult,
    rel: String,
) -> Result<(), String> {
    let project = metadata.github_project.as_deref().expect("checked above");
    let latest = github::resolve_github_release_asset(
        project,
        metadata.github_tag.as_deref(),
        metadata.github_asset.as_deref(),
        None,
        None,
    )?;
    let latest_filename = crate::pathutil::safe_filename(&latest.filename)?;
    reject_manual_target_collision(
        root,
        layout,
        path,
        metadata,
        &latest_filename,
        &latest.hash,
        &latest.hash_format,
    )?;
    if metadata.filename.as_deref() == Some(latest_filename.as_str())
        && metadata.download_url.as_deref() == Some(latest.url.as_str())
        && metadata.download_hash.as_deref() == Some(latest.hash.as_str())
    {
        result.unchanged.push(rel);
        return Ok(());
    }
    let updated = set_top_level_string(text, "filename", &latest_filename);
    let updated = set_section_string(&updated, "download", "url", &latest.url);
    let updated = set_section_string(&updated, "download", "hash-format", &latest.hash_format);
    let updated = set_section_string(&updated, "download", "hash", &latest.hash);
    let updated = set_section_string(&updated, "update.github", "project", project);
    let updated = match metadata.github_tag.as_deref() {
        Some(tag) => set_section_string(&updated, "update.github", "tag", tag),
        None => updated,
    };
    let updated = match metadata.github_asset.as_deref() {
        Some(asset) => set_section_string(&updated, "update.github", "asset", asset),
        None => updated,
    };
    crate::pathutil::write_atomic(path, updated)?;
    result
        .updated
        .push(format!("{rel}: GitHub {latest_filename}"));
    Ok(())
}

fn reject_manual_target_collision(
    root: &Path,
    layout: &PackLayout,
    metadata_path: &Path,
    metadata: &ModMetadata,
    new_filename: &str,
    new_hash: &str,
    hash_format: &str,
) -> Result<(), String> {
    let rel = display_path(root, metadata_path);
    let old_target = metadata
        .filename
        .as_deref()
        .map(|filename| install::resolve_pack_file_path(&rel, filename, layout))
        .transpose()?;
    let new_target = install::resolve_pack_file_path(&rel, new_filename, layout)?;
    if old_target.as_deref() == Some(new_target.as_str()) {
        return Ok(());
    }
    let target = join_slash(root, &new_target);
    if !target.exists() {
        return Ok(());
    }
    let matches = match hash_format {
        "sha1" => crate::sha1::sha1_file_hex(&target),
        "sha256" => crate::sha256::sha256_file_hex(&target),
        "sha512" => crate::sha512::sha512_file_hex(&target),
        other => return Err(format!("{rel}: unsupported update hash format: {other}")),
    };
    match matches {
        Ok(actual) if actual.eq_ignore_ascii_case(new_hash) => Ok(()),
        _ => Err(format!(
            "{rel}: update target collides with existing manual file: {new_target}"
        )),
    }
}

fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(crate::pathutil::to_slash)
        .unwrap_or_else(|_| path.display().to_string())
}

fn set_top_level_string(text: &str, key: &str, value: &str) -> String {
    set_value(
        text,
        "",
        key,
        &format!("{key} = \"{}\"", escape_toml(value)),
    )
}

fn set_section_string(text: &str, section: &str, key: &str, value: &str) -> String {
    set_value(
        text,
        section,
        key,
        &format!("{key} = \"{}\"", escape_toml(value)),
    )
}

fn set_section_u64(text: &str, section: &str, key: &str, value: u64) -> String {
    set_value(text, section, key, &format!("{key} = {value}"))
}

fn set_value(text: &str, section: &str, key: &str, replacement: &str) -> String {
    let mut out = String::new();
    let mut current = "";
    let mut saw_section = section.is_empty();
    let mut replaced = false;
    let mut inserted = false;

    for raw in text.lines() {
        let trimmed = crate::pathutil::strip_comment(raw).trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if current == section && !replaced && !inserted {
                out.push_str(replacement);
                out.push('\n');
                inserted = true;
            }
            current = trimmed
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
                .unwrap_or(trimmed)
                .trim();
            if current == section {
                saw_section = true;
            }
        }
        if current == section && !replaced && key_matches(trimmed, key) {
            out.push_str(replacement);
            out.push('\n');
            replaced = true;
            continue;
        }
        out.push_str(raw);
        out.push('\n');
    }

    if !replaced && !inserted {
        if !saw_section && !section.is_empty() {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
            out.push('[');
            out.push_str(section);
            out.push_str("]\n");
        }
        out.push_str(replacement);
        out.push('\n');
    }

    out
}

fn key_matches(line: &str, key: &str) -> bool {
    line.strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

fn escape_toml(value: &str) -> String {
    crate::ops::escape_toml_string(value)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;

    #[test]
    fn set_top_level_updates_before_sections() {
        let updated = set_top_level_string(
            "name = \"x\"\nfilename = \"a.jar\"\n[download]\n",
            "filename",
            "b.jar",
        );

        assert!(updated.starts_with("name = \"x\"\nfilename = \"b.jar\"\n[download]\n"));
    }

    #[test]
    fn set_section_inserts_missing_section() {
        let updated = set_section_string("name = \"x\"\n", "update.github", "project", "o/r");

        assert!(updated.contains("[update.github]\nproject = \"o/r\"\n"));
    }

    #[test]
    fn update_all_skips_pinned_metadata_without_network() {
        let root = temp_root("update-skips-pinned");
        create_pack(&root);
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(
            root.join("mods").join("pinned.pw"),
            "name = \"Pinned\"\n\
             filename = \"pinned.jar\"\n\
             pin = true\n\
             [download]\n\
             url = \"https://example.invalid/pinned.jar\"\n\
             hash-format = \"sha256\"\n\
             hash = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n\
             [update.github]\n\
             project = \"owner/repo\"\n",
        )
        .unwrap();

        let result = update_all(&root, UpdateOptions::default()).unwrap();

        assert!(result.skipped.iter().any(|item| item.contains("pinned")));
        assert!(result.updated.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn set_section_handles_commented_header() {
        let updated = set_section_string(
            "name = \"x\"\n[download] # source\nurl = \"old\"\n",
            "download",
            "url",
            "new",
        );

        assert!(updated.contains("url = \"new\""));
        assert_eq!(updated.matches("[download]").count(), 1);
    }

    #[test]
    fn rejects_update_filename_colliding_with_manual_file() {
        let root = temp_root("update-target-collision");
        create_pack(&root);
        fs::create_dir_all(root.join("mods")).unwrap();
        let metadata_path = root.join("mods").join("old.pw");
        fs::write(root.join("mods").join("new.jar"), b"manual").unwrap();
        let metadata = ModMetadata::parse("filename = \"old.jar\"\n");

        let result = reject_manual_target_collision(
            &root,
            &layout(),
            &metadata_path,
            &metadata,
            "new.jar",
            &"a".repeat(64),
            "sha256",
        );

        assert!(result.is_err());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_update_collision_when_existing_file_matches_sha1() {
        let root = temp_root("update-target-collision-sha1");
        create_pack(&root);
        fs::create_dir_all(root.join("mods")).unwrap();
        let metadata_path = root.join("mods").join("old.pw");
        fs::write(root.join("mods").join("new.jar"), b"abc").unwrap();
        let metadata = ModMetadata::parse("filename = \"old.jar\"\n");

        let result = reject_manual_target_collision(
            &root,
            &layout(),
            &metadata_path,
            &metadata,
            "new.jar",
            "a9993e364706816aba3e25717850c26c9cd0d89d",
            "sha1",
        );

        assert!(result.is_ok());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_update_collision_when_existing_file_matches_sha512() {
        let root = temp_root("update-target-collision-sha512");
        create_pack(&root);
        fs::create_dir_all(root.join("mods")).unwrap();
        let target = root.join("mods").join("new.jar");
        fs::write(&target, b"abc").unwrap();
        let metadata_path = root.join("mods").join("old.pw");
        let metadata = ModMetadata::parse("filename = \"old.jar\"\n");
        let hash = crate::sha512::sha512_file_hex(&target).unwrap();

        let result = reject_manual_target_collision(
            &root,
            &layout(),
            &metadata_path,
            &metadata,
            "new.jar",
            &hash,
            "sha512",
        );

        assert!(result.is_ok());

        let _ = fs::remove_dir_all(root);
    }

    fn layout() -> PackLayout {
        PackLayout {
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
            metadata_extension: "pw".to_string(),
        }
    }

    fn create_pack(root: &Path) {
        fs::write(
            root.join("pack.toml"),
            "name = \"Test\"\n[index]\nfile = \"index.toml\"\nhash-format = \"sha256\"\nhash = \"old\"\n",
        )
        .unwrap();
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
