use std::fs;
use std::io::{Write, copy};
use std::path::{Path, PathBuf};

use crate::config::ProjectConfig;
use crate::layout::PackLayout;
use crate::metadata::{ModMetadata, Side};
use crate::pathutil::join_slash;
use crate::scan::ScanReport;
use crate::sha256::sha256_file_hex;

pub fn add_local_file(
    root: &Path,
    layout: &PackLayout,
    side: &Side,
    name: &str,
    source: &Path,
    filename: Option<&str>,
) -> Result<PathBuf, String> {
    reject_unknown_side(side)?;
    if !source.exists() {
        return Err(format!("source file does not exist: {}", source.display()));
    }
    let filename = match filename {
        Some(value) => crate::pathutil::safe_filename(value)?,
        None => source
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .ok_or_else(|| format!("invalid source filename: {}", source.display()))
            .and_then(|value| crate::pathutil::safe_filename(&value))?,
    };
    let slug = safe_slug(name)?;
    let metadata_path = root
        .join(&layout.metadata_root)
        .join(metadata_filename(&slug, layout));
    if metadata_path.exists() {
        return Err(format!(
            "metadata already exists: {}",
            metadata_path.display()
        ));
    }
    let target = root.join(&layout.jar_root).join(&filename);
    if target.exists() {
        return Err(format!("target file already exists: {}", target.display()));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let tmp = unique_tmp_path(&target);
    fs::copy(source, &tmp).map_err(|err| {
        format!(
            "failed to copy {} to {}: {err}",
            source.display(),
            tmp.display()
        )
    })?;
    let hash = sha256_file_hex(&tmp)?;
    let text = write_url_metadata(name, &filename, side.as_str(), "", &hash);
    if let Err(err) = create_new_text_file(&metadata_path, &text) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = move_without_replace(&tmp, &target) {
        let _ = fs::remove_file(&metadata_path);
        let _ = fs::remove_file(&tmp);
        return Err(format!("failed to move {}: {err}", target.display()));
    }
    Ok(metadata_path)
}

pub fn add_url_metadata(
    root: &Path,
    layout: &PackLayout,
    side: &Side,
    name: &str,
    filename: &str,
    url: &str,
    hash: &str,
) -> Result<PathBuf, String> {
    add_url_metadata_in_dir(
        root,
        &layout.metadata_root,
        &layout.metadata_extension,
        side,
        name,
        filename,
        url,
        hash,
    )
}

pub fn add_resourcepack_metadata(
    root: &Path,
    layout: &PackLayout,
    name: &str,
    filename: &str,
    url: &str,
    hash: &str,
) -> Result<PathBuf, String> {
    add_url_metadata_in_dir(
        root,
        Path::new("resourcepacks"),
        &layout.metadata_extension,
        &Side::Client,
        name,
        filename,
        url,
        hash,
    )
}

pub fn add_shaderpack_metadata(
    root: &Path,
    layout: &PackLayout,
    name: &str,
    filename: &str,
    url: &str,
    hash: &str,
) -> Result<PathBuf, String> {
    add_url_metadata_in_dir(
        root,
        Path::new("shaderpacks"),
        &layout.metadata_extension,
        &Side::Client,
        name,
        filename,
        url,
        hash,
    )
}

fn add_url_metadata_in_dir(
    root: &Path,
    metadata_dir: &Path,
    metadata_extension: &str,
    side: &Side,
    name: &str,
    filename: &str,
    url: &str,
    hash: &str,
) -> Result<PathBuf, String> {
    reject_unknown_side(side)?;
    let slug = safe_slug(name)?;
    let rel = metadata_dir.join(format!(
        "{slug}.{}",
        metadata_extension.trim_start_matches('.')
    ));
    let path = root.join(&rel);
    let filename = crate::pathutil::safe_filename(filename)?;
    validate_hash("sha256", hash)?;

    let text = write_url_metadata(name, &filename, side.as_str(), url, hash);
    create_new_text_file(&path, &text)?;
    Ok(path)
}

pub fn add_curseforge_metadata(
    root: &Path,
    layout: &PackLayout,
    side: &Side,
    name: &str,
    filename: &str,
    hash_format: &str,
    hash: &str,
    project_id: u64,
    file_id: u64,
) -> Result<PathBuf, String> {
    reject_unknown_side(side)?;
    let slug = safe_slug(name)?;
    let rel = layout.metadata_root.join(metadata_filename(&slug, layout));
    let path = root.join(&rel);
    let filename = crate::pathutil::safe_filename(filename)?;
    validate_hash(hash_format, hash)?;

    let text = write_curseforge_metadata(
        name,
        &filename,
        side.as_str(),
        hash_format,
        hash,
        project_id,
        file_id,
    );
    create_new_text_file(&path, &text)?;
    Ok(path)
}

pub fn add_github_metadata(
    root: &Path,
    layout: &PackLayout,
    side: &Side,
    name: &str,
    filename: &str,
    url: &str,
    hash: &str,
    project: &str,
    tag: Option<&str>,
    asset: Option<&str>,
    export_cf_project_id: Option<u64>,
    export_cf_file_id: Option<u64>,
) -> Result<PathBuf, String> {
    reject_unknown_side(side)?;
    let slug = safe_slug(name)?;
    let rel = layout.metadata_root.join(metadata_filename(&slug, layout));
    let path = root.join(&rel);
    let filename = crate::pathutil::safe_filename(filename)?;

    let text = write_github_metadata(
        name,
        &filename,
        side.as_str(),
        url,
        hash,
        project,
        tag,
        asset,
        export_cf_project_id,
        export_cf_file_id,
    );
    create_new_text_file(&path, &text)?;
    Ok(path)
}

fn create_new_text_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| format!("failed to create {}: {err}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn metadata_filename(slug: &str, layout: &PackLayout) -> String {
    format!(
        "{slug}.{}",
        layout.metadata_extension.trim_start_matches('.')
    )
}

fn safe_slug(name: &str) -> Result<String, String> {
    let slug = slugify(name);
    if slug.is_empty() {
        return Err(format!(
            "metadata name cannot be converted to a safe filename: {name}"
        ));
    }
    Ok(slug)
}

fn validate_hash(format: &str, hash: &str) -> Result<(), String> {
    let format = format.trim().to_ascii_lowercase();
    let expected_len = match format.as_str() {
        "sha1" => 40,
        "sha256" => 64,
        "sha512" => 128,
        other => return Err(format!("unsupported hash format: {other}")),
    };
    if hash.len() != expected_len || !hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("invalid {format} hash: {hash}"));
    }
    Ok(())
}

