use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::config::ProjectConfig;
use crate::layout::PackLayout;
use crate::pathutil::{is_under_slash, to_slash};
use crate::{check, install, metadata::ModMetadata, refresh, scan::ScanReport};

pub struct PreparedPack {
    pub root: PathBuf,
    pub refreshed: refresh::RefreshResult,
    pub staged: bool,
    _stage: Option<Stage>,
}

struct Stage(PathBuf);

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl PreparedPack {
    pub fn new(root: &Path, output: &Path, runtime_files: bool) -> Result<Self, String> {
        let config = ProjectConfig::load(root)?;
        let layout = PackLayout::from_config(&config);
        if !config.release.enabled {
            return Ok(Self {
                root: root.to_path_buf(),
                refreshed: refresh::refresh(root, &config, &layout)?,
                staged: false,
                _stage: None,
            });
        }
        let canonical_root = canonical_pack_root(root)?;
        let root = canonical_root.as_path();
        let templates = template_files(root, &config)?;
        let report = ScanReport::build(root, &config, &layout)?;
        if let Some(dir) = &config.release.template_dir {
            for rel in &report.included {
                if within_template_dir(Path::new(rel), dir)
                    && !templates.iter().any(|(source, _)| {
                        to_slash(source.strip_prefix(root).unwrap()).eq_ignore_ascii_case(rel)
                    })
                {
                    return Err(format!("template directory overlaps publish input: {rel}"));
                }
            }
        }
        let mut files: BTreeSet<PathBuf> = report.included.iter().map(PathBuf::from).collect();
        for rel in ["pack.toml", "packwiz.json", ".pw/config.toml", ".gitignore"] {
            files.insert(PathBuf::from(rel));
        }
        files.insert(config.scan.packwizignore.clone());
        collect_tree(root, &layout.root_overlays, &mut files)?;
        // Preserve nested ignore scopes even when the ignore files are excluded.
        for rel in files.clone() {
            for parent in rel.ancestors().skip(1) {
                files.insert(parent.join(".gitignore"));
            }
        }
        let stage = Stage::new()?;
        if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let output_rel = crate::export_server::output_rel_in_root(root, output)?;
        if let Some(rel) = &output_rel {
            if ["common", "client", "server"].iter().any(|side| {
                let target = to_slash(&layout.root_overlays.join(side).join(rel));
                files
                    .iter()
                    .any(|file| to_slash(file).eq_ignore_ascii_case(&target))
            }) {
                return Err(format!(
                    "output path collides with release overlay target: {rel}"
                ));
            }
        }
        for (source, rel) in &templates {
            if output_rel.as_ref().is_some_and(|out| {
                out.eq_ignore_ascii_case(&to_slash(rel))
                    || out.eq_ignore_ascii_case(&to_slash(source.strip_prefix(root).unwrap()))
            }) {
                return Err("output path collides with release template".into());
            }
        }
        let output_abs = output.canonicalize().ok();
        for rel in files {
            let source = root.join(&rel);
            if templates.iter().any(|(_, target)| case_alias(target, &rel)) {
                return Err(format!(
                    "release template case differs from publish input: {}",
                    to_slash(&rel)
                ));
            }
            reject_symlinks(&source)?;
            if !source.is_file()
                || config
                    .release
                    .template_dir
                    .as_ref()
                    .is_some_and(|dir| within_template_dir(&rel, dir))
                || output_abs
                    .as_ref()
                    .is_some_and(|out| source.canonicalize().ok().as_ref() == Some(out))
            {
                continue;
            }
            copy_file(&source, &stage.0.join(rel))?;
        }
        for (source, rel) in &templates {
            copy_file(source, &stage.0.join(rel))?;
        }
        // Explicit template files are publish inputs, even if generated copies
        // are gitignored in the development workspace.
        let ignore_path = stage.0.join(&config.scan.packwizignore);
        let mut ignores = fs::read_to_string(&ignore_path).unwrap_or_default();
        ignores.push('\n');
        for (_, rel) in &templates {
            for ancestor in rel
                .ancestors()
                .skip(1)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                if !ancestor.as_os_str().is_empty() {
                    ignores.push_str(&format!("!/{}\n", to_slash(ancestor)));
                }
            }
            ignores.push_str(&format!("!/{}\n", to_slash(rel)));
        }
        crate::pathutil::write_atomic(&ignore_path, ignores)?;
        // Templates can replace metadata, so resolve runtime files only after
        // expanding them. Explicit runtime templates take priority over local files.
        for entry in ScanReport::build(&stage.0, &config, &layout)?.metadata {
            let metadata = ModMetadata::load(&stage.0.join(&entry.path))?;
            if let Some(filename) = metadata.filename {
                let rel = install::resolve_pack_file_path(&entry.path, &filename, &layout)?;
                if templates
                    .iter()
                    .any(|(_, path)| case_alias(path, Path::new(&rel)))
                {
                    return Err(format!(
                        "release template case differs from managed runtime target: {rel}"
                    ));
                }
                if !runtime_files {
                    continue;
                }
                if output_rel
                    .as_ref()
                    .is_some_and(|out| out.eq_ignore_ascii_case(&rel))
                {
                    return Err(format!(
                        "output path collides with managed runtime file: {rel}"
                    ));
                }
                let source = root.join(&rel);
                reject_symlinks(&source)?;
                let target = stage.0.join(&rel);
                if !target.exists() && source.is_file() {
                    copy_file(&source, &target)?;
                }
            }
        }
        let template_targets = templates.iter().map(|(_, rel)| to_slash(rel)).collect();
        let result = check::check_release(&stage.0, &template_targets);
        if !result.is_ok() {
            return Err(format!(
                "release validation failed:\n{}",
                result.errors.join("\n")
            ));
        }
        let refreshed = refresh::refresh(&stage.0, &config, &layout)?;
        Ok(Self {
            root: stage.0.clone(),
            refreshed,
            staged: true,
            _stage: Some(stage),
        })
    }
}

