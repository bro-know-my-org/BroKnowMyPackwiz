use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::config::ProjectConfig;
use crate::install;
use crate::layout::PackLayout;
use crate::metadata::{ModMetadata, Side};
use crate::packinfo::PackInfo;
use crate::pathutil::{join_slash, safe_filename, safe_slash_path};
use crate::scan::{ScanReport, is_metadata_file};
use crate::zipstore::ZipStore;

pub fn export_server(root: &Path, output: &Path) -> Result<usize, String> {
    reject_path_symlink(root)?;
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let entries = collect_direct_entries(root, &Side::Server, output_rel_in_root(root, output)?)?;

    let temp_output = temp_zip_path(output);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_output)
        .map_err(|err| format!("failed to create {}: {err}", temp_output.display()))?;
    let mut zip = ZipStore::new(file);
    let result = (|| {
        for entry in &entries {
            reject_symlink(root, &entry.source_rel)?;
            zip.add_file(&entry.target_rel, &join_slash(root, &entry.source_rel))?;
        }
        zip.finish()?;
        replace_output(&temp_output, output)?;
        Ok(entries.len())
    })();
    cleanup_temp_on_error(result, &temp_output)
}

pub fn export_client(root: &Path, output: &Path, root_name: Option<&str>) -> Result<usize, String> {
    reject_path_symlink(root)?;
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let entries = collect_direct_entries(root, &Side::Client, output_rel_in_root(root, output)?)?;
    let root_name = match root_name {
        Some(value) => safe_filename(value)?,
        None => default_client_root_name(root)?,
    };

    let temp_output = temp_zip_path(output);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_output)
        .map_err(|err| format!("failed to create {}: {err}", temp_output.display()))?;
    let mut zip = ZipStore::new(file);
    let result = (|| {
        for entry in &entries {
            reject_symlink(root, &entry.source_rel)?;
            zip.add_file(
                &format!("{root_name}/{}", entry.target_rel),
                &join_slash(root, &entry.source_rel),
            )?;
        }
        zip.finish()?;
        replace_output(&temp_output, output)?;
        Ok(entries.len())
    })();
    cleanup_temp_on_error(result, &temp_output)
}

pub fn prepare_server(root: &Path, output_dir: &Path) -> Result<usize, String> {
    reject_path_symlink(root)?;
    let config = ProjectConfig::load(root)?;
    let layout = PackLayout::from_config(&config);
    reset_output_dir(root, output_dir, &layout)?;
    let entries =
        collect_direct_entries(root, &Side::Server, output_rel_in_root(root, output_dir)?)?;
    fs::create_dir_all(output_dir)
        .map_err(|err| format!("failed to create {}: {err}", output_dir.display()))?;
    for entry in &entries {
        reject_symlink(root, &entry.source_rel)?;
        let target = join_slash(output_dir, &entry.target_rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        fs::copy(join_slash(root, &entry.source_rel), &target).map_err(|err| {
            format!(
                "failed to copy {} to {}: {err}",
                entry.source_rel,
                target.display()
            )
        })?;
    }
    Ok(entries.len())
}

pub fn export_server_installer(root: &Path, output: &Path) -> Result<usize, String> {
    reject_path_symlink(root)?;
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let entries = collect_installer_entries(root, output_rel_in_root(root, output)?)?;

    let temp_output = temp_zip_path(output);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_output)
        .map_err(|err| format!("failed to create {}: {err}", temp_output.display()))?;
    let mut zip = ZipStore::new(file);
    let result = (|| {
        for entry in &entries {
            reject_symlink(root, &entry.source_rel)?;
            zip.add_file(&entry.target_rel, &join_slash(root, &entry.source_rel))?;
        }
        zip.add_bytes("install-server.bat", install_server_bat().as_bytes())?;
        zip.add_bytes("install-server.sh", install_server_sh().as_bytes())?;
        zip.finish()?;
        replace_output(&temp_output, output)?;
        Ok(entries.len() + 2)
    })();
    cleanup_temp_on_error(result, &temp_output)
}

fn collect_direct_entries(
    root: &Path,
    target_side: &Side,
    output_rel: Option<String>,
) -> Result<Vec<PackEntry>, String> {
    collect_entries(
        root,
        target_side,
        output_rel,
        CollectOptions {
            include_runtime_jars: true,
            include_metadata: false,
            include_packwiz_files: false,
        },
    )
}

fn collect_installer_entries(
    root: &Path,
    output_rel: Option<String>,
) -> Result<Vec<PackEntry>, String> {
    collect_entries(
        root,
        &Side::Server,
        output_rel,
        CollectOptions {
            include_runtime_jars: false,
            include_metadata: true,
            include_packwiz_files: true,
        },
    )
}

