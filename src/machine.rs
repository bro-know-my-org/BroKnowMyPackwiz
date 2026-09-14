use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::config::ProjectConfig;
use crate::layout::PackLayout;
use crate::metadata::{ModMetadata, Side};
use crate::scan::ScanReport;
use crate::{
    check, export_cf, export_server, init, install, metadata, packinfo, pathutil, refresh, update,
};

pub const PROTOCOL_VERSION: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    JsonLines,
}

pub fn parse_format_args(args: Vec<String>) -> Result<(Vec<String>, Option<Format>), String> {
    let mut format = None;
    let mut filtered = Vec::with_capacity(args.len());
    for arg in args {
        let candidate = match arg.as_str() {
            "--json" => Some(Format::Json),
            "--json-lines" | "--jsonl" => Some(Format::JsonLines),
            _ => None,
        };
        if let Some(candidate) = candidate {
            if format.is_some() {
                return Err("use only one of --json or --json-lines".to_string());
            }
            format = Some(candidate);
        } else {
            filtered.push(arg);
        }
    }
    Ok((filtered, format))
}

struct CommandResult {
    data: Value,
    exit_code: i32,
}

impl CommandResult {
    fn success(data: Value) -> Self {
        Self { data, exit_code: 0 }
    }
}

struct Emitter<'a> {
    command: &'a str,
    format: Format,
    sequence: u64,
    output_error: Option<OutputError>,
}

struct OutputError {
    kind: io::ErrorKind,
    message: String,
}

impl<'a> Emitter<'a> {
    fn new(command: &'a str, format: Format) -> Self {
        Self {
            command,
            format,
            sequence: 0,
            output_error: None,
        }
    }

    fn started(&mut self) -> Result<(), String> {
        if self.format == Format::JsonLines {
            self.event(json!({"event": "started"}))?;
        }
        Ok(())
    }

    fn progress(&mut self, phase: &str, message: &str) -> Result<(), String> {
        if self.format == Format::JsonLines {
            self.event(json!({
                "event": "progress",
                "phase": phase,
                "message": message,
            }))?;
        }
        Ok(())
    }

    fn finish(&mut self, result: Result<CommandResult, String>) -> i32 {
        match result {
            Ok(result) => {
                let ok = result.exit_code == 0;
                let event = if ok { "completed" } else { "failed" };
                let mut value = json!({
                    "ok": ok,
                    "data": result.data,
                });
                if !ok {
                    value["error"] = json!({
                        "code": "COMMAND_FAILED",
                        "message": format!("{} reported a failed result", self.command),
                    });
                }
                if self.format == Format::JsonLines {
                    value["event"] = json!(event);
                    if self.event(value).is_err() {
                        return self.output_exit_code();
                    }
                } else {
                    if self.document(value).is_err() {
                        return self.output_exit_code();
                    }
                }
                result.exit_code
            }
            Err(message) => {
                let mut value = json!({
                    "ok": false,
                    "error": {
                        "code": error_code(&message),
                        "message": message,
                    },
                });
                if self.format == Format::JsonLines {
                    value["event"] = json!("failed");
                    if self.event(value).is_err() {
                        return self.output_exit_code();
                    }
                } else {
                    if self.document(value).is_err() {
                        return self.output_exit_code();
                    }
                }
                2
            }
        }
    }

    fn document(&mut self, mut value: Value) -> Result<(), String> {
        value["protocolVersion"] = json!(PROTOCOL_VERSION);
        value["command"] = json!(self.command);
        self.write(&value)
    }

    fn event(&mut self, mut value: Value) -> Result<(), String> {
        self.sequence += 1;
        value["protocolVersion"] = json!(PROTOCOL_VERSION);
        value["command"] = json!(self.command);
        value["sequence"] = json!(self.sequence);
        self.write(&value)
    }

    fn write(&mut self, value: &Value) -> Result<(), String> {
        write_json(value).map_err(|err| {
            let message = err.to_string();
            self.output_error = Some(OutputError {
                kind: err.kind(),
                message: message.clone(),
            });
            format!("failed to write machine output: {message}")
        })
    }