/// Explicitly expand templates into the development root; exports use a stage.
pub fn prepare_pack(root: &Path) -> Result<usize, String> {
    let canonical_root = canonical_pack_root(root)?;
    let root = canonical_root.as_path();
    let config = ProjectConfig::load(root)?;
    let templates = template_files(root, &config)?;
    if templates.is_empty() {
        return Err("prepare-pack needs release.template-dir and release.template-files".into());
    }
    let stage = Stage::new()?;
    let targets: BTreeSet<_> = templates
        .iter()
        .map(|(_, rel)| rel.clone())
        .chain([PathBuf::from("pack.toml"), PathBuf::from("index.toml")])
        .collect();
    let mut existed = BTreeSet::new();
    let mut new_dirs = BTreeSet::new();
    // Validate and back up every affected file before touching the workspace.
    for rel in &targets {
        let target = root.join(rel);
        reject_symlinks(&target)?;
        for parent in rel
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
        {
            if !root.join(parent).exists() {
                new_dirs.insert(parent.to_path_buf());
            }
        }
        if target.exists() {
            if !target.is_file() {
                return Err(format!(
                    "template destination is not a file: {}",
                    target.display()
                ));
            }
            copy_file(&target, &stage.0.join("backup").join(rel))?;
            existed.insert(rel.clone());
        }
    }
    for (source, rel) in &templates {
        copy_file(source, &stage.0.join("replacement").join(rel))?;
    }
    let mut touched = Vec::new();
    let result = (|| {
        for (_, rel) in &templates {
            touched.push(rel.clone());
            replace_file(&stage.0.join("replacement").join(rel), &root.join(rel))?;
        }
        touched.extend([PathBuf::from("pack.toml"), PathBuf::from("index.toml")]);
        refresh::refresh(root, &config, &PackLayout::from_config(&config))?;
        Ok(templates.len())
    })();
    if let Err(error) = result {
        let mut rollback_errors = Vec::new();
        for rel in touched.into_iter().collect::<BTreeSet<_>>() {
            let restored = if existed.contains(&rel) {
                replace_file(&stage.0.join("backup").join(&rel), &root.join(&rel))
            } else {
                match fs::remove_file(root.join(&rel)) {
                    Ok(()) => Ok(()),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(err) => Err(err.to_string()),
                }
            };
            if let Err(err) = restored {
                rollback_errors.push(format!("{}: {err}", rel.display()));
            }
        }
        for dir in new_dirs.into_iter().rev() {
            if let Err(err) = fs::remove_dir(root.join(&dir)) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    rollback_errors.push(format!("{}: {err}", dir.display()));
                }
            }
        }
        if !rollback_errors.is_empty() {
            let backup = stage.0.clone();
            std::mem::forget(stage);
            return Err(format!(
                "{error}; rollback failed: {}; originals retained at {}",
                rollback_errors.join("; "),
                backup.display()
            ));
        }
        return Err(error);
    }
    result
}

