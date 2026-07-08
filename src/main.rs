mod check;
mod config;
mod curseforge;
mod export_cf;
mod github;
mod http;
mod ignore;
mod init;
mod install;
mod layout;
mod metadata;
mod modlist;
mod murmur2;
mod ops;
mod packinfo;
mod pathutil;
mod refresh;
mod scan;
mod self_update;
mod sha1;
mod sha256;
mod sha512;
mod update;
mod zipstore;

use std::env;
use std::path::PathBuf;

use config::ProjectConfig;
use layout::PackLayout;
use scan::ScanReport;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first() else {
        print_help();
        return;
    };
    let rest = &args[1..];

    let result = match command.as_str() {
        "-h" | "--help" | "help" => {
            print_help();
            Ok(())
        }
        "-V" | "--version" | "version" => {
            println!("bkmpw {VERSION}");
            Ok(())
        }
        "add-url" => add_url(rest),
        "add-curseforge" => add_curseforge(rest),
        "add-file" => add_file(rest),
        "add-github" => add_github(rest),
        "add-resourcepack" => add_resourcepack(rest),
        "add-shaderpack" => add_shaderpack(rest),
        "check" => check_cmd(first_path(rest)),
        "export-curseforge" => export_curseforge(rest),
        "hash" => hash_cmd(rest),
        "init" => init(first_path(rest)),
        "download-files" => download_files(rest),
        "install-files" | "install-files-headless" => install_files_headless(rest),
        "install-files-retry" => install_files_retry(rest),
        "install-local" => install_local(rest),
        "inspect" => inspect(first_path(rest)),
        "list" => list(first_path(rest)),
        "modlist" => write_modlist(rest),
        "pin" => pin_or_unpin(rest, true),
        "remove" | "rm" => remove(rest),
        "scan" => scan(first_path(rest)),
        "self-update" | "update-self" => self_update_cmd(rest),
        "sync" => sync(rest),
        "refresh" => refresh(first_path(rest)),
        "update" => update_cmd(rest),
        "unpin" => pin_or_unpin(rest, false),
        other => Err(format!("unknown command: {other}")),
    };

    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(2);
    }
}

fn print_help() {
    println!("bkmpw {VERSION}");
    println!();
    println!("Usage:");
    println!("  bkmpw add-url <pack-root> <side> <name> <filename> <url> <sha256>");
    println!("  bkmpw add-resourcepack <pack-root> <name> <filename> <url> <sha256>");
    println!("  bkmpw add-shaderpack <pack-root> <name> <filename> <url> <sha256>");
    println!(
        "  bkmpw add-curseforge <pack-root> <side> <project|url|project-id> [file-id] [options]"
    );
    println!("  bkmpw add-file <pack-root> <side> <name> <source-file> [filename]");
    println!(
        "  bkmpw add-github <pack-root> <side> <owner/repo|url> [--tag tag] [--asset text] [--name n] [--filename f] [--cf-project-id id --cf-file-id id]"
    );
    println!("  bkmpw check [pack-root]");
    println!("  bkmpw export-curseforge <pack-root> [output.zip] [side]");
    println!(
        "  bkmpw download-files <pack-root> [jobs] [--force] [--retries n] [--retry-delay-seconds n]"
    );
    println!("  bkmpw hash <sha1|sha256|sha512|murmur2> <file>");
    println!("  bkmpw init [pack-root]");
    println!("  bkmpw install-files-headless <pack-root> [attempts] [delay-seconds]");
    println!("  bkmpw install-files-retry <pack-root> [attempts] [delay-seconds]");
    println!(
        "  bkmpw install-local <source-root> <target-root> <side> [jobs] [--force] [--retries n] [--retry-delay-seconds n]"
    );
    println!("  bkmpw inspect [pack-root]");
    println!("  bkmpw list [pack-root]");
    println!("  bkmpw modlist <pack-root> [output-dir]");
    println!("  bkmpw pin <pack-root> <name>");
    println!("  bkmpw remove <pack-root> <name>");
    println!("  bkmpw scan [pack-root]");
    println!("  bkmpw self-update [--tag vX.Y.Z] [--repo owner/repo] [--dry-run]");
    println!(
        "  bkmpw sync <source-root> <target-root> [side] [jobs] [--force] [--retries n] [--retry-delay-seconds n]"
    );
    println!("  bkmpw refresh [pack-root]");
    println!("  bkmpw update <pack-root> (--all|<name>) [--mc-version v] [--loader neoforge]");
    println!("  bkmpw unpin <pack-root> <name>");
    println!("  bkmpw --help");
    println!("  bkmpw --version");
}