    fn output_exit_code(&self) -> i32 {
        match &self.output_error {
            Some(error) if error.kind == io::ErrorKind::BrokenPipe => 0,
            Some(error) => {
                eprintln!("error: failed to write machine output: {}", error.message);
                1
            }
            None => 1,
        }
    }
}

fn write_json(value: &Value) -> io::Result<()> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer(&mut lock, value).map_err(io::Error::other)?;
    lock.write_all(b"\n")?;
    lock.flush()
}

pub fn run(command: &str, args: &[String], format: Format, version: &str) -> i32 {
    let normalized = match command {
        "-V" | "--version" => "version",
        other => other,
    };
    let mut emitter = Emitter::new(normalized, format);
    if emitter.started().is_err() {
        return emitter.output_exit_code();
    }
    let result = match crate::operation::lock::for_command(normalized, args) {
        Ok(_locks) => dispatch(normalized, args, version, &mut emitter),
        Err(error) => Err(error.to_string()),
    };
    if emitter.output_error.is_some() {
        return emitter.output_exit_code();
    }
    emitter.finish(result)
}

fn dispatch(
    command: &str,
    args: &[String],
    version: &str,
    emitter: &mut Emitter<'_>,
) -> Result<CommandResult, String> {
    match command {
        "version" => no_args(args, || {
            Ok(CommandResult::success(json!({"version": version})))
        }),
        "protocol-version" => no_args(args, || {
            Ok(CommandResult::success(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "cliVersion": version,
                "formats": ["json", "json-lines"],
            })))
        }),
        "inspect" => inspect(args),
        "check" => check_command(args, emitter),
        "scan" => scan(args, emitter),
        "list" => list(args, emitter),
        "refresh" => refresh_command(args, emitter),
        "init" => init_command(args, emitter),
        "update" => update_command(args, emitter),
        "download-files" => download_files(args, emitter),
        "install-files" | "install-files-headless" => install_files(args, emitter, 5, 10),
        "install-files-retry" => install_files(args, emitter, 5, 10),
        "install-local" => install_local(args, emitter),
        "sync" => sync(args, emitter),
        "export-client" => export_client(args, emitter),
        "export-server" => export_server_command(args, emitter),
        "export-server-installer" => export_server_installer(args, emitter),
        "export-curseforge" => export_curseforge(args, emitter),
        "prepare-server" => prepare_server(args, emitter),
        "prepare-pack" => {
            if args.len() != 1 {
                return Err("usage: bkmpw prepare-pack <pack-root>".into());
            }
            let root = PathBuf::from(&args[0]);
            emitter.progress("preparing", "expanding release templates")?;
            let files = crate::release::prepare_pack(&root)?;
            Ok(CommandResult::success(
                json!({"packRoot": path(&root), "files": files}),
            ))
        }
        other => Err(format!(
            "command does not support machine output in protocol v{PROTOCOL_VERSION}: {other}"
        )),
    }
}

fn no_args(
    args: &[String],
    operation: impl FnOnce() -> Result<CommandResult, String>,
) -> Result<CommandResult, String> {
    if !args.is_empty() {
        return Err(format!("unexpected argument: {}", args[0]));
    }
    operation()
}

fn optional_root(args: &[String]) -> Result<PathBuf, String> {
    if args.len() > 1 {
        return Err(format!("unexpected argument: {}", args[1]));
    }
    args.first().map(PathBuf::from).map(Ok).unwrap_or_else(|| {
        env::current_dir().map_err(|err| format!("failed to read current dir: {err}"))
    })
}

