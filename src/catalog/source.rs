use super::github::{self, Asset};
use crate::{
    config::ProjectConfig,
    layout::PackLayout,
    metadata::{ModMetadata, Side},
    operation::{
        Control, Error, ErrorCode, Result, durable,
        edit::{self, Attachment},
        preview::Guard,
        queue::Request,
        transfer,
    },
    scan::ScanReport,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Value};

#[derive(Clone)]
pub enum Input {
    Url {
        url: String,
        sha256: String,
    },
    Local(PathBuf),
    GitHub {
        repository: String,
        asset: Asset,
        update_tag: String,
        update_filter: String,
    },
}
#[derive(Clone)]
pub struct Options {
    pub name: String,
    pub filename: String,
    pub kind: String,
    pub side: Side,
    pub download: bool,
}
pub struct Prepared {
    pub request: Request,
    pub name: String,
    pub filename: String,
    pub sha256: String,
    pub download: bool,
}
pub fn prepare(
    root: &Path,
    state: &Path,
    input: Input,
    options: Options,
    control: &Control,
) -> Result<Prepared> {
    let directory = state.join("drafts/assets");
    fs::create_dir_all(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    }
    let payload = directory.join(durable::unique_id());
    let result = prepare_inner(root, state, &input, &options, &payload, control);
    if result.is_err() || !options.download {
        crate::operation::artifacts::remove_temporary(&payload);
    }
    result
}
fn prepare_inner(
    root: &Path,
    state: &Path,
    input: &Input,
    options: &Options,
    payload: &Path,
    control: &Control,
) -> Result<Prepared> {
    control.check()?;
    let mut guard = Guard::capture(root, control)?;
    let config = ProjectConfig::load_operation(root)?;
    let layout = PackLayout::from_config(&config);
    if options.name.trim().is_empty() {
        return Err(invalid("name_required"));
    }
    let filename = crate::operation::paths::filename(&options.filename)?;
    let directory = match options.kind.as_str() {
        "mods" => layout.metadata_root.to_string_lossy().replace('\\', "/"),
        "resourcepacks" | "shaderpacks" => options.kind.clone(),
        _ => return Err(invalid("unknown_file_type")),
    };
    let side = if options.kind != "mods" {
        &Side::Client
    } else {
        &options.side
    };
    if matches!(side, Side::Unknown(_)) {
        return Err(invalid("unknown_side"));
    }
    let slug = crate::ops::safe_slug(&options.name).map_err(Error::from)?;
    let path = format!(
        "{directory}/{}",
        crate::ops::metadata_filename(&slug, &layout)
    );
    crate::operation::paths::relative(&path)?;
    let target = crate::install::resolve_pack_file_path_operation(&path, &filename, &layout)?;
    if root.join(&path).exists() || root.join(&target).exists() {
        return Err(collision(&target));
    }
    let report = ScanReport::build_operation(root, &config, &layout)?;
    for entry in report.metadata {
        control.check()?;
        if entry.path.eq_ignore_ascii_case(&path) {
            return Err(collision(&path));
        }
        let metadata = ModMetadata::load_operation(&root.join(&entry.path))?;
        if let Some(name) = metadata.filename {
            let existing_target =
                crate::install::resolve_pack_file_path_operation(&entry.path, &name, &layout)?;
            if target.eq_ignore_ascii_case(&existing_target) {
                return Err(collision(&target));
            }
        }
    }
    guard.watch(root, &path)?;
    guard.watch(root, &target)?;
    let (url, hash) = match input {
        Input::Url { url, sha256 } => {
            validate_url(url)?;
            if !sha256.is_empty() {
                validate_hash(sha256)?;
            }
            let hash = if sha256.is_empty() {
                transfer::download(
                    url,
                    payload,
                    (!sha256.is_empty()).then_some(("sha256", sha256.as_str())),
                    control,
                )?
                .sha256
            } else {
                sha256.to_ascii_lowercase()
            };
            (url.clone(), hash)
        }
        Input::Local(path) => {
            let path = durable::canonical(path)?;
            let expected = durable::fingerprint(&path)?;
            let hashes = if options.download {
                transfer::copy(&path, payload, control)?
            } else {
                transfer::hash(&path, control)?
            };
            if durable::fingerprint(&path)? != expected {
                return Err(Error::key(ErrorCode::Conflict, "local_source_changed"));
            }
            (String::new(), hashes.sha256)
        }
        Input::GitHub {
            repository, asset, ..
        } => {
            let client = github::Client {
                transport: github::Http,
            };
            let current = client.asset(repository, asset.id)?;
            if current.url != asset.url
                || current.name != asset.name
                || current.size != asset.size
                || current.sha256 != asset.sha256
            {
                return Err(Error::key(ErrorCode::Conflict, "github_asset_changed"));
            }
            let hash = if asset.sha256.is_none() {
                let hashes = transfer::download(
                    &asset.url,
                    payload,
                    asset.sha256.as_ref().map(|h| ("sha256", h.as_str())),
                    control,
                )?;
                if hashes.size != asset.size {
                    return Err(Error::key(ErrorCode::Failed, "github_asset_size_mismatch"));
                }
                hashes.sha256
            } else {
                asset.sha256.clone().unwrap()
            };
            (asset.url.clone(), hash)
        }
    };
    let mut document = DocumentMut::new();
    for (keys, value) in [
        (vec!["name"], options.name.as_str()),
        (vec!["filename"], filename.as_str()),
        (vec!["side"], side.as_str()),
        (vec!["download", "url"], url.as_str()),
        (vec!["download", "hash-format"], "sha256"),
        (vec!["download", "hash"], hash.as_str()),
    ] {
        edit::set(&mut document, &keys, Value::from(value))?;
    }
    if let Input::GitHub {
        repository,
        update_tag,
        update_filter,
        ..
    } = input
    {
        for (key, value) in [
            (
                "project",
                crate::github::normalize_project(repository).map_err(Error::from)?,
            ),
            ("tag", update_tag.clone()),
            ("asset", update_filter.clone()),
        ] {
            edit::set(
                &mut document,
                &["update", "github", key],
                Value::from(value),
            )?;
        }
    }
    guard.validate(root, control)?;
    let drafts = vec![edit::draft(state, &path, None, &document)?];
    let request = if options.download && !payload.exists() {
        Request::Download {
            drafts,
            downloads: vec![crate::operation::download::Download {
                relative: target,
                expected: None,
                source: crate::operation::download::Source::Url(url),
                hash_format: "sha256".into(),
                hash: hash.clone(),
                preserve: false,
            }],
            guard,
        }
    } else if options.download {
        let file = Attachment {
            relative: target,
            source: payload.into(),
            expected: None,
            prepared: durable::fingerprint(payload)?.ok_or_else(|| invalid("missing_payload"))?,
            sha256: hash.clone(),
        };
        Request::PreparedFiles {
            drafts,
            files: vec![file],
            guard,
        }
    } else {
        Request::PreparedEdit { drafts, guard }
    };
    Ok(Prepared {
        request,
        name: options.name.clone(),
        filename,
        sha256: hash,
        download: options.download,
    })
}
fn validate_url(value: &str) -> Result<()> {
    let uri = value
        .parse::<ureq::http::Uri>()
        .map_err(|_| invalid("http_url_required"))?;
    if !matches!(uri.scheme_str(), Some("https" | "http")) || uri.authority().is_none() {
        return Err(invalid("http_url_required"));
    }
    Ok(())
}
fn validate_hash(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("sha256_required"));
    }
    Ok(())
}
fn invalid(message: &str) -> Error {
    Error::key(ErrorCode::Invalid, message)
}
fn collision(path: &str) -> Error {
    Error::key(ErrorCode::Conflict, "file_collision").context(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_add_defaults_to_metadata_and_optional_copy_is_frozen() {
        let base = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(base.join("pack/mods")).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let root = base.join("pack");
        fs::write(root.join("pack.toml"), "name = \"Test\"\n").unwrap();
        let file = base.join("source.jar");
        fs::write(&file, b"original").unwrap();
        let options = Options {
            name: "Test mod".into(),
            filename: "test.jar".into(),
            kind: "mods".into(),
            side: Side::Both,
            download: false,
        };
        let control = Control::default();
        let state = base.join("state");
        let metadata = prepare(
            &root,
            &state,
            Input::Local(file.clone()),
            options.clone(),
            &control,
        )
        .unwrap();
        let remote = prepare(
            &root,
            &state,
            Input::Url {
                url: "http://127.0.0.1:1/test.jar".into(),
                sha256: "a".repeat(64),
            },
            Options {
                download: true,
                ..options.clone()
            },
            &control,
        )
        .unwrap();
        assert!(matches!(remote.request, Request::Download { .. }));
        assert!(matches!(metadata.request, Request::PreparedEdit { .. }));
        let copy = prepare(
            &root,
            &state,
            Input::Local(file.clone()),
            Options {
                download: true,
                ..options
            },
            &control,
        )
        .unwrap();
        fs::write(&file, b"changed later").unwrap();
        let Request::PreparedFiles {
            drafts,
            files,
            guard,
        } = copy.request
        else {
            panic!()
        };
        edit::execute_files(
            &root,
            &state,
            &base.join("task"),
            &drafts,
            &files,
            Some(&guard),
            &control,
        )
        .unwrap();
        assert_eq!(fs::read(root.join("mods/test.jar")).unwrap(), b"original");
        assert!(root.join("mods/test-mod.pw.toml").exists());
        fs::remove_dir_all(base).unwrap();
    }
}