fn first_path(args: &[String]) -> Option<PathBuf> {
    args.first().map(PathBuf::from)
}

fn self_update_cmd(args: &[String]) -> Result<(), String> {
    let mut options = self_update::SelfUpdateOptions {
        repo: self_update::default_repo().to_string(),
        tag: None,
        dry_run: false,
    };
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--repo" => {
                idx += 1;
                options.repo = parse_string_arg(args.get(idx), "--repo")?;
            }
            "--tag" => {
                idx += 1;
                options.tag = Some(parse_string_arg(args.get(idx), "--tag")?);
            }
            "--dry-run" => options.dry_run = true,
            other => {
                return Err(format!(
                    "unknown self-update option: {other}. usage: bkmpw self-update [--tag vX.Y.Z] [--repo owner/repo] [--dry-run]"
                ));
            }
        }
        idx += 1;
    }
    self_update::update_self(&options, VERSION)
}

fn hash_cmd(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("usage: bkmpw hash <sha1|sha256|sha512|murmur2> <file>".to_string());
    }
    let path = PathBuf::from(&args[1]);
    match args[0].as_str() {
        "sha1" => println!("{}", sha1::sha1_file_hex(&path)?),
        "sha256" => println!("{}", sha256::sha256_file_hex(&path)?),
        "sha512" => println!("{}", sha512::sha512_file_hex(&path)?),
        "murmur2" => println!("{}", murmur2::curseforge_murmur2_file(&path)?),
        other => return Err(format!("unsupported hash format: {other}")),
    }
    Ok(())
}

fn check_cmd(root: Option<PathBuf>) -> Result<(), String> {
    let root = match root {
        Some(path) => path,
        None => env::current_dir().map_err(|err| format!("failed to read current dir: {err}"))?,
    };
    let result = check::check(&root);
    for item in &result.ok {
        println!("ok: {item}");
    }
    for item in &result.warnings {
        eprintln!("warn: {item}");
    }
    for item in &result.errors {
        eprintln!("error: {item}");
    }
    if result.is_ok() {
        Ok(())
    } else {
        Err(format!(
            "check failed with {} error(s)",
            result.errors.len()
        ))
    }
}

fn inspect(root: Option<PathBuf>) -> Result<(), String> {
    let root = match root {
        Some(path) => path,
        None => env::current_dir().map_err(|err| format!("failed to read current dir: {err}"))?,
    };
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);

    println!("pack root: {}", root.display());
    println!("config: {}", config.source.display());
    println!("scan.use-gitignore: {}", config.scan.use_gitignore);
    println!(
        "scan.packwizignore: {}",
        config.scan.packwizignore.display()
    );
    println!("layout.metadata-root: {}", layout.metadata_root.display());
    println!("layout.server-meta: {}", layout.server_meta.display());
    println!("layout.client-meta: {}", layout.client_meta.display());
    println!("layout.common-meta: {}", layout.common_meta.display());
    println!("layout.jar-root: {}", layout.jar_root.display());

    Ok(())
}