fn inspect(args: &[String]) -> Result<CommandResult, String> {
    let root = optional_root(args)?;
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let pack = packinfo::PackInfo::load(&root)?;
    Ok(CommandResult::success(json!({
        "packRoot": path(&root),
        "pack": {
            "name": pack.name,
            "author": pack.author,
            "version": pack.version,
            "minecraft": pack.minecraft,
            "loader": pack.loader_name(),
            "loaderId": pack.primary_loader_id(),
        },
        "config": {
            "path": path(&config.source),
            "scan": {
                "useGitignore": config.scan.use_gitignore,
                "packwizignore": path(&config.scan.packwizignore),
            },
            "install": {
                "jobs": config.install.jobs,
                "retries": config.install.retries,
                "retryDelaySeconds": config.install.retry_delay_seconds,
                "force": config.install.force,
                "splitDownloadMinBytes": config.install.split_download_min_bytes,
                "splitDownloadChunks": config.install.split_download_chunks,
            },
        },
        "layout": {
            "metadataRoot": path(&layout.metadata_root),
            "metadataRoots": layout.metadata_roots.iter().map(|item| path(item)).collect::<Vec<_>>(),
            "serverMetadata": path(&layout.server_meta),
            "clientMetadata": path(&layout.client_meta),
            "commonMetadata": path(&layout.common_meta),
            "rootOverlays": path(&layout.root_overlays),
            "jarRoot": path(&layout.jar_root),
            "metadataExtension": layout.metadata_extension,
        }
    })))
}

fn check_command(args: &[String], emitter: &mut Emitter<'_>) -> Result<CommandResult, String> {
    let root = optional_root(args)?;
    emitter.progress("checking", "validating pack workspace")?;
    let result = check::check(&root);
    let valid = result.is_ok();
    Ok(CommandResult {
        data: json!({
            "packRoot": path(&root),
            "valid": valid,
            "ok": result.ok,
            "warnings": result.warnings,
            "errors": result.errors,
        }),
        exit_code: if valid { 0 } else { 2 },
    })
}

fn scan(args: &[String], emitter: &mut Emitter<'_>) -> Result<CommandResult, String> {
    let root = optional_root(args)?;
    emitter.progress("scanning", "scanning workspace files")?;
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let report = ScanReport::build(&root, &config, &layout)?;
    Ok(CommandResult::success(scan_json(&root, &report)))
}

fn scan_json(root: &Path, report: &ScanReport) -> Value {
    json!({
        "packRoot": path(root),
        "counts": {
            "included": report.included.len(),
            "metadata": report.metadata.len(),
            "jars": report.jars.len(),
            "excluded": report.excluded.len(),
        },
        "included": report.included,
        "metadata": report.metadata.iter().map(|entry| json!({
            "path": entry.path,
            "sideHint": entry.side_hint.as_str(),
        })).collect::<Vec<_>>(),
        "jars": report.jars,
        "excluded": report.excluded.iter().map(|entry| json!({
            "path": entry.path,
            "reason": entry.reason.as_str(),
        })).collect::<Vec<_>>(),
    })
}

fn list(args: &[String], emitter: &mut Emitter<'_>) -> Result<CommandResult, String> {
    let root = optional_root(args)?;
    emitter.progress("scanning", "loading metadata entries")?;
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let report = ScanReport::build(&root, &config, &layout)?;
    let mut entries = Vec::with_capacity(report.metadata.len());
    for entry in report.metadata {
        let metadata_path = pathutil::join_slash(&root, &entry.path);
        let item = ModMetadata::load(&metadata_path)?;
        entries.push(json!({
            "path": entry.path,
            "folderSide": entry.side_hint.as_str(),
            "name": item.name,
            "filename": item.filename,
            "side": item.side.as_ref().map(Side::as_str),
            "preserve": item.preserve,
            "pin": item.pin,
            "optional": item.optional,
            "optionDefault": item.option_default,
            "download": {
                "url": item.download_url,
                "mode": item.download_mode,
                "hashFormat": item.download_hash_format,
                "hash": item.download_hash,
            },
            "curseforge": {
                "projectId": item.curseforge_project_id,
                "fileId": item.curseforge_file_id,
            },
            "github": {
                "project": item.github_project,
                "tag": item.github_tag,
                "asset": item.github_asset,
            },
            "exportCurseforge": {
                "projectId": item.export_curseforge_project_id,
                "fileId": item.export_curseforge_file_id,
                "latest": item.export_curseforge_latest,
            }
        }));
    }
    Ok(CommandResult::success(json!({
        "packRoot": path(&root),
        "count": entries.len(),
        "entries": entries,
    })))
}