#[derive(Debug, Clone, Copy)]
struct CollectOptions {
    include_runtime_jars: bool,
    include_metadata: bool,
    include_packwiz_files: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackEntry {
    source_rel: String,
    target_rel: String,
}

fn collect_entries(
    root: &Path,
    target_side: &Side,
    output_rel: Option<String>,
    options: CollectOptions,
) -> Result<Vec<PackEntry>, String> {
    let config = ProjectConfig::load(root)?;
    let layout = PackLayout::from_config(&config);
    let report = ScanReport::build(root, &config, &layout)?;
    let mut entries = BTreeMap::new();
    let mut metadata_targets = BTreeSet::new();

    for entry in &report.metadata {
        let metadata = ModMetadata::load(&join_slash(root, &entry.path))?;
        let Some(filename) = metadata.filename.as_deref() else {
            continue;
        };
        let target = install::resolve_pack_file_path(&entry.path, filename, &layout)?;
        metadata_targets.insert(target.clone());

        let declared_side = side_from_directory_or_metadata(entry.side_hint, &metadata);
        if metadata.optional && !metadata.option_default {
            continue;
        }
        if !declared_side.installs_on(target_side) {
            continue;
        }
        if options.include_metadata {
            insert_entry(&mut entries, entry.path.clone(), entry.path.clone());
        }
        if options.include_runtime_jars {
            if !join_slash(root, &target).is_file() {
                return Err(format!(
                    "missing required file for {}: {target}",
                    entry.path
                ));
            }
            insert_entry(&mut entries, target.clone(), target);
        }
    }

    for rel in &report.included {
        if is_root_overlay_source(rel, &layout) {
            continue;
        }
        if !options.include_runtime_jars
            && rel.ends_with(".jar")
            && crate::pathutil::is_under_slash(rel, &layout.jar_root.to_string_lossy())
        {
            continue;
        }
        if skip_server_entry(rel, &layout, options.include_packwiz_files)
            || metadata_targets.contains(rel)
            || is_output_path(rel, output_rel.as_deref())
        {
            continue;
        }
        if !server_entry_side(rel, &layout).installs_on(target_side) {
            continue;
        }
        insert_entry(&mut entries, rel.clone(), rel.clone());
    }

    if options.include_packwiz_files {
        for rel in [
            "pack.toml",
            "index.toml",
            ".packwizignore",
            ".pw/config.toml",
        ] {
            if is_output_path(rel, output_rel.as_deref()) {
                continue;
            }
            if join_slash(root, rel).is_file() {
                insert_entry(&mut entries, rel.to_string(), rel.to_string());
            }
        }
    }

    add_root_overlays(
        root,
        &layout,
        target_side,
        output_rel.as_deref(),
        &mut entries,
    )?;

    Ok(entries
        .into_iter()
        .map(|(target_rel, source_rel)| PackEntry {
            source_rel,
            target_rel,
        })
        .collect())
}

fn insert_entry(entries: &mut BTreeMap<String, String>, source_rel: String, target_rel: String) {
    entries.insert(target_rel, source_rel);
}

fn is_root_overlay_source(rel: &str, layout: &PackLayout) -> bool {
    let root_overlays = layout.root_overlays.to_string_lossy();
    crate::pathutil::is_under_slash(rel, &root_overlays)
}

fn add_root_overlays(
    root: &Path,
    layout: &PackLayout,
    target_side: &Side,
    output_rel: Option<&str>,
    entries: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let overlays_rel = crate::pathutil::normalize_slash(&layout.root_overlays.to_string_lossy());
    if overlays_rel.is_empty() || overlays_rel == "." {
        return Ok(());
    }
    let overlays_root = join_slash(root, &overlays_rel);
    if !overlays_root.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(&overlays_root).map_err(|err| {
        format!(
            "failed to read metadata for {}: {err}",
            overlays_root.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!("refusing to export symlink: {overlays_rel}"));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "root overlay path is not a directory: {}",
            overlays_root.display()
        ));
    }
    // Only side buckets are overlays; files directly under the overlays root are ignored.
    for side in overlay_dirs(target_side, &overlays_rel) {
        if join_slash(root, &side).is_dir() {
            collect_overlay_entries(root, &side, &side, output_rel, entries)?;
        }
    }
    Ok(())
}

