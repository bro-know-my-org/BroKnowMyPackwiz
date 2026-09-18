use std::env;
use std::fs;
use std::path::PathBuf;

use crate::http::{http_get_to_file, http_get_to_string, http_get_to_string_with_header};
use crate::operation::{Error, ErrorCode, Result as OperationResult};
use crate::sha256::sha256_file_hex_operation;

#[derive(Debug, Clone)]
pub struct GitHubFileInfo {
    pub name: String,
    pub filename: String,
    pub url: String,
    pub hash_format: String,
    pub hash: String,
}

#[derive(Debug, Clone)]
struct GitHubRelease {
    name: Option<String>,
    tag: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone)]
struct GitHubAsset {
    name: String,
    url: String,
    sha256: Option<String>,
}

pub fn resolve_github_release_asset(
    project: &str,
    tag: Option<&str>,
    asset_filter: Option<&str>,
    filename_override: Option<&str>,
    name_override: Option<&str>,
) -> Result<GitHubFileInfo, String> {
    resolve_github_release_asset_operation(
        project,
        tag,
        asset_filter,
        filename_override,
        name_override,
    )
    .map_err(|error| error.detail)
}

pub fn resolve_github_release_asset_operation(
    project: &str,
    tag: Option<&str>,
    asset_filter: Option<&str>,
    filename_override: Option<&str>,
    name_override: Option<&str>,
) -> OperationResult<GitHubFileInfo> {
    let normalized = crate::operation::paths::github_project(project)?;
    let (owner, repo) = normalized.split_once('/').expect("normalized repository");
    let url = match tag.filter(|value| !value.trim().is_empty() && *value != "latest") {
        Some(tag) => format!(
            "https://api.github.com/repos/{}/{}/releases/tags/{}",
            percent_encode_path_segment(&owner),
            percent_encode_path_segment(&repo),
            percent_encode_path_segment(tag)
        ),
        None => format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            percent_encode_path_segment(&owner),
            percent_encode_path_segment(&repo)
        ),
    };
    let json = match env::var("GITHUB_TOKEN")
        .ok()
        .filter(|value| !value.is_empty())
    {
        Some(token) => {
            http_get_to_string_with_header(&url, "Authorization", &format!("Bearer {token}"))?
        }
        None => http_get_to_string(&url)?,
    };
    let release = parse_release(&json)?;
    let asset = select_asset(&release.assets, asset_filter, filename_override)?;
    let filename = filename_override
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&asset.name)
        .to_string();
    let display_name = name_override
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| release.name.clone())
        .or_else(|| release.tag.clone())
        .unwrap_or_else(|| filename.clone());
    let hash = asset_hash(asset, &filename)?;
    Ok(GitHubFileInfo {
        name: display_name,
        filename,
        url: asset.url.clone(),
        hash_format: "sha256".to_string(),
        hash,
    })
}

fn asset_hash(asset: &GitHubAsset, filename: &str) -> OperationResult<String> {
    if let Some(hash) = &asset.sha256 {
        return Ok(hash.clone());
    }
    let temp_dir = temp_download_dir()?;
    let temp = temp_dir.join(sanitize_filename(filename));
    let result = http_get_to_file(&asset.url, &temp)
        .map_err(Error::from)
        .and_then(|()| sha256_file_hex_operation(&temp));
    let _ = fs::remove_dir_all(&temp_dir);
    result
}

pub fn normalize_project(project: &str) -> Result<String, String> {
    let (owner, repo) = parse_project(project)?;
    Ok(format!("{owner}/{repo}"))
}

fn parse_project(project: &str) -> Result<(String, String), String> {
    let trimmed = project.trim().trim_end_matches('/');
    let path = if let Some(rest) = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .or_else(|| trimmed.strip_prefix("https://www.github.com/"))
        .or_else(|| trimmed.strip_prefix("http://www.github.com/"))
    {
        rest
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Err(format!("invalid GitHub project host: {project}"));
    } else {
        trimmed
    };
    let path = path.strip_prefix('/').unwrap_or(path);
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let owner = parts
        .next()
        .ok_or_else(|| format!("invalid GitHub project: {project}"))?;
    let repo = parts
        .next()
        .ok_or_else(|| format!("invalid GitHub project: {project}"))?;
    let repo = repo.trim_end_matches(".git");
    if !is_valid_github_path_segment(owner) || !is_valid_github_path_segment(repo) {
        return Err(format!("invalid GitHub project: {project}"));
    }
    Ok((owner.to_string(), repo.to_string()))
}