fn refresh(root: Option<PathBuf>) -> Result<(), String> {
    let root = match root {
        Some(path) => path,
        None => env::current_dir().map_err(|err| format!("failed to read current dir: {err}"))?,
    };
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let result = refresh::refresh(&root, &config, &layout)?;

    println!("refreshed {}", result.index_path.display());
    println!("indexed files: {}", result.files_written);
    println!("metadata files: {}", result.metadata_written);
    println!("jar files: {}", result.jar_written);

    Ok(())
}

fn init(root: Option<PathBuf>) -> Result<(), String> {
    let root = root.unwrap_or_else(|| PathBuf::from("."));
    let result = init::init_pack(&root)?;

    println!("initialized: {}", root.display());
    for created in result.created {
        println!("  created {}", created.display());
    }
    for kept in result.kept {
        println!("  kept {}", kept.display());
    }

    Ok(())
}

fn add_url(args: &[String]) -> Result<(), String> {
    if args.len() < 6 {
        return Err(
            "usage: bkmpw add-url <pack-root> <side> <name> <filename> <url> <sha256>".to_string(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let side = metadata::Side::parse(&args[1]);
    reject_unknown_side(&side)?;
    let name = &args[2];
    let filename = &args[3];
    let url = &args[4];
    let hash = &args[5];

    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let created = ops::add_url_metadata(&root, &layout, &side, name, filename, url, hash)?;
    let refreshed = refresh::refresh(&root, &config, &layout)?;

    println!("added {}", created.display());
    println!("refreshed {}", refreshed.index_path.display());
    Ok(())
}

fn add_resourcepack(args: &[String]) -> Result<(), String> {
    add_pack_asset(args, "add-resourcepack", ops::add_resourcepack_metadata)
}

fn add_shaderpack(args: &[String]) -> Result<(), String> {
    add_pack_asset(args, "add-shaderpack", ops::add_shaderpack_metadata)
}

fn add_pack_asset(
    args: &[String],
    command: &str,
    add_metadata: fn(
        &std::path::Path,
        &PackLayout,
        &str,
        &str,
        &str,
        &str,
    ) -> Result<PathBuf, String>,
) -> Result<(), String> {
    if args.len() < 5 {
        return Err(format!(
            "usage: bkmpw {command} <pack-root> <name> <filename> <url> <sha256>"
        ));
    }
    let root = PathBuf::from(&args[0]);
    let name = &args[1];
    let filename = &args[2];
    let url = &args[3];
    let hash = &args[4];

    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let created = add_metadata(&root, &layout, name, filename, url, hash)?;
    let refreshed = refresh::refresh(&root, &config, &layout)?;

    println!("added {}", created.display());
    println!("refreshed {}", refreshed.index_path.display());
    Ok(())
}

fn add_curseforge(args: &[String]) -> Result<(), String> {
    if args.len() < 3 {
        return Err(
            "usage: bkmpw add-curseforge <pack-root> <side> <project|url|project-id> [file-id] [--name n] [--filename f] [--hash h] [--hash-format sha1] [--mc-version v] [--loader neoforge]"
                .to_string(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let side = metadata::Side::parse(&args[1]);
    reject_unknown_side(&side)?;
    let project = &args[2];
    let options = parse_add_curseforge_options(&args[3..])?;
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let pack = packinfo::PackInfo::load(&root)?;
    let api_key = curseforge::api_key(&config);
    let project_id = project.parse::<u64>().ok();

    let resolved = match (
        api_key.as_deref(),
        project_id,
        options.file_id,
        options.filename.as_ref(),
        options.hash.as_ref(),
    ) {
        (None, Some(project_id), Some(file_id), Some(filename), Some(hash)) => {
            curseforge::CurseForgeFileInfo {
                project_id,
                file_id,
                name: options.name.clone(),
                filename: filename.clone(),
                hash_format: options
                    .hash_format
                    .clone()
                    .unwrap_or_else(|| "sha1".to_string()),
                hash: hash.clone(),
            }
        }
        _ => curseforge::resolve_project_and_file(
            api_key.as_deref(),
            project,
            options.file_id,
            options
                .minecraft_version
                .as_deref()
                .or(pack.minecraft.as_deref()),
            options.loader.as_deref().or(pack.loader_name()),
        )?,
    };

    let name = options
        .name
        .as_deref()
        .or(resolved.name.as_deref())
        .unwrap_or(&resolved.filename);
    let created = ops::add_curseforge_metadata(
        &root,
        &layout,
        &side,
        name,
        &resolved.filename,
        &resolved.hash_format,
        &resolved.hash,
        resolved.project_id,
        resolved.file_id,
    )?;
    let refreshed = refresh::refresh(&root, &config, &layout)?;

    println!("added {}", created.display());
    println!("refreshed {}", refreshed.index_path.display());
    Ok(())
}

fn add_github(args: &[String]) -> Result<(), String> {
    if args.len() < 3 {
        return Err(
            "usage: bkmpw add-github <pack-root> <side> <owner/repo|url> [--tag tag] [--asset text] [--name n] [--filename f] [--cf-project-id id --cf-file-id id]"
                .to_string(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let side = metadata::Side::parse(&args[1]);
    reject_unknown_side(&side)?;
    let project = &args[2];
    let options = parse_add_github_options(&args[3..])?;
    if options.cf_project_id.is_some() != options.cf_file_id.is_some() {
        return Err("--cf-project-id and --cf-file-id must be provided together".to_string());
    }
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let project_id = github::normalize_project(project)?;
    let resolved = github::resolve_github_release_asset(
        project,
        options.tag.as_deref(),
        options.asset.as_deref(),
        options.filename.as_deref(),
        options.name.as_deref(),
    )?;
    let created = ops::add_github_metadata(
        &root,
        &layout,
        &side,
        &resolved.name,
        &resolved.filename,
        &resolved.url,
        &resolved.hash,
        &project_id,
        options.tag.as_deref(),
        options.asset.as_deref(),
        options.cf_project_id,
        options.cf_file_id,
    )?;
    let refreshed = refresh::refresh(&root, &config, &layout)?;

    println!("added {}", created.display());
    println!("source {}", resolved.url);
    println!("hash-format {}", resolved.hash_format);
    println!("hash {}", resolved.hash);
    println!("refreshed {}", refreshed.index_path.display());
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct AddGitHubOptions {
    tag: Option<String>,
    asset: Option<String>,
    name: Option<String>,
    filename: Option<String>,
    cf_project_id: Option<u64>,
    cf_file_id: Option<u64>,
}

fn parse_add_github_options(args: &[String]) -> Result<AddGitHubOptions, String> {
    let mut options = AddGitHubOptions::default();
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--tag" => {
                idx += 1;
                options.tag = Some(parse_string_arg(args.get(idx), "--tag")?);
            }
            "--asset" => {
                idx += 1;
                options.asset = Some(parse_string_arg(args.get(idx), "--asset")?);
            }
            "--name" => {
                idx += 1;
                options.name = Some(parse_string_arg(args.get(idx), "--name")?);
            }
            "--filename" => {
                idx += 1;
                options.filename = Some(parse_string_arg(args.get(idx), "--filename")?);
            }
            "--cf-project-id" => {
                idx += 1;
                options.cf_project_id = Some(parse_u64_arg(args.get(idx), "--cf-project-id")?);
            }
            "--cf-file-id" => {
                idx += 1;
                options.cf_file_id = Some(parse_u64_arg(args.get(idx), "--cf-file-id")?);
            }
            value if !value.starts_with('-') && options.asset.is_none() => {
                options.asset = Some(value.to_string());
            }
            other => return Err(format!("unknown add-github option: {other}")),
        }
        idx += 1;
    }
    Ok(options)
}

fn update_cmd(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err(
            "usage: bkmpw update <pack-root> (--all|<name>) [--mc-version v] [--loader neoforge]"
                .to_string(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let target = &args[1];
    let options = parse_update_options(&args[2..])?;
    let result = if target == "--all" || target == "-a" {
        update::update_all(&root, options)?
    } else {
        update::update_one(&root, target, options)?
    };
    for item in &result.updated {
        println!("updated: {item}");
    }
    for item in &result.unchanged {
        println!("unchanged: {item}");
    }
    for item in &result.skipped {
        println!("skipped: {item}");
    }
    println!("updated files: {}", result.updated.len());
    println!("unchanged files: {}", result.unchanged.len());
    println!("skipped files: {}", result.skipped.len());
    Ok(())
}

fn parse_update_options(args: &[String]) -> Result<update::UpdateOptions, String> {
    let mut options = update::UpdateOptions::default();
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--mc-version" => {
                idx += 1;
                options.minecraft_version = Some(parse_string_arg(args.get(idx), "--mc-version")?);
            }
            "--loader" => {
                idx += 1;
                options.loader = Some(parse_string_arg(args.get(idx), "--loader")?);
            }
            other => return Err(format!("unknown update option: {other}")),
        }
        idx += 1;
    }
    Ok(options)
}

#[derive(Debug, Clone, Default)]
struct AddCurseForgeOptions {
    file_id: Option<u64>,
    name: Option<String>,
    filename: Option<String>,
    hash_format: Option<String>,
    hash: Option<String>,
    minecraft_version: Option<String>,
    loader: Option<String>,
}

fn parse_add_curseforge_options(args: &[String]) -> Result<AddCurseForgeOptions, String> {
    let mut options = AddCurseForgeOptions::default();
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--file-id" => {
                idx += 1;
                options.file_id = Some(parse_u64_arg(args.get(idx), "--file-id")?);
            }
            "--name" => {
                idx += 1;
                options.name = Some(parse_string_arg(args.get(idx), "--name")?);
            }
            "--filename" => {
                idx += 1;
                options.filename = Some(parse_string_arg(args.get(idx), "--filename")?);
            }
            "--hash-format" => {
                idx += 1;
                options.hash_format = Some(parse_string_arg(args.get(idx), "--hash-format")?);
            }
            "--hash" => {
                idx += 1;
                options.hash = Some(parse_string_arg(args.get(idx), "--hash")?);
            }
            "--mc-version" => {
                idx += 1;
                options.minecraft_version = Some(parse_string_arg(args.get(idx), "--mc-version")?);
            }
            "--loader" => {
                idx += 1;
                options.loader = Some(parse_string_arg(args.get(idx), "--loader")?);
            }
            value if !value.starts_with('-') && options.file_id.is_none() => {
                options.file_id = Some(
                    value
                        .parse::<u64>()
                        .map_err(|err| format!("invalid file-id: {err}"))?,
                );
            }
            other => return Err(format!("unknown add-curseforge option: {other}")),
        }
        idx += 1;
    }
    Ok(options)
}

fn parse_u64_arg(value: Option<&String>, name: &str) -> Result<u64, String> {
    value
        .ok_or_else(|| format!("missing value for {name}"))?
        .parse::<u64>()
        .map_err(|err| format!("invalid {name}: {err}"))
}

fn reject_unknown_side(side: &metadata::Side) -> Result<(), String> {
    match side {
        metadata::Side::Unknown(value) => Err(format!("unsupported side: {value}")),
        metadata::Side::Client | metadata::Side::Server | metadata::Side::Both => Ok(()),
    }
}

fn parse_string_arg(value: Option<&String>, name: &str) -> Result<String, String> {
    value
        .filter(|value| !value.trim().is_empty() && !value.starts_with('-'))
        .cloned()
        .ok_or_else(|| format!("missing value for {name}"))
}

fn add_file(args: &[String]) -> Result<(), String> {
    if args.len() < 4 {
        return Err(
            "usage: bkmpw add-file <pack-root> <side> <name> <source-file> [filename]".to_string(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let side = metadata::Side::parse(&args[1]);
    reject_unknown_side(&side)?;
    let name = &args[2];
    let source = PathBuf::from(&args[3]);
    let filename = args.get(4).map(String::as_str);

    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let created = ops::add_local_file(&root, &layout, &side, name, &source, filename)?;
    let refreshed = refresh::refresh(&root, &config, &layout)?;

    println!("added {}", created.display());
    println!("refreshed {}", refreshed.index_path.display());
    Ok(())
}

fn export_curseforge(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("usage: bkmpw export-curseforge <pack-root> [output.zip] [side]".to_string());
    }
    let (root, output, side) = parse_export_curseforge_args(args)?;
    reject_unknown_side(&side)?;
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let refreshed = refresh::refresh(&root, &config, &layout)?;
    let files = export_cf::export_curseforge(&root, &output, &side)?;

    println!("refreshed {}", refreshed.index_path.display());
    println!("exported {}", output.display());
    println!("curseforge files: {files}");
    Ok(())
}

fn parse_export_curseforge_args(
    args: &[String],
) -> Result<(PathBuf, PathBuf, metadata::Side), String> {
    if args.is_empty() {
        return Err("usage: bkmpw export-curseforge <pack-root> [output.zip] [side]".to_string());
    }
    let root = PathBuf::from(&args[0]);
    if let Some(value) = args.get(1) {
        let maybe_side = metadata::Side::parse(value);
        if args.get(2).is_none() && !matches!(maybe_side, metadata::Side::Unknown(_)) {
            return Ok((root.clone(), root.join("curseforge-export.zip"), maybe_side));
        }
    }
    let output = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("curseforge-export.zip"));
    let side = args
        .get(2)
        .map(|value| metadata::Side::parse(value))
        .unwrap_or(metadata::Side::Both);
    Ok((root, output, side))
}

fn install_local(args: &[String]) -> Result<(), String> {
    if args.len() < 3 {
        return Err(
            "usage: bkmpw install-local <source-root> <target-root> <side> [jobs] [--force] [--retries n] [--retry-delay-seconds n]"
                .to_string(),
        );
    }
    let source = PathBuf::from(&args[0]);
    let target = PathBuf::from(&args[1]);
    let side = metadata::Side::parse(&args[2]);
    reject_unknown_side(&side)?;
    let config = ProjectConfig::load(&source)?;
    let options = parse_install_options(&config, &args[3..])?;
    let result = install::install_local(&source, &target, &side, options)?;

    println!("installed files: {}", result.installed);
    println!("skipped files: {}", result.skipped);
    println!("removed files: {}", result.removed);
    if !result.errors.is_empty() {
        for err in result.errors {
            eprintln!("install error: {err}");
        }
        return Err("install completed with errors".to_string());
    }
    Ok(())
}

fn download_files(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err(
            "usage: bkmpw download-files <pack-root> [jobs] [--force] [--retries n] [--retry-delay-seconds n]"
                .to_string(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let config = ProjectConfig::load(&root)?;
    let mut options = parse_install_options(&config, &args[1..])?;
    options.cleanup = false;
    options.preserve_existing = true;
    let result = install::install_local(&root, &root, &metadata::Side::Both, options)?;
    print_install_result("downloaded/satisfied", result)
}

fn sync(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err(
            "usage: bkmpw sync <source-root> <target-root> [side] [jobs] [--force] [--retries n] [--retry-delay-seconds n]"
                .to_string(),
        );
    }
    let source = PathBuf::from(&args[0]);
    let target = PathBuf::from(&args[1]);
    let side = args
        .get(2)
        .filter(|value| !value.starts_with('-') && !value.parse::<usize>().is_ok())
        .map(|value| metadata::Side::parse(value))
        .unwrap_or(metadata::Side::Both);
    reject_unknown_side(&side)?;
    let option_start = if args
        .get(2)
        .is_some_and(|value| !value.starts_with('-') && value.parse::<usize>().is_err())
    {
        3
    } else {
        2
    };
    let config = ProjectConfig::load(&source)?;
    let mut options = parse_install_options(&config, &args[option_start..])?;
    options.cleanup = true;
    let result = install::install_local(&source, &target, &side, options)?;
    print_install_result("synced", result)
}

fn install_files_headless(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err(
            "usage: bkmpw install-files-headless <pack-root> [attempts] [delay-seconds]"
                .to_string(),
        );
    }
    install_files_with_retry_args(args)
}

fn install_files_retry(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err(
            "usage: bkmpw install-files-retry <pack-root> [attempts] [delay-seconds]".to_string(),
        );
    }
    install_files_with_retry_args(args)
}

fn install_files_with_retry_args(args: &[String]) -> Result<(), String> {
    let root = PathBuf::from(&args[0]);
    let config = ProjectConfig::load(&root)?;
    let mut options = default_install_options(&config)?;
    options.cleanup = true;
    options.retries = args
        .get(1)
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|err| format!("invalid attempts: {err}"))
        })
        .transpose()?
        .unwrap_or(5);
    options.retry_delay_seconds = args
        .get(2)
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|err| format!("invalid delay seconds: {err}"))
        })
        .transpose()?
        .unwrap_or(10);
    let result = install::install_local(&root, &root, &metadata::Side::Both, options)?;
    print_install_result("synced", result)
}