fn overlay_dirs(target_side: &Side, overlays_rel: &str) -> Vec<String> {
    let mut dirs = vec![format!("{overlays_rel}/common")];
    match target_side {
        Side::Client => dirs.push(format!("{overlays_rel}/client")),
        Side::Server => dirs.push(format!("{overlays_rel}/server")),
        Side::Both => {
            dirs.push(format!("{overlays_rel}/client"));
            dirs.push(format!("{overlays_rel}/server"));
        }
        Side::Unknown(_) => {}
    }
    dirs
}

fn collect_overlay_entries(
    root: &Path,
    overlay_rel: &str,
    current_rel: &str,
    output_rel: Option<&str>,
    entries: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let current = join_slash(root, current_rel);
    let dir = fs::read_dir(&current)
        .map_err(|err| format!("failed to read {}: {err}", current.display()))?;
    for item in dir {
        let item = item.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let path = item.path();
        let rel = crate::pathutil::relative_slash(root, &path)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|err| format!("failed to read metadata for {}: {err}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("refusing to export symlink: {rel}"));
        }
        if metadata.is_dir() {
            collect_overlay_entries(root, overlay_rel, &rel, output_rel, entries)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let target_rel = rel
            .strip_prefix(overlay_rel)
            .and_then(|value| value.strip_prefix('/'))
            .ok_or_else(|| format!("invalid root overlay path: {rel}"))?
            .to_string();
        crate::pathutil::safe_slash_path(&target_rel)?;
        if is_output_path(&target_rel, output_rel) {
            continue;
        }
        insert_entry(entries, rel, target_rel);
    }
    Ok(())
}

fn side_from_directory_or_metadata(hint: crate::scan::SideHint, metadata: &ModMetadata) -> Side {
    match hint {
        crate::scan::SideHint::Server => Side::Server,
        crate::scan::SideHint::Client => Side::Client,
        crate::scan::SideHint::Common => Side::Both,
        crate::scan::SideHint::Unknown => metadata.side.clone().unwrap_or(Side::Both),
    }
}

fn server_entry_side(rel: &str, layout: &PackLayout) -> Side {
    if crate::pathutil::is_under_slash(rel, &layout.client_meta.to_string_lossy())
        || layout.metadata_roots.iter().any(|root| {
            normalize_layout_path(root) != normalize_layout_path(&layout.metadata_root)
                && crate::pathutil::is_under_slash(rel, &root.to_string_lossy())
        })
    {
        Side::Client
    } else if crate::pathutil::is_under_slash(rel, &layout.server_meta.to_string_lossy()) {
        Side::Server
    } else {
        Side::Both
    }
}

fn normalize_layout_path(path: &Path) -> String {
    crate::pathutil::normalize_slash(&path.to_string_lossy())
}

fn skip_server_entry(rel: &str, layout: &PackLayout, include_packwiz_files: bool) -> bool {
    rel == "packwiz.json"
        || rel.ends_with(".mrpack")
        || (!include_packwiz_files
            && (rel == "pack.toml"
                || rel == "index.toml"
                || rel == ".pw/config.toml"
                || rel == ".packwizignore"))
        || is_metadata_file(rel, layout)
}

fn is_output_path(rel: &str, output_rel: Option<&str>) -> bool {
    let Some(output_rel) = output_rel else {
        return false;
    };
    rel == output_rel || crate::pathutil::is_under_slash(rel, output_rel)
}

fn reset_output_dir(root: &Path, output_dir: &Path, layout: &PackLayout) -> Result<(), String> {
    let root_abs = root
        .canonicalize()
        .map_err(|err| format!("failed to resolve {}: {err}", root.display()))?;
    let output_abs = normalized_existing_or_future_path(output_dir)?;
    if output_abs == root_abs {
        return Err("refusing to prepare server pack into the pack root".to_string());
    }
    reject_pack_content_output_dir(root, &output_abs, layout)?;
    if output_abs.parent().is_none() {
        return Err(format!(
            "refusing to prepare server pack into filesystem root: {}",
            output_dir.display()
        ));
    }
    if output_dir.exists() {
        let metadata = fs::symlink_metadata(output_dir).map_err(|err| {
            format!(
                "failed to read metadata for {}: {err}",
                output_dir.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "refusing to remove symlink output path: {}",
                output_dir.display()
            ));
        }
        if !metadata.is_dir() {
            return Err(format!(
                "refusing to remove non-directory output path: {}",
                output_dir.display()
            ));
        }
        let stale = stale_output_dir_path(output_dir);
        fs::rename(output_dir, &stale).map_err(|err| {
            format!(
                "failed to stage old output directory {} for removal: {err}",
                output_dir.display()
            )
        })?;
        let metadata = fs::symlink_metadata(&stale)
            .map_err(|err| format!("failed to read metadata for {}: {err}", stale.display()))?;
        if metadata.file_type().is_symlink() {
            fs::remove_file(&stale)
                .map_err(|err| format!("failed to remove {}: {err}", stale.display()))?;
        } else {
            fs::remove_dir_all(&stale)
                .map_err(|err| format!("failed to remove {}: {err}", stale.display()))?;
        }
    }
    Ok(())
}

fn reject_pack_content_output_dir(
    root: &Path,
    output_abs: &Path,
    layout: &PackLayout,
) -> Result<(), String> {
    let root_abs = root
        .canonicalize()
        .map_err(|err| format!("failed to resolve {}: {err}", root.display()))?;
    let mut protected = BTreeSet::new();
    protected.insert(PathBuf::from("config"));
    protected.insert(layout.jar_root.clone());
    protected.insert(layout.server_meta.clone());
    protected.insert(layout.client_meta.clone());
    protected.insert(layout.common_meta.clone());
    protected.insert(layout.root_overlays.clone());
    for metadata_root in &layout.metadata_roots {
        protected.insert(metadata_root.clone());
    }

    for rel in protected {
        let rel = normalize_layout_path(&rel);
        if rel.is_empty() || rel == "." {
            continue;
        }
        let protected_abs = root_abs
            .join(&rel)
            .canonicalize()
            .unwrap_or_else(|_| root_abs.join(&rel));
        if output_abs == protected_abs || output_abs.starts_with(&protected_abs) {
            return Err(format!(
                "refusing to prepare server pack inside pack content directory: {}",
                protected_abs.display()
            ));
        }
    }
    Ok(())
}

fn stale_output_dir_path(output_dir: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let suffix = format!(
        "bkmpw-old-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    match output_dir.file_name() {
        Some(name) => output_dir.with_file_name(format!("{}.{}", name.to_string_lossy(), suffix)),
        None => PathBuf::from(suffix),
    }
}

fn temp_zip_path(output: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let suffix = format!(
        "bkmpw-tmp-{}-{}-{}",
        crate::tempfiles::run_id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos()),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    match output.file_name() {
        Some(name) => output.with_file_name(format!("{}.{}", name.to_string_lossy(), suffix)),
        None => PathBuf::from(suffix),
    }
}

fn replace_output(temp_output: &Path, output: &Path) -> Result<(), String> {
    match fs::rename(temp_output, output) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && output.exists() => {
            fs::remove_file(output)
                .map_err(|err| format!("failed to replace {}: {err}", output.display()))?;
            fs::rename(temp_output, output).map_err(|err| {
                format!(
                    "failed to move {} to {}: {err}",
                    temp_output.display(),
                    output.display()
                )
            })
        }
        Err(err) => Err(format!(
            "failed to move {} to {}: {err}",
            temp_output.display(),
            output.display()
        )),
    }
}