fn refresh_command(args: &[String], emitter: &mut Emitter<'_>) -> Result<CommandResult, String> {
    let root = optional_root(args)?;
    emitter.progress("refreshing", "rebuilding pack index")?;
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let result = refresh::refresh(&root, &config, &layout)?;
    Ok(CommandResult::success(json!({
        "packRoot": path(&root),
        "indexPath": path(&result.index_path),
        "filesWritten": result.files_written,
        "metadataWritten": result.metadata_written,
        "jarsWritten": result.jar_written,
    })))
}

fn init_command(args: &[String], emitter: &mut Emitter<'_>) -> Result<CommandResult, String> {
    let root = optional_root(args)?;
    emitter.progress("initializing", "initializing pack workspace")?;
    let result = init::init_pack(&root)?;
    Ok(CommandResult::success(json!({
        "packRoot": path(&root),
        "created": paths(&result.created),
        "kept": paths(&result.kept),
    })))
}

fn update_command(args: &[String], emitter: &mut Emitter<'_>) -> Result<CommandResult, String> {
    if args.len() < 2 {
        return Err(
            "usage: bkmpw update <pack-root> (--all|<name>) [--mc-version v] [--loader name]"
                .to_string(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let target = &args[1];
    let options = parse_update_options(&args[2..])?;
    emitter.progress("updating", "resolving metadata updates")?;
    let result = if target == "--all" || target == "-a" {
        update::update_all(&root, options)?
    } else {
        update::update_one(&root, target, options)?
    };
    Ok(CommandResult::success(json!({
        "packRoot": path(&root),
        "target": target,
        "updated": result.updated,
        "unchanged": result.unchanged,
        "skipped": result.skipped,
    })))
}

fn parse_update_options(args: &[String]) -> Result<update::UpdateOptions, String> {
    let mut options = update::UpdateOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--mc-version" => {
                index += 1;
                options.minecraft_version = Some(required(args.get(index), "--mc-version")?);
            }
            "--loader" => {
                index += 1;
                options.loader = Some(required(args.get(index), "--loader")?);
            }
            other => return Err(format!("unknown update option: {other}")),
        }
        index += 1;
    }
    Ok(options)
}

fn download_files(args: &[String], emitter: &mut Emitter<'_>) -> Result<CommandResult, String> {
    if args.is_empty() {
        return Err("usage: bkmpw download-files <pack-root> [jobs] [options]".to_string());
    }
    let root = PathBuf::from(&args[0]);
    let config = ProjectConfig::load(&root)?;
    if let Some(option) = args[1..]
        .iter()
        .find(|arg| matches!(arg.as_str(), "--cleanup" | "--no-cleanup"))
    {
        return Err(format!("unknown download-files option: {option}"));
    }
    let mut options = parse_install_options(&config, &args[1..])?;
    options.cleanup = false;
    options.preserve_existing = true;
    emitter.progress("downloading", "installing missing managed files")?;
    let result = install::install_local(&root, &root, &Side::Both, options)?;
    install_result(&root, result)
}

fn install_files(
    args: &[String],
    emitter: &mut Emitter<'_>,
    default_attempts: usize,
    default_delay: u64,
) -> Result<CommandResult, String> {
    if args.is_empty() {
        return Err("missing pack root".to_string());
    }
    if args.len() > 3 {
        return Err(format!("unexpected argument: {}", args[3]));
    }
    let root = PathBuf::from(&args[0]);
    let config = ProjectConfig::load(&root)?;
    let mut options = default_install_options(&config)?;
    options.cleanup = true;
    options.retries = args
        .get(1)
        .map(|value| parse_usize(value, "attempts"))
        .transpose()?
        .unwrap_or(default_attempts);
    options.retry_delay_seconds = args
        .get(2)
        .map(|value| parse_u64(value, "delay seconds"))
        .transpose()?
        .unwrap_or(default_delay);
    emitter.progress("installing", "synchronizing managed files")?;
    let result = install::install_local(&root, &root, &Side::Both, options)?;
    install_result(&root, result)
}