fn template_files(root: &Path, config: &ProjectConfig) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let Some(dir) = &config.release.template_dir else {
        if !config.release.template_files.is_empty() {
            return Err("release.template-files needs release.template-dir".into());
        }
        return Ok(Vec::new());
    };
    if config.release.template_files.is_empty() {
        return Err("release.template-dir needs release.template-files".into());
    }
    let dir_slash = to_slash(dir).to_ascii_lowercase();
    let mut protected: Vec<_> = config
        .layout
        .metadata_roots
        .iter()
        .chain([
            &config.layout.server_meta,
            &config.layout.client_meta,
            &config.layout.common_meta,
        ])
        .map(|p| to_slash(p).to_ascii_lowercase())
        .collect();
    protected.extend([
        to_slash(&config.layout.jar_root),
        to_slash(&config.layout.root_overlays),
        to_slash(&config.scan.packwizignore),
        "pack.toml".into(),
        "index.toml".into(),
        "packwiz.json".into(),
        ".gitignore".into(),
        ".git".into(),
        ".pw".into(),
        ".bkmpw".into(),
        "target".into(),
    ]);
    if protected.iter().any(|p| {
        is_under_slash(&dir_slash, &p.to_ascii_lowercase())
            || is_under_slash(&p.to_ascii_lowercase(), &dir_slash)
    }) {
        return Err(format!(
            "template directory overlaps protected pack path: {}",
            dir.display()
        ));
    }
    let mut destinations = BTreeSet::new();
    config
        .release
        .template_files
        .iter()
        .map(|rel| {
            let slash = to_slash(rel).to_ascii_lowercase();
            if !destinations.insert(slash.clone()) {
                return Err(format!("duplicate release template destination: {slash}"));
            }
            if slash.contains(['*', '?']) {
                return Err(format!(
                    "release template destination contains wildcard: {slash}"
                ));
            }
            if [
                ".git",
                ".pw",
                ".bkmpw",
                "target",
                ".gitignore",
                "index.toml",
                "packwiz.json",
            ]
            .iter()
            .any(|p| is_under_slash(&slash, p))
                || is_under_slash(
                    &slash,
                    &to_slash(&config.scan.packwizignore).to_ascii_lowercase(),
                )
                || slash == ".gitignore"
                || slash == "index.toml"
                || slash == "packwiz.json"
                || is_under_slash(&slash, &dir_slash)
            {
                return Err(format!("reserved release template destination: {slash}"));
            }
            let source = root.join(dir).join(rel);
            reject_symlinks(&source)?;
            if !source.is_file() {
                return Err(format!("missing release template: {}", source.display()));
            }
            Ok((source, rel.clone()))
        })
        .collect()
}

impl Stage {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .map_err(|err| err.to_string())?;
        let path = temp_root.join(format!(
            "bkmpw-release-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|err| err.to_string())?
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).map_err(|err| err.to_string())?;
        Ok(Self(path))
    }
}

fn canonical_pack_root(root: &Path) -> Result<PathBuf, String> {
    if fs::symlink_metadata(root)
        .map_err(|err| err.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err(format!("refusing release symlink: {}", root.display()));
    }
    root.canonicalize().map_err(|err| err.to_string())
}

fn within_template_dir(rel: &Path, dir: &Path) -> bool {
    is_under_slash(
        &to_slash(rel).to_ascii_lowercase(),
        &to_slash(dir).to_ascii_lowercase(),
    )
}

fn collect_tree(root: &Path, rel: &Path, files: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    let path = root.join(rel);
    reject_symlinks(&path)?;
    if path.is_dir() {
        for entry in fs::read_dir(&path).map_err(|err| err.to_string())? {
            let entry = entry.map_err(|err| err.to_string())?;
            collect_tree(root, &rel.join(entry.file_name()), files)?;
        }
    } else if path.is_file() {
        files.insert(rel.to_path_buf());
    }
    Ok(())
}

fn reject_symlinks(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("refusing release symlink: {}", ancestor.display()));
            }
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => return Err(err.to_string()),
            _ => {}
        }
    }
    Ok(())
}

fn copy_file(source: &Path, target: &Path) -> Result<(), String> {
    reject_symlinks(source)?;
    reject_symlinks(target)?;
    fs::create_dir_all(target.parent().ok_or("missing destination parent")?)
        .map_err(|err| err.to_string())?;
    fs::copy(source, target)
        .map_err(|err| format!("failed to copy {}: {err}", source.display()))?;
    Ok(())
}

fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    reject_symlinks(target)?;
    fs::create_dir_all(target.parent().ok_or("missing destination parent")?)
        .map_err(|err| err.to_string())?;
    let temp = crate::export_server::temp_zip_path(target);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|err| err.to_string())?;
    let result =
        copy_file(source, &temp).and_then(|()| crate::export_server::replace_output(&temp, target));
    crate::export_server::cleanup_temp_on_error(result, &temp)
}

fn case_alias(left: &Path, right: &Path) -> bool {
    left != right && to_slash(left).eq_ignore_ascii_case(&to_slash(right))
}