fn cleanup_temp_on_error<T>(result: Result<T, String>, temp_output: &Path) -> Result<T, String> {
    if result.is_err() {
        let _ = fs::remove_file(temp_output);
    }
    result
}

fn normalized_existing_or_future_path(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|err| format!("failed to resolve {}: {err}", path.display()));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .map_err(|err| format!("failed to resolve parent of {}: {err}", path.display()))?;
    let Some(name) = path.file_name() else {
        return Err(format!("invalid output path: {}", path.display()));
    };
    Ok(parent.join(name))
}

fn output_rel_in_root(root: &Path, output: &Path) -> Result<Option<String>, String> {
    let root_abs = root
        .canonicalize()
        .map_err(|err| format!("failed to resolve root {}: {err}", root.display()))?;
    let output_abs = match output.canonicalize() {
        Ok(path) => path,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let cwd = std::env::current_dir()
                .map_err(|err| format!("failed to get current directory: {err}"))?;
            let output = if output.is_absolute() {
                output.to_path_buf()
            } else {
                cwd.join(output)
            };
            let parent = output
                .parent()
                .ok_or_else(|| format!("output path has no parent: {}", output.display()))?;
            let parent_abs = parent
                .canonicalize()
                .map_err(|err| format!("failed to resolve {}: {err}", parent.display()))?;
            let name = output
                .file_name()
                .ok_or_else(|| format!("output path has no filename: {}", output.display()))?;
            parent_abs.join(name)
        }
        Err(err) => return Err(format!("failed to resolve {}: {err}", output.display())),
    };
    let rel = output_abs
        .strip_prefix(root_abs)
        .ok()
        .map(crate::pathutil::to_slash);
    if rel.is_none()
        && let Ok(raw) = output.strip_prefix(root)
    {
        return Ok(Some(crate::pathutil::to_slash(raw)));
    }
    Ok(rel)
}