fn install_local(args: &[String], emitter: &mut Emitter<'_>) -> Result<CommandResult, String> {
    if args.len() < 3 {
        return Err(
            "usage: bkmpw install-local <source-root> <target-root> <side> [jobs] [options]"
                .to_string(),
        );
    }
    let source = PathBuf::from(&args[0]);
    let target = PathBuf::from(&args[1]);
    let side = parse_side(&args[2])?;
    let config = ProjectConfig::load(&source)?;
    let options = parse_install_options(&config, &args[3..])?;
    emitter.progress("installing", "installing managed files")?;
    let result = install::install_local(&source, &target, &side, options)?;
    install_result(&target, result)
}

fn sync(args: &[String], emitter: &mut Emitter<'_>) -> Result<CommandResult, String> {
    if args.len() < 2 {
        return Err(
            "usage: bkmpw sync <source-root> <target-root> [side] [jobs] [options]".to_string(),
        );
    }
    let source = PathBuf::from(&args[0]);
    let target = PathBuf::from(&args[1]);
    let side_argument = args
        .get(2)
        .filter(|value| !value.starts_with('-') && value.parse::<usize>().is_err());
    let side = side_argument
        .map(|value| parse_side(value))
        .transpose()?
        .unwrap_or(Side::Both);
    let option_start = if side_argument.is_some() { 3 } else { 2 };
    let config = ProjectConfig::load(&source)?;
    let mut options = parse_install_options(&config, &args[option_start..])?;
    options.cleanup = true;
    emitter.progress("syncing", "synchronizing managed files")?;
    let result = install::install_local(&source, &target, &side, options)?;
    install_result(&target, result)
}

fn install_result(root: &Path, result: install::InstallResult) -> Result<CommandResult, String> {
    let failed = !result.errors.is_empty();
    Ok(CommandResult {
        data: json!({
            "targetRoot": path(root),
            "installed": result.installed,
            "skipped": result.skipped,
            "removed": result.removed,
            "staleTemporaryFilesRemoved": result.temp_removed,
            "warnings": result.warnings,
            "errors": result.errors,
        }),
        exit_code: if failed { 2 } else { 0 },
    })
}

fn default_install_options(config: &ProjectConfig) -> Result<install::InstallOptions, String> {
    let jobs = env::var("CDPR_DOWNLOAD_THREADS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| parse_usize(&value, "CDPR_DOWNLOAD_THREADS"))
        .transpose()?
        .unwrap_or(config.install.jobs);
    Ok(install::InstallOptions {
        jobs,
        retries: config.install.retries,
        retry_delay_seconds: config.install.retry_delay_seconds,
        force: config.install.force,
        cleanup: false,
        preserve_existing: false,
        split_download_min_bytes: config.install.split_download_min_bytes,
        split_download_chunks: config.install.split_download_chunks,
    })
}

fn parse_install_options(
    config: &ProjectConfig,
    args: &[String],
) -> Result<install::InstallOptions, String> {
    let mut options = default_install_options(config)?;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--cleanup" => options.cleanup = true,
            "--no-cleanup" => options.cleanup = false,
            "--force" | "-f" => options.force = true,
            "--retries" => {
                index += 1;
                options.retries = parse_usize(
                    args.get(index)
                        .ok_or_else(|| "missing value for --retries".to_string())?,
                    "retries",
                )?;
            }
            "--retry-delay-seconds" => {
                index += 1;
                options.retry_delay_seconds = parse_u64(
                    args.get(index)
                        .ok_or_else(|| "missing value for --retry-delay-seconds".to_string())?,
                    "retry delay",
                )?;
            }
            value if !value.starts_with('-') => options.jobs = parse_usize(value, "jobs")?,
            other => return Err(format!("unknown install option: {other}")),
        }
        index += 1;
    }
    Ok(options)
}

