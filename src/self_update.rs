use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::http::{http_get_to_file, http_get_to_string, http_get_to_string_with_header};
use crate::sha256::sha256_file_hex;

const DEFAULT_REPO: &str = "bro-know-my-org/BroKnowMyPackwiz";

#[derive(Debug, Clone)]
pub struct SelfUpdateOptions {
    pub repo: String,
    pub tag: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct SelfUpdatePlan {
    pub tag: String,
    pub asset_name: String,
    pub asset_url: String,
    pub checksum_url: Option<String>,
}

#[derive(Debug, Clone)]
struct Release {
    tag: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Clone)]
struct Asset {
    name: String,
    url: String,
}

pub fn default_repo() -> &'static str {
    DEFAULT_REPO
}

pub fn plan_update(options: &SelfUpdateOptions) -> Result<SelfUpdatePlan, String> {
    let release = fetch_release(&options.repo, options.tag.as_deref())?;
    let asset_name = current_asset_name(&release.tag)?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| {
            format!(
                "release {} has no asset for this platform: {asset_name}",
                release.tag
            )
        })?;
    let checksum_url = release
        .assets
        .iter()
        .find(|candidate| candidate.name == format!("{asset_name}.sha256"))
        .map(|asset| asset.url.clone());
    Ok(SelfUpdatePlan {
        tag: release.tag,
        asset_name,
        asset_url: asset.url.clone(),
        checksum_url,
    })
}

pub fn update_self(options: &SelfUpdateOptions, current_version: &str) -> Result<(), String> {
    if is_managed_by_npm() {
        return Err(
            "bkmpw was installed by npm; use `npm update -g @bro-know-my/packwiz` instead"
                .to_string(),
        );
    }

    let plan = plan_update(options)?;
    let latest_version = plan.tag.trim_start_matches('v');
    if latest_version == current_version && options.tag.is_none() {
        println!("bkmpw is already up to date ({current_version})");
        return Ok(());
    }

    println!(
        "latest release: {} ({})",
        plan.tag,
        if latest_version == current_version {
            "same version"
        } else {
            latest_version
        }
    );
    println!("asset: {}", plan.asset_name);
    if options.dry_run {
        println!("dry run: would download {}", plan.asset_url);
        return Ok(());
    }

    let current_exe =
        env::current_exe().map_err(|err| format!("failed to locate current executable: {err}"))?;
    let temp = temp_download_path(&plan.asset_name)?;
    http_get_to_file(&plan.asset_url, &temp)?;
    let checksum_url = plan.checksum_url.as_deref().ok_or_else(|| {
        let _ = fs::remove_file(&temp);
        format!(
            "no checksum asset found for {}; refusing to install unverified binary",
            plan.asset_name
        )
    })?;
    verify_checksum(checksum_url, &temp)?;
    install_downloaded_binary(&temp, &current_exe)?;
    println!("update staged: {}", current_exe.display());
    Ok(())
}

fn is_managed_by_npm() -> bool {
    env::var("BKMPW_MANAGED_BY_NPM")
        .ok()
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

fn fetch_release(repo: &str, tag: Option<&str>) -> Result<Release, String> {
    let (owner, repo) = parse_repo(repo)?;
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
    parse_release(&json)
}

fn parse_release(json: &str) -> Result<Release, String> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| format!("failed to parse GitHub release JSON: {err}"))?;
    let tag = value
        .get("tag_name")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "GitHub release response has no tag_name".to_string())?
        .to_string();
    let assets = value
        .get("assets")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "GitHub release response has no assets field".to_string())?
        .iter()
        .filter_map(|asset| {
            Some(Asset {
                name: asset.get("name")?.as_str()?.to_string(),
                url: asset.get("browser_download_url")?.as_str()?.to_string(),
            })
        })
        .collect::<Vec<_>>();
    if assets.is_empty() {
        return Err("GitHub release has no assets".to_string());
    }
    Ok(Release { tag, assets })
}

fn parse_repo(repo: &str) -> Result<(String, String), String> {
    let trimmed = repo.trim().trim_end_matches('/');
    let path = if let Some(rest) = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .or_else(|| trimmed.strip_prefix("https://www.github.com/"))
        .or_else(|| trimmed.strip_prefix("http://www.github.com/"))
    {
        rest
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Err(format!("invalid GitHub repo host: {repo}"));
    } else {
        trimmed
    };
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let owner = parts
        .next()
        .ok_or_else(|| format!("invalid GitHub repo: {repo}"))?;
    let name = parts
        .next()
        .ok_or_else(|| format!("invalid GitHub repo: {repo}"))?;
    let name = name.trim_end_matches(".git");
    if !is_valid_github_path_segment(owner) || !is_valid_github_path_segment(name) {
        return Err(format!("invalid GitHub repo: {repo}"));
    }
    Ok((owner.to_string(), name.to_string()))
}