fn unique_tmp_path(target: &Path) -> PathBuf {
    let mut extension = target
        .extension()
        .and_then(|value| value.to_str())
        .map_or(String::new(), |value| format!("{value}."));
    extension.push_str("bkmpw-add-tmp-");
    extension.push_str(&std::process::id().to_string());
    extension.push('-');
    extension.push_str(
        &std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos())
            .to_string(),
    );
    target.with_extension(extension)
}

fn move_without_replace(source: &Path, target: &Path) -> Result<(), String> {
    let mut source_file = fs::File::open(source)
        .map_err(|err| format!("failed to open {}: {err}", source.display()))?;
    let mut target_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|err| format!("failed to create {}: {err}", target.display()))?;
    if let Err(err) = copy(&mut source_file, &mut target_file) {
        let _ = fs::remove_file(target);
        return Err(format!("failed to copy {}: {err}", target.display()));
    }
    if let Err(err) = target_file.flush() {
        let _ = fs::remove_file(target);
        return Err(format!("failed to flush {}: {err}", target.display()));
    }
    fs::remove_file(source).map_err(|err| format!("failed to remove {}: {err}", source.display()))
}

fn reject_unknown_side(side: &Side) -> Result<(), String> {
    match side {
        Side::Unknown(value) => Err(format!("unsupported side: {value}")),
        Side::Client | Side::Server | Side::Both => Ok(()),
    }
}

pub fn remove_metadata(
    root: &Path,
    config: &ProjectConfig,
    layout: &PackLayout,
    name: &str,
) -> Result<PathBuf, String> {
    let path = find_metadata(root, config, layout, name)?;
    fs::remove_file(&path).map_err(|err| format!("failed to remove {}: {err}", path.display()))?;
    Ok(path)
}

fn write_url_metadata(name: &str, filename: &str, side: &str, url: &str, hash: &str) -> String {
    let mut out = String::new();
    out.push_str("name = \"");
    out.push_str(&escape_toml_string(name));
    out.push_str("\"\n");
    out.push_str("filename = \"");
    out.push_str(&escape_toml_string(filename));
    out.push_str("\"\n");
    out.push_str("side = \"");
    out.push_str(if side == "common" { "both" } else { side });
    out.push_str("\"\n\n");
    out.push_str("[download]\n");
    out.push_str("url = \"");
    out.push_str(&escape_toml_string(url));
    out.push_str("\"\n");
    out.push_str("hash-format = \"sha256\"\n");
    out.push_str("hash = \"");
    out.push_str(&escape_toml_string(hash));
    out.push_str("\"\n");
    out
}