fn reject_symlink(root: &Path, rel: &str) -> Result<(), String> {
    safe_slash_path(rel)?;
    let rel_path = Path::new(rel);
    let mut ancestors = rel_path.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        if ancestor.as_os_str().is_empty() || ancestor == Path::new(".") {
            continue;
        }
        let ancestor_rel = crate::pathutil::to_slash(ancestor);
        if ancestor_rel.is_empty() || ancestor_rel == rel {
            continue;
        }
        let path = join_slash(root, &ancestor_rel);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|err| format!("failed to read metadata for {}: {err}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("refusing to export symlink: {ancestor_rel}"));
        }
    }
    let path = join_slash(root, rel);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|err| format!("failed to read metadata for {}: {err}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("refusing to export symlink: {rel}"));
    }
    Ok(())
}

fn reject_path_symlink(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to read metadata for {}: {err}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("refusing to export symlink: {}", path.display()));
    }
    Ok(())
}

fn default_client_root_name(root: &Path) -> Result<String, String> {
    let info = PackInfo::load(root)?;
    let name = info.name.unwrap_or_else(|| "minecraft-pack".to_string());
    safe_filename(&name)
}

fn install_server_bat() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal\r\n",
        "cd /d \"%~dp0\"\r\n",
        "set \"JOBS=%~1\"\r\n",
        "if \"%JOBS%\"==\"\" set \"JOBS=8\"\r\n",
        "powershell -NoProfile -ExecutionPolicy Bypass -Command \"$ErrorActionPreference='Stop'; ",
        "$repo='",
        "bro-know-my-org/BroKnowMyPackwiz",
        "'; ",
        "$api='https://api.github.com/repos/'+$repo+'/releases/latest'; ",
        "$release=Invoke-RestMethod -Headers @{'User-Agent'='bkmpw-installer'} -Uri $api; ",
        "$tag=$release.tag_name; ",
        "$asset='bkmpw-'+$tag+'-x86_64-pc-windows-msvc.exe'; ",
        "$dir='tools'; New-Item -ItemType Directory -Force -Path $dir | Out-Null; ",
        "$bin=Join-Path $dir 'bkmpw.exe'; $sum=$bin+'.sha256'; ",
        "$base='https://github.com/'+$repo+'/releases/download/'+$tag+'/'; ",
        "Invoke-WebRequest -UseBasicParsing -Uri ($base+$asset) -OutFile $bin; ",
        "Invoke-WebRequest -UseBasicParsing -Uri ($base+$asset+'.sha256') -OutFile $sum; ",
        "$expected=(Get-Content -Raw $sum).Split()[0]; ",
        "$actual=(Get-FileHash -Algorithm SHA256 $bin).Hash.ToLowerInvariant(); ",
        "if ($actual -ne $expected.ToLowerInvariant()) { throw 'bkmpw checksum mismatch' }\"\r\n",
        "if errorlevel 1 exit /b %errorlevel%\r\n",
        "tools\\bkmpw.exe install-local . . server %JOBS% --retries 3 --retry-delay-seconds 5\r\n",
        "exit /b %errorlevel%\r\n",
    )
}