fn is_valid_github_path_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn parse_release(json: &str) -> OperationResult<GitHubRelease> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|err| {
        diagnostic(
            "github_release_json",
            format!("failed to parse GitHub release JSON: {err}"),
            err.to_string(),
        )
    })?;
    let name = value
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let tag = value
        .get("tag_name")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let assets = value
        .get("assets")
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            diagnostic(
                "github_release_assets_missing",
                "GitHub release response has no assets field",
                "",
            )
        })?
        .iter()
        .filter_map(|asset| {
            Some(GitHubAsset {
                name: asset.get("name")?.as_str()?.to_string(),
                url: asset.get("browser_download_url")?.as_str()?.to_string(),
                sha256: asset
                    .get("digest")
                    .and_then(|value| value.as_str())
                    .and_then(|digest| digest.strip_prefix("sha256:"))
                    .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                    .map(str::to_ascii_lowercase),
            })
        })
        .collect::<Vec<_>>();
    if assets.is_empty() {
        return Err(diagnostic(
            "github_release_empty",
            "GitHub release has no assets",
            "",
        ));
    }
    Ok(GitHubRelease { name, tag, assets })
}

fn select_asset<'a>(
    assets: &'a [GitHubAsset],
    asset_filter: Option<&str>,
    filename_override: Option<&str>,
) -> OperationResult<&'a GitHubAsset> {
    if let Some(filename) = filename_override.filter(|value| !value.trim().is_empty()) {
        return assets
            .iter()
            .find(|asset| asset.name == filename)
            .ok_or_else(|| {
                diagnostic(
                    "github_asset_named_missing",
                    format!("GitHub release has no asset named: {filename}"),
                    filename,
                )
            });
    }
    if let Some(filter) = asset_filter.filter(|value| !value.trim().is_empty()) {
        let matches = assets
            .iter()
            .filter(|asset| asset.name.contains(filter))
            .collect::<Vec<_>>();
        return one_asset(matches, filter);
    }
    let jars = assets
        .iter()
        .filter(|asset| asset.name.ends_with(".jar"))
        .collect::<Vec<_>>();
    if jars.len() == 1 {
        return Ok(jars[0]);
    }
    if assets.len() == 1 {
        return Ok(&assets[0]);
    }
    let names = asset_names(assets.iter());
    Err(diagnostic(
        "github_assets_ambiguous",
        format!("release has multiple assets; use --asset. assets: {names}"),
        names,
    ))
}

fn one_asset<'a>(assets: Vec<&'a GitHubAsset>, filter: &str) -> OperationResult<&'a GitHubAsset> {
    match assets.as_slice() {
        [asset] => Ok(*asset),
        [] => Err(diagnostic(
            "github_asset_filter_missing",
            format!("no GitHub release asset matched: {filter}"),
            filter,
        )),
        _ => {
            let names = asset_names(assets.iter().copied());
            Err(diagnostic(
                "github_asset_filter_ambiguous",
                format!("multiple GitHub release assets matched {filter}: {names}"),
                format!("{filter}: {names}"),
            ))
        }
    }
}

fn asset_names<'a>(assets: impl Iterator<Item = &'a GitHubAsset>) -> String {
    assets
        .map(|asset| asset.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn diagnostic(key: &str, legacy: impl Into<String>, context: impl Into<String>) -> Error {
    Error::named(ErrorCode::Failed, key, legacy).context(context)
}

fn temp_download_dir() -> OperationResult<PathBuf> {
    for attempt in 0..100 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let path = env::temp_dir().join(format!(
            "bkmpw-github-{}-{nanos}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(diagnostic(
                    "github_temp_create",
                    format!("failed to create {}: {err}", path.display()),
                    format!("{}: {err}", path.display()),
                ));
            }
        }
    }
    Err(diagnostic(
        "github_temp_unique",
        "failed to create unique GitHub download temp directory",
        "",
    ))
}