fn write_curseforge_metadata(
    name: &str,
    filename: &str,
    side: &str,
    hash_format: &str,
    hash: &str,
    project_id: u64,
    file_id: u64,
) -> String {
    let mut out = String::new();
    out.push_str("name = \"");
    out.push_str(&escape_toml_string(name));
    out.push_str("\"\n");
    out.push_str("filename = \"");
    out.push_str(&escape_toml_string(filename));
    out.push_str("\"\n");
    out.push_str("side = \"");
    out.push_str(if side == "common" { "both" } else { side });
    out.push_str("\"\n\n");
    out.push_str("[download]\n");
    out.push_str("hash-format = \"");
    out.push_str(&escape_toml_string(hash_format));
    out.push_str("\"\n");
    out.push_str("hash = \"");
    out.push_str(&escape_toml_string(hash));
    out.push_str("\"\n");
    out.push_str("mode = \"metadata:curseforge\"\n\n");
    out.push_str("[update]\n");
    out.push_str("[update.curseforge]\n");
    out.push_str("file-id = ");
    out.push_str(&file_id.to_string());
    out.push('\n');
    out.push_str("project-id = ");
    out.push_str(&project_id.to_string());
    out.push('\n');
    out
}

fn write_github_metadata(
    name: &str,
    filename: &str,
    side: &str,
    url: &str,
    hash: &str,
    project: &str,
    tag: Option<&str>,
    asset: Option<&str>,
    export_cf_project_id: Option<u64>,
    export_cf_file_id: Option<u64>,
) -> String {
    let mut out = write_url_metadata(name, filename, side, url, hash);
    out.push('\n');
    out.push_str("[update]\n");
    out.push_str("[update.github]\n");
    out.push_str("project = \"");
    out.push_str(&escape_toml_string(project));
    out.push_str("\"\n");
    if let Some(tag) = tag.filter(|value| !value.trim().is_empty()) {
        out.push_str("tag = \"");
        out.push_str(&escape_toml_string(tag));
        out.push_str("\"\n");
    }
    if let Some(asset) = asset.filter(|value| !value.trim().is_empty()) {
        out.push_str("asset = \"");
        out.push_str(&escape_toml_string(asset));
        out.push_str("\"\n");
    }
    if let (Some(project_id), Some(file_id)) = (export_cf_project_id, export_cf_file_id) {
        out.push('\n');
        out.push_str("[export.curseforge]\n");
        out.push_str("project-id = ");
        out.push_str(&project_id.to_string());
        out.push('\n');
        out.push_str("file-id = ");
        out.push_str(&file_id.to_string());
        out.push('\n');
    }
    out
}