fn install_server_sh() -> &'static str {
    concat!(
        "#!/usr/bin/env sh\n",
        "set -eu\n",
        "cd \"$(dirname \"$0\")\"\n",
        "JOBS=\"${1:-8}\"\n",
        "REPO=\"",
        "bro-know-my-org/BroKnowMyPackwiz",
        "\"\n",
        "OS=\"$(uname -s)\"\n",
        "ARCH=\"$(uname -m)\"\n",
        "case \"$OS/$ARCH\" in\n",
        "  Linux/x86_64) TARGET=\"x86_64-unknown-linux-gnu\" ;;\n",
        "  Darwin/x86_64) TARGET=\"x86_64-apple-darwin\" ;;\n",
        "  Darwin/arm64|Darwin/aarch64) TARGET=\"aarch64-apple-darwin\" ;;\n",
        "  *) echo \"unsupported platform: $OS/$ARCH\" >&2; exit 1 ;;\n",
        "esac\n",
        "TAG=\"$(curl -fsSL -H 'User-Agent: bkmpw-installer' \"https://api.github.com/repos/$REPO/releases/latest\" | sed -n 's/.*\"tag_name\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p' | head -n 1)\"\n",
        "if [ -z \"$TAG\" ]; then echo \"failed to resolve latest bkmpw release\" >&2; exit 1; fi\n",
        "ASSET=\"bkmpw-$TAG-$TARGET\"\n",
        "BASE=\"https://github.com/$REPO/releases/download/$TAG\"\n",
        "mkdir -p tools\n",
        "curl -fsSL \"$BASE/$ASSET\" -o tools/bkmpw\n",
        "curl -fsSL \"$BASE/$ASSET.sha256\" -o tools/bkmpw.sha256\n",
        "EXPECTED=\"$(awk '{print $1}' tools/bkmpw.sha256)\"\n",
        "if command -v sha256sum >/dev/null 2>&1; then ACTUAL=\"$(sha256sum tools/bkmpw | awk '{print $1}')\"; else ACTUAL=\"$(shasum -a 256 tools/bkmpw | awk '{print $1}')\"; fi\n",
        "if [ \"$(printf '%s' \"$ACTUAL\" | tr 'A-F' 'a-f')\" != \"$(printf '%s' \"$EXPECTED\" | tr 'A-F' 'a-f')\" ]; then echo \"bkmpw checksum mismatch\" >&2; exit 1; fi\n",
        "chmod +x ./tools/bkmpw\n",
        "./tools/bkmpw install-local . . server \"$JOBS\" --retries 3 --retry-delay-seconds 5\n",
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;

    #[test]
    fn export_server_writes_direct_server_pack() {
        let root = unique_test_dir("bkmpw-export-server");
        create_pack(&root);
        fs::create_dir_all(root.join("mods/client")).unwrap();
        fs::create_dir_all(root.join("mods/common")).unwrap();
        fs::create_dir_all(root.join("config")).unwrap();
        fs::create_dir_all(root.join("resourcepacks")).unwrap();
        fs::write(root.join("mods/server.jar"), b"server").unwrap();
        fs::write(root.join("mods/client.jar"), b"client").unwrap();
        fs::write(root.join("config/server.toml"), b"server-config").unwrap();
        fs::write(root.join("config/client-client.toml"), b"client-config").unwrap();
        fs::write(root.join("resourcepacks/client.zip"), b"client-resource").unwrap();
        fs::write(
            root.join("mods/common/server.pw.toml"),
            "filename = \"server.jar\"\n",
        )
        .unwrap();
        fs::write(
            root.join("mods/client/client.pw.toml"),
            "filename = \"client.jar\"\n",
        )
        .unwrap();

        let output = root.join("server-pack.zip");
        let count = export_server(&root, &output).unwrap();
        let bytes = fs::read(&output).unwrap();
        let zip_text = String::from_utf8_lossy(&bytes);

        assert!(count > 0);
        assert!(zip_text.contains("mods/server.jar"));
        assert!(zip_text.contains("config/server.toml"));
        assert!(!zip_text.contains("manifest.json"));
        assert!(!zip_text.contains("overrides/"));
        assert!(!zip_text.contains("mods/client.jar"));
        assert!(!zip_text.contains("mods/common/server.pw.toml"));
        assert!(zip_text.contains("config/client-client.toml"));
        assert!(!zip_text.contains("resourcepacks/client.zip"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_server_applies_common_and_server_root_overlays() {
        let root = unique_test_dir("bkmpw-export-server-overlays");
        create_pack(&root);
        fs::create_dir_all(root.join("roots/common/config")).unwrap();
        fs::create_dir_all(root.join("roots/server/config")).unwrap();
        fs::create_dir_all(root.join("roots/client")).unwrap();
        fs::write(root.join("roots/common/icon.png"), b"icon").unwrap();
        fs::write(root.join("roots/common/config/shared.toml"), b"base").unwrap();
        fs::write(root.join("roots/server/config/shared.toml"), b"server").unwrap();
        fs::write(root.join("roots/server/start.bat"), b"start").unwrap();
        fs::write(root.join("roots/client/options.txt"), b"client").unwrap();

        let output = root.join("server-pack.zip");
        export_server(&root, &output).unwrap();
        let bytes = fs::read(&output).unwrap();
        let zip_text = String::from_utf8_lossy(&bytes);

        assert!(zip_text.contains("icon.png"));
        assert!(zip_text.contains("config/shared.toml"));
        assert!(zip_text.contains("server"));
        assert!(!zip_text.contains("base"));
        assert!(zip_text.contains("start.bat"));
        assert!(!zip_text.contains("options.txt"));
        assert!(!zip_text.contains("roots/common"));
        assert!(!zip_text.contains("roots/server"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_server_fails_when_required_jar_is_missing() {
        let root = unique_test_dir("bkmpw-export-server-missing-jar");
        create_pack(&root);
        fs::create_dir_all(root.join("mods/common")).unwrap();
        fs::write(
            root.join("mods/common/server.pw.toml"),
            "filename = \"missing.jar\"\n",
        )
        .unwrap();

        let err = export_server(&root, &root.join("server-pack.zip")).unwrap_err();

        assert!(err.contains("missing required file"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_client_wraps_full_pack_in_root_directory() {
        let root = unique_test_dir("bkmpw-export-client");
        create_pack(&root);
        fs::create_dir_all(root.join("mods/client")).unwrap();
        fs::create_dir_all(root.join("mods/server")).unwrap();
        fs::create_dir_all(root.join("resourcepacks")).unwrap();
        fs::write(root.join("mods/client.jar"), b"client").unwrap();
        fs::write(root.join("mods/server.jar"), b"server").unwrap();
        fs::write(root.join("resourcepacks/client.zip"), b"resource").unwrap();
        fs::write(
            root.join("mods/client/client.pw.toml"),
            "filename = \"client.jar\"\n",
        )
        .unwrap();
        fs::write(
            root.join("mods/server/server.pw.toml"),
            "filename = \"server.jar\"\n",
        )
        .unwrap();

        let output = root.join("client-full.zip");
        export_client(&root, &output, Some("PackRoot")).unwrap();
        let bytes = fs::read(&output).unwrap();
        let zip_text = String::from_utf8_lossy(&bytes);

        assert!(zip_text.contains("PackRoot/mods/client.jar"));
        assert!(zip_text.contains("PackRoot/resourcepacks/client.zip"));
        assert!(!zip_text.contains("PackRoot/mods/server.jar"));
        assert!(!zip_text.contains("manifest.json"));
        assert!(!zip_text.contains("overrides/"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_client_applies_common_and_client_root_overlays_inside_root_dir() {
        let root = unique_test_dir("bkmpw-export-client-overlays");
        create_pack(&root);
        fs::create_dir_all(root.join("roots/common")).unwrap();
        fs::create_dir_all(root.join("roots/client")).unwrap();
        fs::create_dir_all(root.join("roots/server")).unwrap();
        fs::write(root.join("roots/common/icon.png"), b"common-icon").unwrap();
        fs::write(root.join("roots/client/options.txt"), b"client").unwrap();
        fs::write(root.join("roots/server/start.sh"), b"server").unwrap();

        let output = root.join("client-full.zip");
        export_client(&root, &output, Some("PackRoot")).unwrap();
        let bytes = fs::read(&output).unwrap();
        let zip_text = String::from_utf8_lossy(&bytes);

        assert!(zip_text.contains("PackRoot/icon.png"));
        assert!(zip_text.contains("PackRoot/options.txt"));
        assert!(!zip_text.contains("PackRoot/start.sh"));
        assert!(!zip_text.contains("PackRoot/roots/common"));
        assert!(!zip_text.contains("PackRoot/roots/client"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_server_installer_contains_metadata_and_runner_without_runtime_jar() {
        let root = unique_test_dir("bkmpw-export-server-installer");
        create_pack(&root);
        fs::create_dir_all(root.join("mods/common")).unwrap();
        fs::write(root.join("mods/server.jar"), b"server").unwrap();
        fs::write(root.join("mods/manual.jar"), b"manual").unwrap();
        fs::write(
            root.join("mods/common/server.pw.toml"),
            "filename = \"server.jar\"\n",
        )
        .unwrap();

        let output = root.join("server-installer.zip");
        export_server_installer(&root, &output).unwrap();
        let bytes = fs::read(&output).unwrap();
        let zip_text = String::from_utf8_lossy(&bytes);

        assert!(zip_text.contains("install-server.bat"));
        assert!(zip_text.contains("install-server.sh"));
        assert!(zip_text.contains("releases/latest"));
        assert!(!zip_text.contains("tools/bkmpw.exe"));
        assert!(zip_text.contains("mods/common/server.pw.toml"));
        assert!(zip_text.contains("pack.toml"));
        assert!(!zip_text.contains("mods/server.jar"));
        assert!(!zip_text.contains("mods/manual.jar"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn secondary_metadata_roots_are_client_only_without_hardcoded_names() {
        let layout = PackLayout {
            metadata_root: PathBuf::from("mods"),
            metadata_roots: vec![PathBuf::from("mods"), PathBuf::from("packs")],
            jar_root: PathBuf::from("mods"),
            server_meta: PathBuf::from("mods/server"),
            client_meta: PathBuf::from("mods/client"),
            common_meta: PathBuf::from("mods/common"),
            root_overlays: PathBuf::from("roots"),
            metadata_extension: "pw.toml".to_string(),
        };

        assert_eq!(
            server_entry_side("packs/client-only.zip", &layout),
            Side::Client
        );
    }

    #[test]
    fn prepare_server_replaces_existing_output_dir() {
        let root = unique_test_dir("bkmpw-prepare-server");
        create_pack(&root);
        fs::create_dir_all(root.join("mods/common")).unwrap();
        fs::write(root.join("mods/server.jar"), b"server").unwrap();
        fs::write(
            root.join("mods/common/server.pw.toml"),
            "filename = \"server.jar\"\n",
        )
        .unwrap();
        let output = root.join(".bkmpw").join("server-pack");
        fs::create_dir_all(output.join("old")).unwrap();
        fs::write(output.join("old").join("stale.txt"), b"stale").unwrap();

        let count = prepare_server(&root, &output).unwrap();

        assert!(count > 0);
        assert!(output.join("mods").join("server.jar").is_file());
        assert!(!output.join("old").join("stale.txt").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prepare_server_refuses_pack_root_output() {
        let root = unique_test_dir("bkmpw-prepare-server-root");
        create_pack(&root);

        let err = prepare_server(&root, &root).unwrap_err();

        assert!(err.contains("pack root"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prepare_server_refuses_pack_content_output() {
        let root = unique_test_dir("bkmpw-prepare-server-content-dir");
        create_pack(&root);

        let err = prepare_server(&root, &root.join("mods")).unwrap_err();

        assert!(err.contains("pack content directory"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_server_rejects_symlink_root() {
        let root = unique_test_dir("bkmpw-export-server-root-target");
        let link_parent = unique_test_dir("bkmpw-export-server-root-link-parent");
        let link = link_parent.join("pack-link");
        create_pack(&root);
        if create_dir_symlink(&root, &link).is_err() {
            let _ = fs::remove_dir_all(root);
            let _ = fs::remove_dir_all(link_parent);
            return;
        }

        let err = export_server(&link, &link_parent.join("server-pack.zip")).unwrap_err();

        assert!(err.contains("symlink"));

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(link_parent);
    }

    #[test]
    fn export_server_allows_symlink_parent_outside_pack_root() {
        let real_parent = unique_test_dir("bkmpw-export-server-real-parent");
        let link_parent = unique_test_dir("bkmpw-export-server-linked-parent");
        let parent_link = link_parent.join("parent-link");
        if create_dir_symlink(&real_parent, &parent_link).is_err() {
            let _ = fs::remove_dir_all(real_parent);
            let _ = fs::remove_dir_all(link_parent);
            return;
        }
        let root = parent_link.join("pack");
        create_pack(&root);
        fs::create_dir_all(root.join("mods/common")).unwrap();
        fs::write(root.join("mods/server.jar"), b"server").unwrap();
        fs::write(
            root.join("mods/common/server.pw.toml"),
            "filename = \"server.jar\"\n",
        )
        .unwrap();

        let count = export_server(&root, &root.join("server-pack.zip")).unwrap();

        assert!(count > 0);

        let _ = fs::remove_dir_all(real_parent);
        let _ = fs::remove_dir_all(link_parent);
    }

    #[test]
    fn export_server_removes_temp_zip_after_failure() {
        let root = unique_test_dir("bkmpw-export-server-partial-failure");
        let outside = unique_test_dir("bkmpw-export-server-partial-outside");
        create_pack(&root);
        fs::write(outside.join("server.jar"), b"server").unwrap();
        if create_dir_symlink(&outside, &root.join("modslink")).is_err() {
            let _ = fs::remove_dir_all(root);
            let _ = fs::remove_dir_all(outside);
            return;
        }
        fs::create_dir_all(root.join("mods/common")).unwrap();
        fs::write(
            root.join("mods/common/server.pw.toml"),
            "filename = \"modslink/server.jar\"\n",
        )
        .unwrap();
        let output = root.join("server-pack.zip");

        let err = export_server(&root, &output).unwrap_err();

        assert!(err.contains("symlink"));
        assert!(!output.exists());

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn reject_symlink_rejects_symlink_parent_directory() {
        let root = unique_test_dir("bkmpw-export-server-symlink-parent");
        let outside = unique_test_dir("bkmpw-export-server-symlink-outside");
        fs::write(outside.join("server.jar"), b"server").unwrap();
        if create_dir_symlink(&outside, &root.join("modslink")).is_err() {
            let _ = fs::remove_dir_all(root);
            let _ = fs::remove_dir_all(outside);
            return;
        }

        let err = reject_symlink(&root, "modslink/server.jar").unwrap_err();

        assert!(err.contains("symlink"));

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    fn create_pack(root: &Path) {
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join(".packwizignore"), "").unwrap();
        fs::write(
            root.join("pack.toml"),
            "name = \"Test Pack\"\n\
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
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[cfg(unix)]
    fn create_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }
}