fn print_install_result(label: &str, result: install::InstallResult) -> Result<(), String> {
    println!("{label} files: {}", result.installed);
    println!("skipped files: {}", result.skipped);
    println!("removed files: {}", result.removed);
    if !result.errors.is_empty() {
        for err in result.errors {
            eprintln!("install error: {err}");
        }
        return Err("install completed with errors".to_string());
    }
    Ok(())
}

fn parse_install_options(
    config: &ProjectConfig,
    args: &[String],
) -> Result<install::InstallOptions, String> {
    let mut options = default_install_options(config)?;

    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--cleanup" => options.cleanup = true,
            "--no-cleanup" => options.cleanup = false,
            "--force" | "-f" => options.force = true,
            "--retries" => {
                idx += 1;
                let value = args
                    .get(idx)
                    .ok_or_else(|| "missing value for --retries".to_string())?;
                options.retries = value
                    .parse::<usize>()
                    .map_err(|err| format!("invalid retries: {err}"))?;
            }
            "--retry-delay-seconds" => {
                idx += 1;
                let value = args
                    .get(idx)
                    .ok_or_else(|| "missing value for --retry-delay-seconds".to_string())?;
                options.retry_delay_seconds = value
                    .parse::<u64>()
                    .map_err(|err| format!("invalid retry delay: {err}"))?;
            }
            value if !value.starts_with('-') => {
                options.jobs = value
                    .parse::<usize>()
                    .map_err(|err| format!("invalid jobs: {err}"))?;
            }
            other => return Err(format!("unknown install option: {other}")),
        }
        idx += 1;
    }

    Ok(options)
}