pub fn slugify(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

pub fn escape_toml_string(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

pub fn set_pin(
    root: &Path,
    config: &ProjectConfig,
    layout: &PackLayout,
    name: &str,
    pin: bool,
) -> Result<PathBuf, String> {
    let path = find_metadata(root, config, layout, name)?;
    let text = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let updated = set_top_level_bool(&text, "pin", pin);
    fs::write(&path, updated)
        .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    Ok(path)
}

pub(crate) fn find_metadata(
    root: &Path,
    config: &ProjectConfig,
    layout: &PackLayout,
    name: &str,
) -> Result<PathBuf, String> {
    let report = ScanReport::build(root, config, layout)?;
    let mut matches = Vec::new();

    for entry in report.metadata {
        let path = join_slash(root, &entry.path);
        let stem_matches = entry
            .path
            .rsplit('/')
            .next()
            .and_then(metadata_stem)
            .is_some_and(|stem| stem == name);
        let path_matches = entry.path == name
            || entry.path == format!("{name}.pw")
            || entry.path == format!("{name}.pw.toml");
        let metadata = ModMetadata::load(&path)?;
        let name_matches = metadata.name.as_deref() == Some(name);

        if stem_matches || path_matches || name_matches {
            matches.push(path);
        }
    }

    match matches.len() {
        0 => Err(format!("metadata not found: {name}")),
        1 => Ok(matches.remove(0)),
        _ => Err(format!("metadata name is ambiguous: {name}")),
    }
}

fn metadata_stem(file: &str) -> Option<&str> {
    file.strip_suffix(".pw")
        .or_else(|| file.strip_suffix(".pw.toml"))
}

fn set_top_level_bool(text: &str, key: &str, value: bool) -> String {
    let replacement = format!("{key} = {value}");
    let mut out = String::new();
    let mut replaced = false;
    let mut inserted = false;

    for raw in text.lines() {
        let trimmed = raw.trim_start();
        let in_section = trimmed.starts_with('[');
        if !replaced && !inserted && in_section {
            out.push_str(&replacement);
            out.push('\n');
            if !out.ends_with("\n\n") {
                out.push('\n');
            }
            inserted = true;
        }
        if !replaced && !in_section && trimmed.starts_with(key) {
            let after_key = &trimmed[key.len()..];
            if after_key.trim_start().starts_with('=') {
                out.push_str(&replacement);
                out.push('\n');
                replaced = true;
                continue;
            }
        }
        out.push_str(raw);
        out.push('\n');
    }

    if !replaced && !inserted {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&replacement);
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_mod_metadata_writes_to_mods_root() {
        let root = unique_test_dir("bkmpw-add-mod");
        let layout = layout();
        let hash = valid_sha256();
        let created = add_url_metadata(
            &root,
            &layout,
            &Side::Both,
            "Example Mod",
            "example.jar",
            "https://example.invalid/example.jar",
            &hash,
        )
        .unwrap();

        assert_eq!(created, root.join("mods").join("example-mod.pw.toml"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn add_mod_metadata_uses_configured_extension() {
        let root = unique_test_dir("bkmpw-add-mod-extension");
        let mut layout = layout();
        layout.metadata_extension = "pw".to_string();
        let hash = valid_sha256();

        let created = add_url_metadata(
            &root,
            &layout,
            &Side::Both,
            "Example Mod",
            "example.jar",
            "https://example.invalid/example.jar",
            &hash,
        )
        .unwrap();

        assert_eq!(created, root.join("mods").join("example-mod.pw"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn add_pack_asset_metadata_writes_to_asset_root() {
        let root = unique_test_dir("bkmpw-add-asset");
        let hash = valid_sha256();
        let layout = layout();
        let created = add_resourcepack_metadata(
            &root,
            &layout,
            "Example Resource Pack",
            "example.zip",
            "https://example.invalid/example.zip",
            &hash,
        )
        .unwrap();
        let text = fs::read_to_string(&created).unwrap();

        assert_eq!(
            created,
            root.join("resourcepacks")
                .join("example-resource-pack.pw.toml")
        );
        assert!(text.contains("side = \"client\""));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn add_local_file_does_not_overwrite_existing_jar() {
        let root = unique_test_dir("bkmpw-add-local-existing");
        let layout = layout();
        fs::create_dir_all(root.join("mods")).unwrap();
        let source = root.join("source.jar");
        let target = root.join("mods").join("example.jar");
        fs::write(&source, b"new").unwrap();
        fs::write(&target, b"old").unwrap();

        let result = add_local_file(
            &root,
            &layout,
            &Side::Both,
            "Example",
            &source,
            Some("example.jar"),
        );

        assert!(result.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert!(!root.join("mods").join("example.pw").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn set_pin_replaces_existing_top_level_value() {
        let text = "name = \"x\"\npin = false\n[download]\npin = false\n";
        let updated = set_top_level_bool(text, "pin", true);

        assert!(updated.starts_with("name = \"x\"\npin = true\n[download]\npin = false\n"));
    }

    #[test]
    fn set_pin_appends_when_missing() {
        let updated = set_top_level_bool("name = \"x\"\n[download]\nurl = \"x\"\n", "pin", true);

        assert!(updated.starts_with("name = \"x\"\npin = true\n\n[download]\n"));
    }

    #[test]
    fn slugify_is_stable() {
        assert_eq!(slugify("Just Enough Items"), "just-enough-items");
    }

    #[test]
    fn github_metadata_records_update_source() {
        let hash = valid_sha256();
        let text = write_github_metadata(
            "Mod",
            "mod.jar",
            "both",
            "https://example.invalid/mod.jar",
            &hash,
            "owner/repo",
            Some("latest"),
            Some("neoforge"),
            None,
            None,
        );

        assert!(text.contains("[update.github]"));
        assert!(text.contains("project = \"owner/repo\""));
        assert!(text.contains("asset = \"neoforge\""));
    }

    #[test]
    fn github_metadata_can_record_export_curseforge_mapping() {
        let hash = valid_sha256();
        let text = write_github_metadata(
            "Core",
            "core.jar",
            "both",
            "https://example.invalid/core.jar",
            &hash,
            "owner/repo",
            None,
            None,
            Some(123456),
            Some(789012),
        );

        assert!(text.contains("[export.curseforge]"));
        assert!(text.contains("project-id = 123456"));
        assert!(text.contains("file-id = 789012"));
    }

    #[test]
    fn curseforge_metadata_accepts_sha1_hash() {
        let root = unique_test_dir("bkmpw-add-cf-sha1");
        let layout = layout();

        let created = add_curseforge_metadata(
            &root,
            &layout,
            &Side::Both,
            "Example",
            "example.jar",
            "sha1",
            &"a".repeat(40),
            1,
            2,
        )
        .unwrap();

        assert!(created.exists());

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
            metadata_extension: "pw.toml".to_string(),
        }
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

    fn valid_sha256() -> String {
        "a".repeat(64)
    }
}
