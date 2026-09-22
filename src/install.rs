use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::config::ProjectConfig;
use crate::curseforge;
use crate::http::{DownloadProgress, HttpDownloadOptions, http_get_to_file_with_options};
use crate::layout::PackLayout;
use crate::metadata::{ModMetadata, Side};
use crate::pathutil::join_slash;
use crate::progress::ProgressRenderer;
use crate::scan::ScanReport;
use crate::sha1::sha1_file_hex;
use crate::sha256::sha256_file_hex;
use crate::sha512::sha512_file_hex;
use crate::tempfiles;

const STALE_TEMP_AFTER: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone)]
pub struct InstallResult {
    pub installed: usize,
    pub skipped: usize,
    pub removed: usize,
    pub temp_removed: usize,
    pub warnings: Vec<String>,
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
    pub split_download_min_bytes: u64,
    pub split_download_chunks: usize,
}

#[derive(Debug, Clone)]
struct CopyTask {
    name: String,
    source: Option<PathBuf>,
    url: Option<String>,
    curseforge: Option<CurseForgeDownload>,
    target: PathBuf,
    target_root: PathBuf,
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
    let _run_guard = tempfiles::start_run(target_root)?;
    let temp_cleanup = cleanup_stale_temp_files(target_root, &layout, STALE_TEMP_AFTER);
    let report = ScanReport::build(source_root, &config, &layout)?;
    let curseforge_api_key = curseforge::api_key(&config);
    let mut tasks = VecDeque::new();
    let mut target_paths = BTreeSet::new();
    let mut skipped = 0;
    let cleanup = options.cleanup;
    // The manifest marks which existing targets are managed by us, so it is
    // needed even without cleanup to allow replacing managed files.
    let mut warnings = temp_cleanup.warnings;
    let old_manifest = read_manifest_paths(target_root, &mut warnings)?;
    // Canonical once: per-file symlink checks resolve against this root.
    let target_root_canonical = canonical_prefix(target_root)?;

    if cleanup && report.metadata.is_empty() {
        return Err(
            "refusing to clean managed files because no metadata files were scanned".to_string(),
        );
    }

    let mut plan_errors = Vec::new();
    for entry in report.metadata {
        let metadata_path = join_slash(source_root, &entry.path);
        let metadata = match ModMetadata::load(&metadata_path) {
            Ok(metadata) => metadata,
            Err(err) => {
                plan_errors.push(err);
                continue;
            }
        };
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
            target_root: target_root_canonical.clone(),
            managed_target: old_manifest.contains(&file_target),
            target_rel: file_target,
            expected_hash,
            preserve: metadata.preserve,
            force: options.force,
        });
    }

    let (installed_files, mut errors) = run_copy_tasks(tasks, options);
    errors.extend(plan_errors);
    let removed = if errors.is_empty() && cleanup {
        cleanup_removed_files(
            &target_root_canonical,
            &layout,
            &old_manifest,
            &installed_files,
            &mut warnings,
        )?
    } else {
        0
    };
    if errors.is_empty() {
        write_manifest(target_root, target_side, &installed_files)?;
    }
    Ok(InstallResult {
        installed: installed_files.len(),
        skipped,
        removed,
        temp_removed: temp_cleanup.removed,
        warnings,
        errors,
    })
}

pub(crate) fn side_from_directory_or_metadata(
    hint: crate::scan::SideHint,
    metadata: &ModMetadata,
) -> Side {
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
    resolve_pack_file_path_operation(metadata_path, filename, layout).map_err(|error| error.detail)
}