fn default_install_options(config: &ProjectConfig) -> Result<install::InstallOptions, String> {
    Ok(install::InstallOptions {
        jobs: env::var("CDPR_DOWNLOAD_THREADS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|err| format!("invalid CDPR_DOWNLOAD_THREADS: {err}"))
            })
            .transpose()?
            .unwrap_or(config.install.jobs),
        retries: config.install.retries,
        retry_delay_seconds: config.install.retry_delay_seconds,
        force: config.install.force,
        cleanup: false,
        preserve_existing: false,
        split_download_min_bytes: config.install.split_download_min_bytes,
        split_download_chunks: config.install.split_download_chunks,
    })
}

fn write_modlist(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("usage: bkmpw modlist <pack-root> [output-dir]".to_string());
    }
    let root = PathBuf::from(&args[0]);
    let output_dir = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("docs").join("generated"));
    let (count, md, csv) = modlist::write_modlist(&root, &output_dir)?;
    println!("modlist entries: {count}");
    println!("wrote {md}");
    println!("wrote {csv}");
    Ok(())
}

fn list(root: Option<PathBuf>) -> Result<(), String> {
    let root = match root {
        Some(path) => path,
        None => env::current_dir().map_err(|err| format!("failed to read current dir: {err}"))?,
    };
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let report = ScanReport::build(&root, &config, &layout)?;

    for entry in &report.metadata {
        let metadata_path = pathutil::join_slash(&root, &entry.path);
        let metadata = metadata::ModMetadata::load(&metadata_path)?;
        let name = metadata.name.as_deref().unwrap_or("(unnamed)");
        let filename = metadata.filename.as_deref().unwrap_or("(no filename)");
        let side = metadata
            .side
            .as_ref()
            .map(metadata::Side::as_str)
            .unwrap_or("(unset)");
        println!(
            "{} | file={} | side={} | folder={}",
            name,
            filename,
            side,
            entry.side_hint.as_str()
        );
    }

    Ok(())
}