fn is_valid_github_path_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn current_asset_name(tag: &str) -> Result<String, String> {
    let target = current_target()?;
    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
    Ok(format!("bkmpw-{tag}-{target}{exe_suffix}"))
}

fn current_target() -> Result<&'static str, String> {
    match (env::consts::OS, env::consts::ARCH) {
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        (os, arch) => Err(format!("unsupported platform for self-update: {os}/{arch}")),
    }
}

fn verify_checksum(url: &str, path: &Path) -> Result<(), String> {
    let text = http_get_to_string(url)?;
    let expected = text
        .split_whitespace()
        .next()
        .ok_or_else(|| format!("empty checksum response from {url}"))?;
    let actual = sha256_file_hex(path)?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        let _ = fs::remove_file(path);
        Err(format!(
            "checksum mismatch for {}: expected {expected}, got {actual}",
            path.display()
        ))
    }
}

fn temp_download_path(asset_name: &str) -> Result<PathBuf, String> {
    let mut path = env::temp_dir();
    let safe_name = asset_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    path.push(format!(
        "bkmpw-self-update-{}-{safe_name}",
        std::process::id()
    ));
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|err| format!("failed to remove old temp file {}: {err}", path.display()))?;
    }
    Ok(path)
}

#[cfg(windows)]
fn install_downloaded_binary(downloaded: &Path, current_exe: &Path) -> Result<(), String> {
    let staged = current_exe.with_extension("exe.bkmpw-new");
    if staged.exists() {
        fs::remove_file(&staged)
            .map_err(|err| format!("failed to remove old staged update: {err}"))?;
    }
    fs::rename(downloaded, &staged)
        .map_err(|err| format!("failed to stage update {}: {err}", staged.display()))?;
    let script = env::temp_dir().join(format!("bkmpw-self-update-{}.cmd", std::process::id()));
    let staged = batch_escape_path(&staged);
    let current_exe = batch_escape_path(current_exe);
    let script_text = format!(
        "@echo off\r\n\
         timeout /t 1 /nobreak >NUL\r\n\
         move /Y \"{}\" \"{}\" >NUL 2>&1 || echo bkmpw self-update failed: could not replace the running executable\r\n\
         del \"%~f0\"\r\n",
        staged, current_exe
    );
    fs::write(&script, script_text)
        .map_err(|err| format!("failed to write update script {}: {err}", script.display()))?;
    Command::new("cmd")
        .args(["/C", "start", "", "/MIN", "cmd", "/C"])
        .arg(&script)
        .spawn()
        .map_err(|err| format!("failed to launch update script: {err}"))?;
    println!("Windows will replace the running executable after this process exits.");
    Ok(())
}

fn batch_escape_path(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('^', "^^")
        .replace('%', "%%")
        .replace('&', "^&")
        .replace('|', "^|")
        .replace('<', "^<")
        .replace('>', "^>")
}

#[cfg(unix)]
fn install_downloaded_binary(downloaded: &Path, current_exe: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(downloaded)
        .map_err(|err| format!("failed to stat {}: {err}", downloaded.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(downloaded, permissions)
        .map_err(|err| format!("failed to make update executable: {err}"))?;
    match fs::rename(downloaded, current_exe) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(18) => {
            fs::copy(downloaded, current_exe).map_err(|err| {
                format!("failed to copy update to {}: {err}", current_exe.display())
            })?;
            fs::remove_file(downloaded)
                .map_err(|err| format!("failed to remove {}: {err}", downloaded.display()))?;
            Ok(())
        }
        Err(err) => Err(format!(
            "failed to replace {} with {}: {err}",
            current_exe.display(),
            downloaded.display()
        )),
    }
}

#[cfg(not(any(windows, unix)))]
fn install_downloaded_binary(_downloaded: &Path, _current_exe: &Path) -> Result<(), String> {
    Err("self-update is not supported on this platform".to_string())
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
    fn parses_release_assets() {
        let json = r#"{
            "tag_name": "v1.2.3",
            "assets": [
                {
                    "name": "bkmpw-v1.2.3-x86_64-pc-windows-msvc.exe",
                    "browser_download_url": "https://example.invalid/bkmpw.exe"
                }
            ]
        }"#;

        let release = parse_release(json).unwrap();

        assert_eq!(release.tag, "v1.2.3");
        assert_eq!(
            release.assets[0].name,
            "bkmpw-v1.2.3-x86_64-pc-windows-msvc.exe"
        );
    }

    #[test]
    fn parses_repo_url() {
        assert_eq!(
            parse_repo("https://github.com/owner/repo.git").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
    }

    #[test]
    fn rejects_traversal_repo_segments() {
        assert!(parse_repo("https://github.com/../repo").is_err());
        assert!(parse_repo("owner/..").is_err());
    }

    #[test]
    fn escapes_batch_path_metacharacters() {
        let escaped = batch_escape_path(Path::new(r"C:\Temp & 100%\bkmpw^.exe"));

        assert_eq!(escaped, r"C:\Temp ^& 100%%\bkmpw^^.exe");
    }
}