pub(crate) fn resolve_pack_file_path_operation(
    metadata_path: &str,
    filename: &str,
    layout: &PackLayout,
) -> crate::operation::Result<String> {
    let target = if filename.contains('/') {
        crate::operation::paths::relative(filename)?
    } else {
        let filename = crate::operation::paths::filename(filename)?;
        if is_side_metadata_path(metadata_path, layout) {
            crate::operation::paths::relative(&format!(
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
                crate::operation::paths::relative(&format!("{parent}/{filename}"))?
            }
        }
    };

    if is_managed_pack_file_path(&target, layout) {
        Ok(target)
    } else {
        Err(crate::operation::Error::named(
            crate::operation::ErrorCode::Failed,
            "managed_root_escape",
            format!("metadata filename resolves outside managed roots: {filename}"),
        )
        .context(filename))
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
    let total = tasks.len();
    let tasks = Arc::new(Mutex::new(tasks));
    let errors = Arc::new(Mutex::new(Vec::new()));
    let installed = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::new(Mutex::new(()));
    let jobs = options.jobs.max(1);
    let progress = ProgressRenderer::start(jobs, total);

    thread::scope(|scope| {
        for worker_idx in 0..jobs {
            let tasks = Arc::clone(&tasks);
            let errors = Arc::clone(&errors);
            let installed = Arc::clone(&installed);
            let log = Arc::clone(&log);
            let options = options.clone();
            let progress = progress.as_ref();
            scope.spawn(move || {
                loop {
                    let task = {
                        let mut guard = tasks.lock().expect("copy task mutex poisoned");
                        guard.pop_front()
                    };
                    let Some(task) = task else {
                        break;
                    };
                    let task_progress = if let Some(progress) = progress {
                        Some(progress.slot_progress(worker_idx, &task.target_rel))
                    } else {
                        log_install(
                            &log,
                            format!("processing {} ({})", task.target_rel, task_source(&task)),
                        );
                        None
                    };
                    match copy_with_retries(&task, &options, &log, task_progress.clone()) {
                        Ok(outcome) => {
                            if let Some(progress) = progress {
                                progress.finish_slot(worker_idx, false);
                            } else {
                                log_install(&log, format!("done {}", task.target_rel));
                            }
                            let mut guard = installed.lock().expect("installed mutex poisoned");
                            guard.push(InstalledFile {
                                name: task.name,
                                path: task.target_rel,
                                sha256: outcome.sha256,
                                record: outcome.record,
                            });
                        }
                        Err(err) => {
                            let err =
                                format!("{} ({}): {err}", task.target_rel, task_source(&task));
                            if let Some(progress) = progress {
                                progress.finish_slot(worker_idx, true);
                                progress.log(&format!("failed {err}"));
                                progress.clear_slot(worker_idx);
                            } else {
                                log_install(&log, format!("failed {err}"));
                            }
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

fn copy_with_retries(
    task: &CopyTask,
    options: &InstallOptions,
    log: &Mutex<()>,
    progress: Option<Arc<dyn DownloadProgress>>,
) -> Result<CopyOutcome, String> {
    let attempts = options.retries.max(1);
    let mut last_error = None;
    for attempt in 1..=attempts {
        if let Some(progress) = &progress {
            progress.reset();
        }
        match copy_atomic(task, options, progress.clone()) {
            Ok(outcome) => return Ok(outcome),
            Err(err) => {
                if err.permanent {
                    return Err(err.message);
                }
                if progress.is_none() {
                    log_install(
                        log,
                        format!(
                            "failed attempt {attempt}/{attempts} for {} ({}): {}",
                            task.target_rel,
                            task_source(task),
                            err.message
                        ),
                    );
                }
                last_error = Some(err.message);
                if attempt < attempts && options.retry_delay_seconds > 0 {
                    thread::sleep(Duration::from_secs(options.retry_delay_seconds));
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| format!("failed to install {}", task.name)))
}

fn log_install(log: &Mutex<()>, message: String) {
    let _guard = log.lock().unwrap_or_else(|err| err.into_inner());
    eprintln!("{message}");
}

fn task_source(task: &CopyTask) -> String {
    if let Some(source) = task.source.as_ref().filter(|source| source.exists()) {
        if same_path(source, &task.target) {
            return "local (source equals target)".to_string();
        }
        return format!("local {}", source.display());
    }
    if let Some(url) = &task.url {
        return format!("url {url}");
    }
    if let Some(curseforge) = &task.curseforge {
        return format!(
            "curseforge project {} file {}",
            curseforge.project_id, curseforge.file_id
        );
    }
    "missing source".to_string()
}

#[derive(Debug)]
struct CopyError {
    message: String,
    /// Deterministic failures that can never succeed on retry.
    permanent: bool,
}

impl CopyError {
    fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            permanent: true,
        }
    }
    fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            permanent: false,
        }
    }
}

#[derive(Debug)]
struct CopyOutcome {
    sha256: String,
    /// False when the existing file was preserved rather than written by us;
    /// such targets must not become managed in the manifest.
    record: bool,
}

fn copy_atomic(
    task: &CopyTask,
    options: &InstallOptions,
    progress: Option<Arc<dyn DownloadProgress>>,
) -> Result<CopyOutcome, CopyError> {
    require_within_root(task).map_err(CopyError::permanent)?;
    let preserve_existing = options.preserve_existing && !task.managed_target;
    if !task.force
        && let Some(expected) = &task.expected_hash
        && task.target.exists()
    {
        if file_hash_hex(&task.target, &expected.format).as_ref() == Ok(&expected.value) {
            return Ok(CopyOutcome {
                sha256: sha256_file_hex(&task.target).map_err(CopyError::transient)?,
                // Content matched without us writing it; only files that were
                // already managed or are free to adopt become managed.
                record: task.managed_target || (!task.preserve && !preserve_existing),
            });
        }
        if preserve_existing || task.preserve {
            return Err(CopyError::permanent(format!(
                "existing file hash mismatch for {}; use --force to replace it",
                task.target.display()
            )));
        }
    }
    if !task.force && (task.preserve || preserve_existing) && task.target.exists() {
        return Ok(CopyOutcome {
            sha256: sha256_file_hex(&task.target).map_err(CopyError::transient)?,
            // Preserved files keep whatever managed status they already had:
            // a managed file stays managed, a manual file stays unmanaged.
            record: task.managed_target,
        });
    }
    if !task.force && task.target.exists() && !task.managed_target {
        return Err(CopyError::permanent(format!(
            "refusing to overwrite unmanaged file: {}",
            task.target.display()
        )));
    }
    if let Some(parent) = task.target.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            CopyError::transient(format!("failed to create {}: {err}", parent.display()))
        })?;
    }

    let tmp = unique_tmp_path(&task.target);
    if let Some(source) = valid_local_source(task).map_err(CopyError::transient)? {
        let source_size = fs::metadata(source)
            .map_err(|err| {
                CopyError::transient(format!("failed to stat {}: {err}", source.display()))
            })?
            .len();
        if let Some(progress) = &progress {
            progress.set_total(source_size);
        }
        let copied = fs::copy(source, &tmp).map_err(|err| {
            CopyError::transient(format!(
                "failed to copy {} to {}: {err}",
                source.display(),
                tmp.display()
            ))
        })?;
        if let Some(progress) = &progress {
            progress.add_bytes(copied);
        }
    } else if let Some(url) = &task.url {
        http_get_to_file_with_options(url, &tmp, &http_download_options(options, progress))
            .map_err(CopyError::transient)?;
    } else if let Some(curseforge) = &task.curseforge {
        download_curseforge(curseforge, &tmp, &http_download_options(options, progress))
            .map_err(CopyError::transient)?;
    } else {
        return Err(CopyError::permanent(format!(
            "missing source file for {}",
            task.name
        )));
    }
    if let Some(expected) = &task.expected_hash {
        let actual = file_hash_hex(&tmp, &expected.format).map_err(CopyError::transient)?;
        if actual != expected.value {
            let _ = fs::remove_file(&tmp);
            // A bad download might succeed on retry (CDN propagation), so
            // this stays transient.
            return Err(CopyError::transient(format!(
                "hash mismatch for {}: expected {} {}, got {}",
                task.name, expected.format, expected.value, actual
            )));
        }
    }
    replace_with_tmp(&tmp, &task.target).map_err(CopyError::transient)?;
    Ok(CopyOutcome {
        sha256: sha256_file_hex(&task.target).map_err(CopyError::transient)?,
        record: true,
    })
}

// Canonicalizes the deepest existing ancestor of `path` and reattaches the
// remaining components, so symlinked directories in the target root are
// detected even when the final file does not exist yet. The reattached suffix
// is compared lexically — this is only safe because callers pass paths that
// already went through `safe_slash_path` (no `..` components). This is a
// check-then-act guard: it prevents persistent symlinked roots, but a
// concurrently swapped symlink could still escape; a local CLI accepts that.
fn canonical_prefix(path: &Path) -> Result<PathBuf, String> {
    let mut suffix = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        match current.canonicalize() {
            Ok(mut resolved) => {
                for part in suffix.iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = current.file_name() else {
                    return Err(format!("failed to resolve {}: {err}", path.display()));
                };
                suffix.push(name.to_os_string());
                if !current.pop() {
                    return Err(format!("failed to resolve {}: {err}", path.display()));
                }
            }
            Err(err) => {
                return Err(format!("failed to resolve {}: {err}", path.display()));
            }
        }
    }
}

fn path_within_root(canonical_root: &Path, path: &Path) -> Result<bool, String> {
    Ok(canonical_prefix(path)?.starts_with(canonical_root))
}

fn require_within_root(task: &CopyTask) -> Result<(), String> {
    if path_within_root(&task.target_root, &task.target)? {
        Ok(())
    } else {
        Err(format!(
            "target {} resolves outside the target root (possible symlink)",
            task.target.display()
        ))
    }
}

fn replace_with_tmp(tmp: &Path, target: &Path) -> Result<(), String> {
    match fs::rename(tmp, target) {
        Ok(()) => Ok(()),
        Err(first_err) if target.is_dir() => Err(format!(
            "failed to replace {}: {first_err} (target is a directory)",
            target.display()
        )),
        Err(first_err) if target.exists() => {
            // Move the old file aside first so a failed retry restores it.
            let backup = unique_tmp_path(target);
            fs::rename(target, &backup)
                .map_err(|err| format!("failed to move aside {}: {err}", target.display()))?;
            match fs::rename(tmp, target) {
                Ok(()) => {
                    let _ = fs::remove_file(&backup);
                    Ok(())
                }
                Err(err) => match fs::rename(&backup, target) {
                    Ok(()) => Err(format!(
                        "failed to replace {} after moving old file aside: {err}; initial rename error: {first_err}",
                        target.display()
                    )),
                    Err(restore_err) => Err(format!(
                        "failed to replace {} after moving old file aside: {err}; initial rename error: {first_err}; restore failed: {restore_err}; original preserved at {}",
                        target.display(),
                        backup.display()
                    )),
                },
            }
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
    extension.push_str(tempfiles::install_temp_marker());
    extension.push_str(tempfiles::run_id());
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

fn http_download_options(
    options: &InstallOptions,
    progress: Option<Arc<dyn DownloadProgress>>,
) -> HttpDownloadOptions {
    HttpDownloadOptions {
        split_min_bytes: options.split_download_min_bytes,
        split_chunks: options.split_download_chunks,
        progress,
    }
}

fn download_curseforge(
    curseforge: &CurseForgeDownload,
    tmp: &Path,
    download_options: &HttpDownloadOptions,
) -> Result<(), String> {
    let mut cdn_error = None;
    if curseforge.cdn_fallback
        && let Some(url) = curseforge::cdn_download_url(curseforge.file_id, &curseforge.filename)
    {
        match http_get_to_file_with_options(&url, tmp, download_options) {
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
        return http_get_to_file_with_options(&url, tmp, download_options);
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
    record: bool,
}

fn cleanup_removed_files(
    root: &Path,
    layout: &PackLayout,
    old_paths: &BTreeSet<String>,
    new_files: &[InstalledFile],
    warnings: &mut Vec<String>,
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
        if !is_safe_manifest_path(path, layout) {
            continue;
        }
        let full_path = join_slash(root, path);
        match path_within_root(root, &full_path) {
            Ok(true) => {}
            Ok(false) => {
                warnings.push(format!(
                    "skipping cleanup of {}: path resolves outside the target root (possible symlink)",
                    full_path.display()
                ));
                continue;
            }
            Err(err) => {
                warnings.push(format!(
                    "skipping cleanup of {}: failed to resolve path ({err})",
                    full_path.display()
                ));
                continue;
            }
        }
        if full_path.is_file() {
            fs::remove_file(&full_path)
                .map_err(|err| format!("failed to remove {}: {err}", full_path.display()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn cleanup_stale_temp_files(
    root: &Path,
    layout: &PackLayout,
    stale_after: Duration,
) -> tempfiles::StaleTempCleanup {
    let mut managed_roots = layout.metadata_roots.clone();
    managed_roots.push(layout.jar_root.clone());
    tempfiles::cleanup_stale_temp_files(root, &managed_roots, stale_after)
}

fn is_safe_manifest_path(path: &str, layout: &PackLayout) -> bool {
    let Ok(normalized) = crate::pathutil::safe_slash_path(path) else {
        return false;
    };
    is_managed_pack_file_path(&normalized, layout)
}

pub(crate) fn read_manifest_paths(
    root: &Path,
    warnings: &mut Vec<String>,
) -> Result<BTreeSet<String>, String> {
    let path = root.join("packwiz.json");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeSet::new());
        }
        Err(err) => {
            return Err(format!("failed to read {}: {err}", path.display()));
        }
    };
    // The manifest gates both overwrite permission and deletion; a corrupt
    // manifest silently downgrades managed files to unmanaged, so warn.
    let manifest: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(err) => {
            warnings.push(format!(
                "ignoring malformed {}: {err}; managed files will be treated as unmanaged",
                path.display()
            ));
            return Ok(BTreeSet::new());
        }
    };
    if manifest.get("format").and_then(|v| v.as_str()) != Some("bkmpw:1") {
        return Ok(BTreeSet::new());
    }
    let mut paths = BTreeSet::new();
    match manifest.get("files").and_then(|v| v.as_array()) {
        Some(files) => {
            for file in files {
                if let Some(path) = file.get("path").and_then(|v| v.as_str()) {
                    paths.insert(crate::pathutil::normalize_slash(path));
                }
            }
        }
        None => warnings.push(format!(
            "{} has format bkmpw:1 but no files array; managed files will be treated as unmanaged",
            path.display()
        )),
    }
    Ok(paths)
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
    let recorded: Vec<_> = files.iter().filter(|file| file.record).collect();
    for (idx, item) in recorded.iter().enumerate() {
        let comma = if idx + 1 == recorded.len() { "" } else { "," };
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
            root_overlays: PathBuf::from("roots"),
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
        let client_meta = ModMetadata::parse("side = \"server\"\n").unwrap();
        let server_meta = ModMetadata::parse("side = \"client\"\n").unwrap();
        let common_meta = ModMetadata::parse("side = \"server\"\n").unwrap();

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
            target_root: root.canonicalize().unwrap(),
            target_rel: "mods/target.jar".to_string(),
            expected_hash: None,
            preserve: true,
            force: true,
            managed_target: true,
        };

        copy_atomic(&task, &test_install_options(), None).unwrap();
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
            target_root: root.canonicalize().unwrap(),
            target_rel: "mods/target.jar".to_string(),
            expected_hash: Some(ExpectedHash {
                format: "sha512".to_string(),
                value: expected,
            }),
            preserve: false,
            force: false,
            managed_target: false,
        };

        copy_atomic(&task, &test_install_options(), None).unwrap();
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
            target_root: root.canonicalize().unwrap(),
            target_rel: "mods/target.jar".to_string(),
            expected_hash: Some(ExpectedHash {
                format: "sha256".to_string(),
                value: expected,
            }),
            preserve: false,
            force: false,
            managed_target: false,
        };

        copy_atomic(&task, &test_install_options(), None).unwrap();
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
            target_root: root.canonicalize().unwrap(),
            target_rel: "mods/target.jar".to_string(),
            expected_hash: Some(ExpectedHash {
                format: "sha256".to_string(),
                value: expected,
            }),
            preserve: false,
            force: false,
            managed_target: false,
        };

        let mut options = test_install_options();
        options.preserve_existing = true;
        let err = copy_atomic(&task, &options, None).unwrap_err();
        assert!(err.message.contains("existing file hash mismatch"));
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

        let removed = cleanup_removed_files(
            &root.canonicalize().unwrap(),
            &layout(),
            &old,
            &[],
            &mut Vec::new(),
        )
        .unwrap();

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

        let paths = read_manifest_paths(&root, &mut Vec::new()).unwrap();

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

        let paths = read_manifest_paths(&root, &mut Vec::new()).unwrap();

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

        let paths = read_manifest_paths(&root, &mut Vec::new()).unwrap();

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
            target_root: root.canonicalize().unwrap(),
            target_rel: "mods/target.jar".to_string(),
            expected_hash: None,
            preserve: false,
            force: false,
            managed_target: false,
        };

        assert!(copy_atomic(&task, &test_install_options(), None).is_err());
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
                split_download_min_bytes: 16 * 1024 * 1024,
                split_download_chunks: 4,
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
    fn task_source_reports_url_when_local_source_is_missing() {
        let root = temp_root("task-source-reports-url");
        let task = CopyTask {
            name: "remote".to_string(),
            source: Some(root.join("mods").join("remote.jar")),
            url: Some("https://example.com/remote.jar".to_string()),
            curseforge: None,
            target: root.join("mods").join("remote.jar"),
            target_root: root.canonicalize().unwrap(),
            target_rel: "mods/remote.jar".to_string(),
            expected_hash: None,
            preserve: false,
            force: false,
            managed_target: false,
        };

        assert_eq!(
            task_source(&task),
            "url https://example.com/remote.jar".to_string()
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn task_source_reports_curseforge_when_metadata_download_is_used() {
        let root = temp_root("task-source-reports-curseforge");
        let task = CopyTask {
            name: "cf".to_string(),
            source: None,
            url: None,
            curseforge: Some(CurseForgeDownload {
                api_key: None,
                cdn_fallback: true,
                project_id: 123,
                file_id: 456,
                filename: "cf.jar".to_string(),
            }),
            target: root.join("mods").join("cf.jar"),
            target_root: root.canonicalize().unwrap(),
            target_rel: "mods/cf.jar".to_string(),
            expected_hash: None,
            preserve: false,
            force: false,
            managed_target: false,
        };

        assert_eq!(task_source(&task), "curseforge project 123 file 456");

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
                split_download_min_bytes: 16 * 1024 * 1024,
                split_download_chunks: 4,
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
                split_download_min_bytes: 16 * 1024 * 1024,
                split_download_chunks: 4,
            },
        )
        .unwrap_err();

        assert!(err.contains("no metadata files were scanned"));
        assert!(root.join("mods").join("old.jar").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reinstall_updates_managed_file_without_cleanup() {
        let root = temp_root("reinstall-updates-managed-file");
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods/mod.jar"), b"v1").unwrap();
        let meta = root.join("mods/mod.pw.toml");
        fs::write(&meta, "filename = \"mod.jar\"\n").unwrap();
        let options = || InstallOptions {
            jobs: 1,
            retries: 1,
            retry_delay_seconds: 0,
            force: false,
            cleanup: false,
            preserve_existing: false,
            split_download_min_bytes: 16 * 1024 * 1024,
            split_download_chunks: 4,
        };

        let target = temp_root("reinstall-updates-managed-target");
        let first = install_local(&root, &target, &Side::Both, options()).unwrap();
        assert!(first.errors.is_empty());
        assert_eq!(fs::read(target.join("mods/mod.jar")).unwrap(), b"v1");

        // Simulate an update: new jar bytes and updated metadata hash.
        fs::write(root.join("mods/mod.jar"), b"v2").unwrap();
        let hash = sha256_file_hex(&root.join("mods/mod.jar")).unwrap();
        fs::write(
            &meta,
            format!(
                "filename = \"mod.jar\"\n[download]\nhash-format = \"sha256\"\nhash = \"{hash}\"\n"
            ),
        )
        .unwrap();

        let second = install_local(&root, &target, &Side::Both, options()).unwrap();
        assert!(second.errors.is_empty());
        assert_eq!(fs::read(target.join("mods/mod.jar")).unwrap(), b"v2");

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(target);
    }

    #[test]
    fn download_mode_repairs_corrupt_managed_file() {
        let root = temp_root("download-mode-repairs-managed");
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods/mod.jar"), b"good").unwrap();
        let hash = sha256_file_hex(&root.join("mods/mod.jar")).unwrap();
        fs::write(
            root.join("mods/mod.pw.toml"),
            format!(
                "filename = \"mod.jar\"\n[download]\nhash-format = \"sha256\"\nhash = \"{hash}\"\n"
            ),
        )
        .unwrap();
        let options = || InstallOptions {
            jobs: 1,
            retries: 1,
            retry_delay_seconds: 0,
            force: false,
            cleanup: false,
            preserve_existing: true,
            split_download_min_bytes: 16 * 1024 * 1024,
            split_download_chunks: 4,
        };

        let target = temp_root("download-mode-repairs-target");
        install_local(&root, &target, &Side::Both, options()).unwrap();
        fs::write(target.join("mods/mod.jar"), b"corrupt").unwrap();

        let repaired = install_local(&root, &target, &Side::Both, options()).unwrap();
        assert!(repaired.errors.is_empty());
        assert_eq!(fs::read(target.join("mods/mod.jar")).unwrap(), b"good");

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(target);
    }

    #[test]
    fn install_refuses_symlinked_managed_root_in_target() {
        let root = temp_root("install-refuses-symlinked-root");
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods/mod.jar"), b"jar").unwrap();
        fs::write(root.join("mods/mod.pw.toml"), "filename = \"mod.jar\"\n").unwrap();
        let outside = temp_root("install-symlink-outside");
        fs::write(outside.join("keep.txt"), b"precious").unwrap();
        let target = temp_root("install-symlink-target");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, target.join("mods")).unwrap();
            let result = install_local(
                &root,
                &target,
                &Side::Both,
                InstallOptions {
                    jobs: 1,
                    retries: 1,
                    retry_delay_seconds: 0,
                    force: false,
                    cleanup: false,
                    preserve_existing: false,
                    split_download_min_bytes: 16 * 1024 * 1024,
                    split_download_chunks: 4,
                },
            )
            .unwrap();
            assert_eq!(result.errors.len(), 1);
            assert!(result.errors[0].contains("outside the target root"));
            assert!(!outside.join("mod.jar").exists());
            assert!(outside.join("keep.txt").is_file());
        }

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
        let _ = fs::remove_dir_all(target);
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

    fn test_install_options() -> InstallOptions {
        InstallOptions {
            jobs: 1,
            retries: 1,
            retry_delay_seconds: 0,
            force: false,
            cleanup: false,
            preserve_existing: false,
            split_download_min_bytes: 16 * 1024 * 1024,
            split_download_chunks: 4,
        }
    }
}