fn sanitize_filename(filename: &str) -> String {
    filename
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn percent_encode_path_segment(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_api_digest_avoids_download_and_invalid_digests_use_fallback() {
        let hash = "AB".repeat(32);
        for digest in [
            serde_json::Value::Null,
            serde_json::json!(format!("sha256:{hash}")),
            serde_json::json!("sha256:1234"),
            serde_json::json!(format!("sha256:{}", "z".repeat(64))),
            serde_json::json!(format!("sha512:{hash}")),
        ] {
            let json = serde_json::json!({ "assets": [{
                "name": "mod.jar", "browser_download_url": "invalid-url", "digest": digest,
            }] });
            let release = parse_release(&json.to_string()).unwrap();
            let asset = &release.assets[0];
            if digest == format!("sha256:{hash}") {
                // An invalid URL proves the valid-digest path never tries downloading.
                assert_eq!(
                    asset_hash(asset, "mod.jar").unwrap(),
                    hash.to_ascii_lowercase()
                );
            } else {
                assert!(asset.sha256.is_none());
            }
        }
    }

    #[test]
    fn release_selection_errors_keep_legacy_text_and_stable_message_keys() {
        for (json, key, legacy) in [
            (
                "{}",
                "github_release_assets_missing",
                "GitHub release response has no assets field",
            ),
            (
                r#"{"assets":[]}"#,
                "github_release_empty",
                "GitHub release has no assets",
            ),
        ] {
            let error = parse_release(json).unwrap_err();
            assert_eq!(error.message.as_deref(), Some(key));
            assert_eq!(error.detail, legacy);
        }
        assert_eq!(
            parse_release("{").unwrap_err().message.as_deref(),
            Some("github_release_json")
        );
        let release = parse_release(r#"{"assets":[{"name":"a.jar","browser_download_url":"https://example.invalid/a"},{"name":"b.jar","browser_download_url":"https://example.invalid/b"}]}"#).unwrap();
        for (filter, filename, key, legacy) in [
            (
                None,
                None,
                "github_assets_ambiguous",
                "release has multiple assets; use --asset. assets: a.jar, b.jar",
            ),
            (
                Some("missing"),
                None,
                "github_asset_filter_missing",
                "no GitHub release asset matched: missing",
            ),
            (
                Some(".jar"),
                None,
                "github_asset_filter_ambiguous",
                "multiple GitHub release assets matched .jar: a.jar, b.jar",
            ),
            (
                None,
                Some("missing.jar"),
                "github_asset_named_missing",
                "GitHub release has no asset named: missing.jar",
            ),
        ] {
            let error = select_asset(&release.assets, filter, filename).unwrap_err();
            assert_eq!(error.message.as_deref(), Some(key));
            assert_eq!(error.detail, legacy);
            assert!(
                !error
                    .message_context
                    .as_deref()
                    .unwrap_or_default()
                    .contains("--asset")
            );
        }
        assert_eq!(
            select_asset(&release.assets, Some("a.jar"), None)
                .unwrap()
                .name,
            "a.jar"
        );
        let project = "https://example.invalid/owner/repo";
        assert_eq!(
            resolve_github_release_asset(project, None, None, None, None).unwrap_err(),
            normalize_project(project).unwrap_err()
        );
    }

    #[test]
    fn parses_owner_repo_from_url() {
        assert_eq!(
            parse_project("https://github.com/owner/repo/releases").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
    }

    #[test]
    fn rejects_non_github_urls() {
        assert!(parse_project("https://example.com/github.com/owner/repo").is_err());
        assert!(parse_project("https://notgithub.example/owner/repo").is_err());
    }

    #[test]
    fn rejects_traversal_project_segments() {
        assert!(parse_project("https://github.com/../repo").is_err());
        assert!(parse_project("owner/..").is_err());
    }

    #[test]
    fn selects_single_jar_asset() {
        let assets = vec![
            GitHubAsset {
                sha256: None,
                name: "readme.txt".to_string(),
                url: "https://example.invalid/readme.txt".to_string(),
            },
            GitHubAsset {
                sha256: None,
                name: "mod.jar".to_string(),
                url: "https://example.invalid/mod.jar".to_string(),
            },
        ];

        assert_eq!(select_asset(&assets, None, None).unwrap().name, "mod.jar");
    }

    #[test]
    fn filename_override_requires_exact_asset() {
        let assets = vec![GitHubAsset {
            sha256: None,
            name: "actual.jar".to_string(),
            url: "https://example.invalid/actual.jar".to_string(),
        }];

        let err = select_asset(&assets, None, Some("missing.jar")).unwrap_err();

        assert!(err.detail.contains("missing.jar"));
    }
}