fn remove(args: &[String]) -> Result<(), String> {
    let (root, name) = root_and_name(args)?;
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let removed = ops::remove_metadata(&root, &config, &layout, &name)?;
    let refreshed = refresh::refresh(&root, &config, &layout)?;

    println!("removed {}", removed.display());
    println!("refreshed {}", refreshed.index_path.display());
    Ok(())
}

fn pin_or_unpin(args: &[String], pin: bool) -> Result<(), String> {
    let (root, name) = root_and_name(args)?;
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let changed = ops::set_pin(&root, &config, &layout, &name, pin)?;
    let refreshed = refresh::refresh(&root, &config, &layout)?;

    println!(
        "{} {}",
        if pin { "pinned" } else { "unpinned" },
        changed.display()
    );
    println!("refreshed {}", refreshed.index_path.display());
    Ok(())
}

fn root_and_name(args: &[String]) -> Result<(PathBuf, String), String> {
    let Some(root) = args.first() else {
        return Err("missing pack root".to_string());
    };
    let Some(name) = args.get(1) else {
        return Err("missing metadata name".to_string());
    };
    Ok((PathBuf::from(root), name.clone()))
}

fn scan(root: Option<PathBuf>) -> Result<(), String> {
    let root = match root {
        Some(path) => path,
        None => env::current_dir().map_err(|err| format!("failed to read current dir: {err}"))?,
    };
    let config = ProjectConfig::load(&root)?;
    let layout = PackLayout::from_config(&config);
    let report = ScanReport::build(&root, &config, &layout)?;

    println!("pack root: {}", root.display());
    println!("included files: {}", report.included.len());
    println!("metadata files: {}", report.metadata.len());
    println!("jar files: {}", report.jars.len());
    println!("excluded files: {}", report.excluded.len());

    if !report.metadata.is_empty() {
        println!();
        println!("metadata:");
        for entry in &report.metadata {
            println!("  {} [{}]", entry.path, entry.side_hint.as_str());
        }
    }

    if !report.jars.is_empty() {
        println!();
        println!("jars:");
        for jar in &report.jars {
            println!("  {jar}");
        }
    }

    if !report.excluded.is_empty() {
        println!();
        println!("excluded:");
        for file in &report.excluded {
            println!("  {} [{}]", file.path, file.reason.as_str());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn string_option_value_does_not_consume_next_option() {
        let value = "--asset".to_string();

        let result = parse_string_arg(Some(&value), "--tag");

        assert!(result.is_err());
    }

    #[test]
    fn export_curseforge_accepts_side_as_second_arg() {
        let args = vec!["pack".to_string(), "client".to_string()];

        let (root, output, side) = parse_export_curseforge_args(&args).unwrap();

        assert_eq!(root, PathBuf::from("pack"));
        assert_eq!(output, PathBuf::from("pack").join("curseforge-export.zip"));
        assert_eq!(side, metadata::Side::Client);
    }
}