fn export_client(args: &[String], emitter: &mut Emitter<'_>) -> Result<CommandResult, String> {
    if args.is_empty() || args.len() > 3 {
        return Err("usage: bkmpw export-client <pack-root> [output.zip] [root-dir]".to_string());
    }
    let root = PathBuf::from(&args[0]);
    let output = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("client-full.zip"));
    let root_name = args.get(2).map(String::as_str);
    emitter.progress("refreshing", "rebuilding pack index")?;
    let prepared = crate::release::PreparedPack::new(&root, &output, true)?;
    emitter.progress("exporting", "writing client archive")?;
    let files = export_server::export_client(&prepared.root, &output, root_name)?;
    Ok(release_export_result(
        &root, &output, &prepared, files, "client",
    ))
}

fn export_server_command(
    args: &[String],
    emitter: &mut Emitter<'_>,
) -> Result<CommandResult, String> {
    let (root, output) = export_paths(args, "server-pack.zip", "export-server")?;
    emitter.progress("refreshing", "rebuilding pack index")?;
    let prepared = crate::release::PreparedPack::new(&root, &output, true)?;
    emitter.progress("exporting", "writing server archive")?;
    let files = export_server::export_server(&prepared.root, &output)?;
    Ok(release_export_result(
        &root, &output, &prepared, files, "server",
    ))
}

fn export_server_installer(
    args: &[String],
    emitter: &mut Emitter<'_>,
) -> Result<CommandResult, String> {
    let (root, output) = export_paths(args, "server-installer.zip", "export-server-installer")?;
    emitter.progress("refreshing", "rebuilding pack index")?;
    let prepared = crate::release::PreparedPack::new(&root, &output, false)?;
    emitter.progress("exporting", "writing server installer archive")?;
    let files = export_server::export_server_installer(&prepared.root, &output)?;
    Ok(release_export_result(
        &root,
        &output,
        &prepared,
        files,
        "server-installer",
    ))
}

fn export_curseforge(args: &[String], emitter: &mut Emitter<'_>) -> Result<CommandResult, String> {
    if args.is_empty() || args.len() > 3 {
        return Err("usage: bkmpw export-curseforge <pack-root> [output.zip] [side]".to_string());
    }
    let root = PathBuf::from(&args[0]);
    let (output, side) = if let Some(second) = args.get(1) {
        match Side::parse(second) {
            side @ (Side::Client | Side::Server | Side::Both) if args.len() == 2 => {
                (root.join("curseforge-export.zip"), side)
            }
            _ => (
                PathBuf::from(second),
                args.get(2)
                    .map(|value| parse_side(value))
                    .transpose()?
                    .unwrap_or(Side::Both),
            ),
        }
    } else {
        (root.join("curseforge-export.zip"), Side::Both)
    };
    emitter.progress("refreshing", "rebuilding pack index")?;
    let prepared = crate::release::PreparedPack::new(&root, &output, true)?;
    emitter.progress("exporting", "writing CurseForge archive")?;
    let files = export_cf::export_curseforge(&prepared.root, &output, &side)?;
    Ok(release_export_result(
        &root,
        &output,
        &prepared,
        files,
        "curseforge",
    ))
}

fn prepare_server(args: &[String], emitter: &mut Emitter<'_>) -> Result<CommandResult, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("usage: bkmpw prepare-server <pack-root> [output-dir]".to_string());
    }
    let root = PathBuf::from(&args[0]);
    let output = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".bkmpw").join("server-pack"));
    let refreshed = refresh_pack(&root, emitter)?;
    emitter.progress("preparing", "preparing server directory")?;
    let files = export_server::prepare_server(&root, &output)?;
    Ok(export_result(
        &root,
        &output,
        refreshed,
        files,
        "prepared-server",
    ))
}

