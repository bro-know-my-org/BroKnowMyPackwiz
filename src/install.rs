use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::config::ProjectConfig;
use crate::curseforge;
use crate::http::http_get_to_file;
use crate::layout::PackLayout;
use crate::metadata::{ModMetadata, Side};
use crate::pathutil::join_slash;
use crate::scan::ScanReport;
use crate::sha1::sha1_file_hex;
use crate::sha256::sha256_file_hex;
use crate::sha512::sha512_file_hex;

#[derive(Debug, Clone)]
pub struct InstallResult {
    pub installed: usize,
    pub skipped: usize,
    pub removed: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub jobs: usize,
    pub retries: usize,
    pub retry_delay_seconds: u64,
    pub force: bool,
    pub cleanup: bool,
    pub preserve_existing: bool,
}

#[derive(Debug, Clone)]
struct CopyTask {
    name: String,
    source: Option<PathBuf>,
    url: Option<String>,
    curseforge: Option<CurseForgeDownload>,
    target: PathBuf,
    target_rel: String,
    expected_hash: Option<ExpectedHash>,
    preserve: bool,
    force: bool,
    managed_target: bool,
}

#[derive(Debug, Clone)]
struct ExpectedHash {
    format: String,
    value: String,
}

#[derive(Debug, Clone)]
struct CurseForgeDownload {
    api_key: Option<String>,
    cdn_fallback: bool,
    project_id: u64,
    file_id: u64,
    filename: String,
}

pub fn install_local(
    source_root: &Path,
    target_root: &Path,
    target_side: &Side,
    options: InstallOptions,
) -> Result<InstallResult, String> {
    reject_unknown_side(target_side)?;
    let config = ProjectConfig::load(source_root)?;
    let layout = PackLayout::from_config(&config);
    let report = ScanReport::build(source_root, &config, &layout)?;
    let curseforge_api_key = curseforge::api_key(&config);
    let mut tasks = VecDeque::new();
    let mut target_paths = BTreeSet::new();
    let mut skipped = 0;
    let cleanup = options.cleanup;
    let old_manifest = if cleanup {
        read_manifest_paths(target_root)?
    } else {
        BTreeSet::new()
    };

    if cleanup && report.metadata.is_empty() {
        return Err(
            "refusing to clean managed files because no metadata files were scanned".to_string(),
        );
    }

    for entry in report.metadata {
        let metadata_path = join_slash(source_root, &entry.path);
        let metadata = ModMetadata::load(&metadata_path)?;
        let declared_side = side_from_directory_or_metadata(entry.side_hint, &metadata);

        if !declared_side.installs_on(target_side) {
            skipped += 1;
            continue;
        }
        if metadata.optional && !metadata.option_default {
            skipped += 1;
            continue;
        }

        let Some(filename) = metadata.filename.clone() else {
            skipped += 1;
            continue;
        };
        let name = metadata.name.clone().unwrap_or_else(|| filename.clone());
        let url = metadata
            .download_url
            .clone()
            .filter(|value| !value.is_empty());
        let curseforge = curseforge_download(
            &metadata,
            curseforge_api_key.clone(),
            config.curseforge.cdn_fallback,
        )?;
        let expected_hash = expected_hash(
            metadata.download_hash_format.clone(),
            metadata.download_hash.clone(),
        )?;
        let file_target = resolve_pack_file_path(&entry.path, &filename, &layout)?;
        if !target_paths.insert(file_target.clone()) {
            return Err(format!("multiple metadata files target {file_target}"));
        }
        tasks.push_back(CopyTask {
            name,
            source: Some(join_slash(source_root, &file_target)),
            url,
            curseforge,
            target: join_slash(target_root, &file_target),
            managed_target: old_manifest.contains(&file_target),
            target_rel: file_target,
            expected_hash,
            preserve: metadata.preserve,
            force: options.force,
        });
    }

    let task_count = tasks.len();
    let (installed_files, errors) = run_copy_tasks(tasks, options);
    let removed = if errors.is_empty() && cleanup {
        cleanup_removed_files(target_root, &old_manifest, &installed_files)?
    } else {
        0
    };
    if errors.is_empty() {
        write_manifest(target_root, target_side, &installed_files)?;
    }
    Ok(InstallResult {
        installed: task_count - errors.len(),
        skipped,
        removed,
        errors,
    })
}

fn side_from_directory_or_metadata(hint: crate::scan::SideHint, metadata: &ModMetadata) -> Side {
    match hint {
        crate::scan::SideHint::Server => Side::Server,
        crate::scan::SideHint::Client => Side::Client,
        crate::scan::SideHint::Common => Side::Both,
        crate::scan::SideHint::Unknown => metadata.side.clone().unwrap_or(Side::Both),
    }
}

pub(crate) fn resolve_pack_file_path(
    metadata_path: &str,
    filename: &str,
    layout: &PackLayout,
) -> Result<String, String> {
    let target = if filename.contains('/') {
        crate::pathutil::safe_slash_path(filename)?
    } else {
        let filename = crate::pathutil::safe_filename(filename)?;
        if is_side_metadata_path(metadata_path, layout) {
            crate::pathutil::safe_slash_path(&format!(
                "{}/{}",
                layout.jar_root.to_string_lossy(),
                filename
            ))?
        } else {
            let parent = metadata_path
                .rsplit_once('/')
                .map_or("", |(parent, _)| parent);
            if parent.is_empty() {
                filename
            } else {
                crate::pathutil::safe_slash_path(&format!("{parent}/{filename}"))?
            }
        }
    };

    if is_managed_pack_file_path(&target, layout) {
        Ok(target)
    } else {
        Err(format!(
            "metadata filename resolves outside managed roots: {filename}"
        ))
    }
}

fn reject_unknown_side(side: &Side) -> Result<(), String> {
    match side {
        Side::Unknown(value) => Err(format!("unsupported side: {value}")),
        Side::Client | Side::Server | Side::Both => Ok(()),
    }
}

fn is_managed_pack_file_path(rel: &str, layout: &PackLayout) -> bool {
    crate::pathutil::is_under_slash(rel, &layout.jar_root.to_string_lossy())
        || layout
            .metadata_roots
            .iter()
            .any(|root| crate::pathutil::is_under_slash(rel, &root.to_string_lossy()))
}