fn refresh_pack(root: &Path, emitter: &mut Emitter<'_>) -> Result<refresh::RefreshResult, String> {
    emitter.progress("refreshing", "rebuilding pack index")?;
    let config = ProjectConfig::load(root)?;
    let layout = PackLayout::from_config(&config);
    refresh::refresh(root, &config, &layout)
}

fn release_export_result(
    root: &Path,
    output: &Path,
    prepared: &crate::release::PreparedPack,
    files: usize,
    kind: &str,
) -> CommandResult {
    let mut result = export_result(root, output, prepared.refreshed.clone(), files, kind);
    result.data["releaseStaged"] = json!(prepared.staged);
    result.data["refresh"]["temporary"] = json!(prepared.staged);
    if prepared.staged {
        result.data["refresh"]["indexPath"] = Value::Null;
    }
    result
}

fn export_result(
    root: &Path,
    output: &Path,
    refreshed: refresh::RefreshResult,
    files: usize,
    kind: &str,
) -> CommandResult {
    CommandResult::success(json!({
        "packRoot": path(root),
        "kind": kind,
        "output": path(output),
        "files": files,
        "refresh": {
            "indexPath": path(&refreshed.index_path),
            "filesWritten": refreshed.files_written,
            "metadataWritten": refreshed.metadata_written,
            "jarsWritten": refreshed.jar_written,
        }
    }))
}

fn export_paths(
    args: &[String],
    default_name: &str,
    command: &str,
) -> Result<(PathBuf, PathBuf), String> {
    if args.is_empty() || args.len() > 2 {
        return Err(format!("usage: bkmpw {command} <pack-root> [output]"));
    }
    let root = PathBuf::from(&args[0]);
    let output = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(default_name));
    Ok((root, output))
}

fn parse_side(value: &str) -> Result<Side, String> {
    match metadata::Side::parse(value) {
        side @ (Side::Client | Side::Server | Side::Both) => Ok(side),
        Side::Unknown(value) => Err(format!("unsupported side: {value}")),
    }
}

fn required(value: Option<&String>, option: &str) -> Result<String, String> {
    value
        .filter(|value| !value.trim().is_empty() && !value.starts_with('-'))
        .cloned()
        .ok_or_else(|| format!("missing value for {option}"))
}

fn parse_usize(value: &str, name: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|err| format!("invalid {name}: {err}"))
}

fn parse_u64(value: &str, name: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|err| format!("invalid {name}: {err}"))
}

fn path(value: &Path) -> String {
    value.to_string_lossy().into_owned()
}

fn paths(values: &[PathBuf]) -> Vec<String> {
    values.iter().map(|value| path(value)).collect()
}

fn error_code(message: &str) -> &'static str {
    if message.contains("does not exist") || message.contains("failed to read") {
        "IO_ERROR"
    } else if message.contains("unsupported side")
        || message.contains("does not support machine output")
        || message.contains("usage:")
        || message.contains("unknown")
        || message.contains("unexpected argument")
        || message.contains("missing ")
    {
        "INVALID_ARGUMENT"
    } else {
        "COMMAND_FAILED"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_flag_in_any_position() {
        let args = vec!["list".to_string(), ".".to_string(), "--json".to_string()];
        let (args, format) = parse_format_args(args).unwrap();
        assert_eq!(args, vec!["list", "."]);
        assert_eq!(format, Some(Format::Json));
    }

    #[test]
    fn rejects_multiple_machine_formats() {
        let args = vec![
            "--json".to_string(),
            "list".to_string(),
            "--json-lines".to_string(),
        ];
        assert!(parse_format_args(args).is_err());
    }

    #[test]
    fn parses_side_aliases() {
        assert!(matches!(parse_side("common"), Ok(Side::Both)));
        assert!(parse_side("somewhere").is_err());
    }
}