fn is_side_metadata_path(metadata_path: &str, layout: &PackLayout) -> bool {
    fn is_under(rel: &str, parent: &str) -> bool {
        let parent = crate::pathutil::normalize_slash(parent);
        rel == parent
            || rel
                .strip_prefix(&parent)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    is_under(metadata_path, &layout.server_meta.to_string_lossy())
        || is_under(metadata_path, &layout.client_meta.to_string_lossy())
        || is_under(metadata_path, &layout.common_meta.to_string_lossy())
}

fn curseforge_download(
    metadata: &ModMetadata,
    api_key: Option<String>,
    cdn_fallback: bool,
) -> Result<Option<CurseForgeDownload>, String> {
    if metadata.download_mode.as_deref() != Some("metadata:curseforge") {
        return Ok(None);
    }
    let project_id = metadata
        .curseforge_project_id
        .ok_or_else(|| "CurseForge metadata is missing update.curseforge.project-id".to_string())?;
    let file_id = metadata
        .curseforge_file_id
        .ok_or_else(|| "CurseForge metadata is missing update.curseforge.file-id".to_string())?;
    Ok(Some(CurseForgeDownload {
        api_key,
        cdn_fallback,
        project_id,
        file_id,
        filename: metadata.filename.clone().unwrap_or_default(),
    }))
}

fn expected_hash(
    format: Option<String>,
    value: Option<String>,
) -> Result<Option<ExpectedHash>, String> {
    let (Some(format), Some(value)) = (format, value) else {
        return Ok(None);
    };
    let format = format.trim().to_ascii_lowercase();
    let value = value.trim().to_ascii_lowercase();
    match format.as_str() {
        "sha1" | "sha256" | "sha512" => Ok(Some(ExpectedHash { format, value })),
        _ => Err(format!("unsupported hash format in metadata: {format}")),
    }
}

fn run_copy_tasks(
    tasks: VecDeque<CopyTask>,
    options: InstallOptions,
) -> (Vec<InstalledFile>, Vec<String>) {
    let tasks = Arc::new(Mutex::new(tasks));
    let errors = Arc::new(Mutex::new(Vec::new()));
    let installed = Arc::new(Mutex::new(Vec::new()));
    let jobs = options.jobs.max(1);

    thread::scope(|scope| {
        for _ in 0..jobs {
            let tasks = Arc::clone(&tasks);
            let errors = Arc::clone(&errors);
            let installed = Arc::clone(&installed);
            let options = options.clone();
            scope.spawn(move || {
                loop {
                    let task = {
                        let mut guard = tasks.lock().expect("copy task mutex poisoned");
                        guard.pop_front()
                    };
                    let Some(task) = task else {
                        break;
                    };
                    eprintln!("processing {}", task.target_rel);
                    match copy_with_retries(&task, &options) {
                        Ok(hash) => {
                            let mut guard = installed.lock().expect("installed mutex poisoned");
                            guard.push(InstalledFile {
                                name: task.name,
                                path: task.target_rel,
                                sha256: hash,
                            });
                        }
                        Err(err) => {
                            let mut guard = errors.lock().expect("copy error mutex poisoned");
                            guard.push(err);
                        }
                    }
                }
            });
        }
    });

    let mut errors = Arc::try_unwrap(errors)
        .expect("copy errors still referenced")
        .into_inner()
        .expect("copy error mutex poisoned");
    errors.sort();
    let mut installed = Arc::try_unwrap(installed)
        .expect("installed files still referenced")
        .into_inner()
        .expect("installed mutex poisoned");
    installed.sort_by(|a, b| a.path.cmp(&b.path));
    (installed, errors)
}

fn copy_with_retries(task: &CopyTask, options: &InstallOptions) -> Result<String, String> {
    let attempts = options.retries.max(1);
    let mut last_error = None;
    for attempt in 1..=attempts {
        match copy_atomic(task, options.preserve_existing) {
            Ok(hash) => return Ok(hash),
            Err(err) => {
                last_error = Some(err);
                if attempt < attempts && options.retry_delay_seconds > 0 {
                    thread::sleep(Duration::from_secs(options.retry_delay_seconds));
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| format!("failed to install {}", task.name)))
}

fn copy_atomic(task: &CopyTask, preserve_existing: bool) -> Result<String, String> {
    if !task.force
        && let Some(expected) = &task.expected_hash
        && task.target.exists()
    {
        if file_hash_hex(&task.target, &expected.format).as_ref() == Ok(&expected.value) {
            return sha256_file_hex(&task.target);
        }
        if preserve_existing || task.preserve {
            return Err(format!(
                "existing file hash mismatch for {}; use --force to replace it",
                task.target.display()
            ));
        }
    }
    if !task.force && (task.preserve || preserve_existing) && task.target.exists() {
        return sha256_file_hex(&task.target);
    }
    if !task.force && task.target.exists() && !task.managed_target {
        return Err(format!(
            "refusing to overwrite unmanaged file: {}",
            task.target.display()
        ));
    }
    if let Some(parent) = task.target.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }

    let tmp = unique_tmp_path(&task.target);
    if let Some(source) = valid_local_source(task)? {
        fs::copy(source, &tmp).map_err(|err| {
            format!(
                "failed to copy {} to {}: {err}",
                source.display(),
                tmp.display()
            )
        })?;
    } else if let Some(url) = &task.url {
        http_get_to_file(url, &tmp)?;
    } else if let Some(curseforge) = &task.curseforge {
        download_curseforge(curseforge, &tmp)?;
    } else {
        return Err(format!("missing source file for {}", task.name));
    }
    if let Some(expected) = &task.expected_hash {
        let actual = file_hash_hex(&tmp, &expected.format)?;
        if actual != expected.value {
            let _ = fs::remove_file(&tmp);
            return Err(format!(
                "hash mismatch for {}: expected {} {}, got {}",
                task.name, expected.format, expected.value, actual
            ));
        }
    }
    replace_with_tmp(&tmp, &task.target)?;
    sha256_file_hex(&task.target)
}

fn replace_with_tmp(tmp: &Path, target: &Path) -> Result<(), String> {
    match fs::rename(tmp, target) {
        Ok(()) => Ok(()),
        Err(first_err) if target.exists() => {
            fs::remove_file(target)
                .map_err(|err| format!("failed to remove {}: {err}", target.display()))?;
            fs::rename(tmp, target).map_err(|err| {
                format!(
                    "failed to replace {} after removing old file: {err}; initial rename error: {first_err}",
                    target.display()
                )
            })
        }
        Err(err) => Err(format!("failed to replace {}: {err}", target.display())),
    }
}

fn unique_tmp_path(target: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let mut extension = target
        .extension()
        .and_then(|value| value.to_str())
        .map_or(String::new(), |value| format!("{value}."));
    extension.push_str("bkmpw-tmp-");
    extension.push_str(&std::process::id().to_string());
    extension.push('-');
    extension.push_str(
        &std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos())
            .to_string(),
    );
    extension.push('-');
    extension.push_str(&COUNTER.fetch_add(1, Ordering::Relaxed).to_string());
    target.with_extension(extension)
}

fn valid_local_source(task: &CopyTask) -> Result<Option<&PathBuf>, String> {
    let Some(source) = task.source.as_ref().filter(|source| source.exists()) else {
        return Ok(None);
    };
    if same_path(source, &task.target) {
        return Ok(None);
    }
    if let Some(expected) = &task.expected_hash
        && file_hash_hex(source, &expected.format)? != expected.value
    {
        return Ok(None);
    }
    Ok(Some(source))
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn download_curseforge(curseforge: &CurseForgeDownload, tmp: &Path) -> Result<(), String> {
    let mut cdn_error = None;
    if curseforge.cdn_fallback
        && let Some(url) = curseforge::cdn_download_url(curseforge.file_id, &curseforge.filename)
    {
        match http_get_to_file(&url, tmp) {
            Ok(()) => return Ok(()),
            Err(err) => cdn_error = Some(err),
        }
    }

    if curseforge.api_key.is_some() {
        let url = curseforge::resolve_download_url(
            curseforge.api_key.as_deref(),
            curseforge.project_id,
            curseforge.file_id,
        )?;
        return http_get_to_file(&url, tmp);
    }

    Err(cdn_error.unwrap_or_else(|| {
        "CurseForge metadata needs [curseforge] api-key or CURSEFORGE_API_KEY".to_string()
    }))
}

fn file_hash_hex(path: &Path, format: &str) -> Result<String, String> {
    match format {
        "sha1" => sha1_file_hex(path),
        "sha256" => sha256_file_hex(path),
        "sha512" => sha512_file_hex(path),
        other => Err(format!("unsupported hash format: {other}")),
    }
}

#[derive(Debug, Clone)]
struct InstalledFile {
    name: String,
    path: String,
    sha256: String,
}

fn cleanup_removed_files(
    root: &Path,
    old_paths: &BTreeSet<String>,
    new_files: &[InstalledFile],
) -> Result<usize, String> {
    let new_paths = new_files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut removed = 0;
    for path in old_paths {
        if new_paths.contains(path.as_str()) {
            continue;
        }
        if !is_safe_manifest_path(path) {
            continue;
        }
        let full_path = join_slash(root, path);
        if full_path.is_file() {
            fs::remove_file(&full_path)
                .map_err(|err| format!("failed to remove {}: {err}", full_path.display()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn is_safe_manifest_path(path: &str) -> bool {
    let Ok(normalized) = crate::pathutil::safe_slash_path(path) else {
        return false;
    };
    normalized.starts_with("mods/")
        || normalized.starts_with("resourcepacks/")
        || normalized.starts_with("shaderpacks/")
}

pub(crate) fn read_manifest_paths(root: &Path) -> Result<BTreeSet<String>, String> {
    let path = root.join("packwiz.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(BTreeSet::new());
    };
    if json_string_field(&text, "format").as_deref() != Some("bkmpw:1") {
        return Ok(BTreeSet::new());
    }
    let mut paths = BTreeSet::new();
    let Some(files_idx) = text.find("\"files\"") else {
        return Ok(BTreeSet::new());
    };
    let Some(files_body) = json_array_after_field(&text[files_idx..], "files") else {
        return Ok(BTreeSet::new());
    };
    let mut rest = files_body;
    while let Some(idx) = rest.find("\"path\"") {
        let after_name = &rest[idx + "\"path\"".len()..];
        let Some(after_colon) = after_name.trim_start().strip_prefix(':') else {
            break;
        };
        let after_colon = after_colon.trim_start();
        let Some((value, consumed)) = parse_json_string_with_len(after_colon) else {
            break;
        };
        paths.insert(crate::pathutil::normalize_slash(&value));
        rest = &after_colon[consumed.min(after_colon.len())..];
    }
    Ok(paths)
}

fn json_array_after_field<'a>(text: &'a str, field: &str) -> Option<&'a str> {
    let needle = format!("\"{field}\"");
    let idx = text.find(&needle)?;
    let after_name = &text[idx + needle.len()..];
    let mut chars = after_name
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .char_indices();
    let (_, first) = chars.next()?;
    if first != '[' {
        return None;
    }
    let start = 1;
    let mut depth = 1usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in after_name.trim_start().strip_prefix(':')?.trim_start()[start..].char_indices()
    {
        match ch {
            '\\' if in_string => escaped = !escaped,
            '"' if !escaped => {
                in_string = !in_string;
                escaped = false;
            }
            '[' if !in_string => depth += 1,
            ']' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(
                        &after_name.trim_start().strip_prefix(':')?.trim_start()
                            [start..start + idx],
                    );
                }
            }
            _ => escaped = false,
        }
    }
    None
}

fn json_string_field(text: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\"");
    let idx = text.find(&needle)?;
    let after_name = &text[idx + needle.len()..];
    let after_colon = after_name.trim_start().strip_prefix(':')?.trim_start();
    parse_json_string_with_len(after_colon).map(|(value, _)| value)
}

fn write_manifest(root: &Path, side: &Side, files: &[InstalledFile]) -> Result<(), String> {
    fs::create_dir_all(root)
        .map_err(|err| format!("failed to create {}: {err}", root.display()))?;
    let path = root.join("packwiz.json");
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"format\": \"bkmpw:1\",\n");
    out.push_str("  \"side\": \"");
    out.push_str(&json_escape(side.as_str()));
    out.push_str("\",\n");
    out.push_str("  \"files\": [\n");
    for (idx, item) in files.iter().enumerate() {
        let comma = if idx + 1 == files.len() { "" } else { "," };
        out.push_str("    {\"name\":\"");
        out.push_str(&json_escape(&item.name));
        out.push_str("\",\"path\":\"");
        out.push_str(&json_escape(&item.path));
        out.push_str("\",\"sha256\":\"");
        out.push_str(&item.sha256);
        out.push_str("\"}");
        out.push_str(comma);
        out.push('\n');
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    crate::pathutil::write_atomic(&path, out)
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

fn parse_json_string_with_len(text: &str) -> Option<(String, usize)> {
    let mut chars = text.strip_prefix('"')?.char_indices();
    let mut out = String::new();
    while let Some((idx, ch)) = chars.next() {
        match ch {
            '"' => return Some((out, idx + 2)),
            '\\' => {
                let (_, escaped) = chars.next()?;
                match escaped {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\u{0008}'),
                    'f' => out.push('\u{000c}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    other => out.push(other),
                }
            }
            other => out.push(other),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

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
            metadata_extension: "pw".to_string(),
        }
    }

    #[test]
    fn side_metadata_installs_to_jar_root() {
        assert_eq!(
            resolve_pack_file_path("mods/client/foo.pw", "foo.jar", &layout()).unwrap(),
            "mods/foo.jar"
        );
    }

    #[test]
    fn non_mod_metadata_installs_next_to_metadata() {
        assert_eq!(
            resolve_pack_file_path("resourcepacks/foo.pw.toml", "foo.zip", &layout()).unwrap(),
            "resourcepacks/foo.zip"
        );
    }

    #[test]
    fn explicit_paths_are_root_relative() {
        assert_eq!(
            resolve_pack_file_path("resourcepacks/foo.pw.toml", "mods/foo.jar", &layout()).unwrap(),
            "mods/foo.jar"
        );
    }

    #[test]
    fn rejects_path_traversal_targets() {
        assert!(resolve_pack_file_path("mods/foo.pw", "../pack.toml", &layout()).is_err());
        assert!(resolve_pack_file_path("mods/foo.pw", "saves/world.dat", &layout()).is_err());
        assert!(resolve_pack_file_path("mods/foo.pw", "nested\\foo.jar", &layout()).is_err());
    }

    #[test]
    fn directory_side_overrides_metadata_side() {
        let client_meta = ModMetadata::parse("side = \"server\"\n");
        let server_meta = ModMetadata::parse("side = \"client\"\n");
        let common_meta = ModMetadata::parse("side = \"server\"\n");

        assert_eq!(
            side_from_directory_or_metadata(crate::scan::SideHint::Client, &client_meta),
            Side::Client
        );
        assert_eq!(
            side_from_directory_or_metadata(crate::scan::SideHint::Server, &server_meta),
            Side::Server
        );
        assert_eq!(
            side_from_directory_or_metadata(crate::scan::SideHint::Common, &common_meta),
            Side::Both
        );
    }

    #[test]
    fn force_overwrites_preserved_target() {
        let root = temp_root("force-overwrites-preserved-target");
        let source = root.join("source.jar");
        let target = root.join("target.jar");
        fs::write(&source, b"new").unwrap();
        fs::write(&target, b"old").unwrap();

        let task = CopyTask {
            name: "force-test".to_string(),
            source: Some(source),
            url: None,
            curseforge: None,
            target: target.clone(),
            target_rel: "mods/target.jar".to_string(),
            expected_hash: None,
            preserve: true,
            force: true,
            managed_target: true,
        };

        copy_atomic(&task, false).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn copy_atomic_accepts_sha512_expected_hash() {
        let root = temp_root("copy-atomic-accepts-sha512");
        let source = root.join("source.jar");
        let target = root.join("target.jar");
        fs::write(&source, b"payload").unwrap();
        let expected = sha512_file_hex(&source).unwrap();

        let task = CopyTask {
            name: "sha512-test".to_string(),
            source: Some(source),
            url: None,
            curseforge: None,
            target: target.clone(),
            target_rel: "mods/target.jar".to_string(),
            expected_hash: Some(ExpectedHash {
                format: "sha512".to_string(),
                value: expected,
            }),
            preserve: false,
            force: false,
            managed_target: false,
        };

        copy_atomic(&task, false).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"payload");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn copy_atomic_skips_existing_matching_hash() {
        let root = temp_root("copy-atomic-skips-existing-matching-hash");
        let source = root.join("source.jar");
        let target = root.join("target.jar");
        fs::write(&source, b"new").unwrap();
        fs::write(&target, b"existing").unwrap();
        let expected = sha256_file_hex(&target).unwrap();

        let task = CopyTask {
            name: "hash-match-test".to_string(),
            source: Some(source),
            url: None,
            curseforge: None,
            target: target.clone(),
            target_rel: "mods/target.jar".to_string(),
            expected_hash: Some(ExpectedHash {
                format: "sha256".to_string(),
                value: expected,
            }),
            preserve: false,
            force: false,
            managed_target: false,
        };

        copy_atomic(&task, false).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"existing");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preserve_existing_rejects_hash_mismatch() {
        let root = temp_root("preserve-existing-rejects-hash-mismatch");
        let source = root.join("source.jar");
        let target = root.join("target.jar");
        fs::write(&source, b"expected").unwrap();
        fs::write(&target, b"wrong").unwrap();
        let expected = sha256_file_hex(&source).unwrap();

        let task = CopyTask {
            name: "hash-mismatch-test".to_string(),
            source: Some(source),
            url: None,
            curseforge: None,
            target: target.clone(),
            target_rel: "mods/target.jar".to_string(),
            expected_hash: Some(ExpectedHash {
                format: "sha256".to_string(),
                value: expected,
            }),
            preserve: false,
            force: false,
            managed_target: false,
        };

        let err = copy_atomic(&task, true).unwrap_err();
        assert!(err.contains("existing file hash mismatch"));
        assert_eq!(fs::read(&target).unwrap(), b"wrong");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_removes_only_old_managed_pack_files() {
        let root = temp_root("cleanup-removes-only-old-managed-pack-files");
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::create_dir_all(root.join("saves")).unwrap();
        fs::write(root.join("mods").join("old.jar"), b"old").unwrap();
        fs::write(root.join("saves").join("world.dat"), b"save").unwrap();
        let mut old = BTreeSet::new();
        old.insert("mods/old.jar".to_string());
        old.insert("saves/world.dat".to_string());

        let removed = cleanup_removed_files(&root, &old, &[]).unwrap();

        assert_eq!(removed, 1);
        assert!(!root.join("mods").join("old.jar").exists());
        assert!(root.join("saves").join("world.dat").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_ignores_non_bkmpw_manifest() {
        let root = temp_root("cleanup-ignores-non-bkmpw-manifest");
        fs::write(
            root.join("packwiz.json"),
            "{\"files\":[{\"path\":\"mods/old.jar\"}]}",
        )
        .unwrap();

        let paths = read_manifest_paths(&root).unwrap();

        assert!(paths.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_reads_only_bkmpw_manifest_files() {
        let root = temp_root("cleanup-reads-bkmpw-manifest");
        fs::write(
            root.join("packwiz.json"),
            "{\"path\":\"mods/outside-array.jar\",\"format\":\"bkmpw:1\",\"files\":[{\"path\":\"mods/old.jar\"}]}",
        )
        .unwrap();

        let paths = read_manifest_paths(&root).unwrap();

        assert!(paths.contains("mods/old.jar"));
        assert!(!paths.contains("mods/outside-array.jar"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_ignores_paths_after_files_array() {
        let root = temp_root("cleanup-ignores-after-files");
        fs::write(
            root.join("packwiz.json"),
            "{\"format\":\"bkmpw:1\",\"files\":[{\"path\":\"mods/old.jar\"}],\"other\":{\"path\":\"mods/manual.jar\"}}",
        )
        .unwrap();

        let paths = read_manifest_paths(&root).unwrap();

        assert!(paths.contains("mods/old.jar"));
        assert!(!paths.contains("mods/manual.jar"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn copy_atomic_refuses_unmanaged_existing_target() {
        let root = temp_root("copy-refuses-unmanaged");
        let source = root.join("source.jar");
        let target = root.join("target.jar");
        fs::write(&source, b"new").unwrap();
        fs::write(&target, b"manual").unwrap();
        let task = CopyTask {
            name: "manual".to_string(),
            source: Some(source),
            url: None,
            curseforge: None,
            target: target.clone(),
            target_rel: "mods/target.jar".to_string(),
            expected_hash: None,
            preserve: false,
            force: false,
            managed_target: false,
        };

        assert!(copy_atomic(&task, false).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"manual");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn download_mode_preserves_existing_unmanaged_targets() {
        let root = temp_root("download-mode-preserves-existing-unmanaged-targets");
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(
            root.join("mods").join("existing.pw"),
            "name = \"Existing\"\nfilename = \"existing.jar\"\n",
        )
        .unwrap();
        fs::write(root.join("mods").join("existing.jar"), b"manual").unwrap();

        let result = install_local(
            &root,
            &root,
            &Side::Both,
            InstallOptions {
                jobs: 1,
                retries: 1,
                retry_delay_seconds: 0,
                force: false,
                cleanup: false,
                preserve_existing: true,
            },
        )
        .unwrap();

        assert!(result.errors.is_empty());
        assert_eq!(result.installed, 1);
        assert_eq!(
            fs::read(root.join("mods").join("existing.jar")).unwrap(),
            b"manual"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_refuses_empty_metadata_scan_without_old_manifest() {
        let root = temp_root("cleanup-refuses-empty-metadata-scan-without-old-manifest");
        fs::write(root.join("pack.toml"), "name = \"test\"\n").unwrap();
        fs::write(root.join(".packwizignore"), "/*\n").unwrap();

        let err = install_local(
            &root,
            &root,
            &Side::Both,
            InstallOptions {
                jobs: 1,
                retries: 1,
                retry_delay_seconds: 0,
                force: false,
                cleanup: true,
                preserve_existing: false,
            },
        )
        .unwrap_err();

        assert!(err.contains("no metadata files were scanned"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_refuses_empty_metadata_scan_with_old_manifest() {
        let root = temp_root("cleanup-refuses-empty-metadata-scan");
        fs::write(root.join("pack.toml"), "name = \"test\"\n").unwrap();
        fs::write(root.join(".packwizignore"), "/*\n").unwrap();
        fs::write(
            root.join("packwiz.json"),
            "{\"format\":\"bkmpw:1\",\"files\":[{\"path\":\"mods/old.jar\"}]}",
        )
        .unwrap();
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods").join("old.jar"), b"old").unwrap();

        let err = install_local(
            &root,
            &root,
            &Side::Both,
            InstallOptions {
                jobs: 1,
                retries: 1,
                retry_delay_seconds: 0,
                force: false,
                cleanup: true,
                preserve_existing: false,
            },
        )
        .unwrap_err();

        assert!(err.contains("no metadata files were scanned"));
        assert!(root.join("mods").join("old.jar").exists());

        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("bkmpw-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
