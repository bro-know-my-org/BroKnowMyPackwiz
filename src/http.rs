use std::fs;
use std::io::{Read, copy};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const HTTP_TIMEOUT_SECONDS: u64 = 120;

pub fn http_get_to_file(url: &str, path: &Path) -> Result<(), String> {
    let mut response = agent()
        .get(url)
        .header("User-Agent", "bkmpw")
        .call()
        .map_err(|err| format!("HTTP GET failed for {url}: {err}"))?;
    ensure_success(url, response.status())?;
    let mut body = response.body_mut().as_reader();
    let tmp = unique_tmp_path(path);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|err| format!("failed to create {}: {err}", tmp.display()))?;
    if let Err(err) = copy(&mut body, &mut file) {
        let _ = fs::remove_file(&tmp);
        return Err(format!(
            "failed to download {url} to {}: {err}",
            path.display()
        ));
    }
    if let Err(err) = file.sync_all() {
        let _ = fs::remove_file(&tmp);
        return Err(format!("failed to flush {}: {err}", tmp.display()));
    }
    drop(file);
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!(
            "failed to move download into {}: {err}",
            path.display()
        ));
    }
    Ok(())
}

fn unique_tmp_path(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let mut extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map_or(String::new(), |value| format!("{value}."));
    extension.push_str("bkmpw-download-tmp-");
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
    path.with_extension(extension)
}

pub fn http_get_to_string_with_header(
    url: &str,
    header_name: &str,
    header_value: &str,
) -> Result<String, String> {
    let mut response = agent()
        .get(url)
        .header("User-Agent", "bkmpw")
        .header(header_name, header_value)
        .call()
        .map_err(|err| format!("HTTP GET failed for {url}: {err}"))?;
    ensure_success(url, response.status())?;
    let mut body = response.body_mut().as_reader();
    let mut text = String::new();
    body.read_to_string(&mut text)
        .map_err(|err| format!("failed to read HTTP body for {url}: {err}"))?;
    Ok(text)
}

pub fn http_get_to_string(url: &str) -> Result<String, String> {
    let mut response = agent()
        .get(url)
        .header("User-Agent", "bkmpw")
        .call()
        .map_err(|err| format!("HTTP GET failed for {url}: {err}"))?;
    ensure_success(url, response.status())?;
    let mut body = response.body_mut().as_reader();
    let mut text = String::new();
    body.read_to_string(&mut text)
        .map_err(|err| format!("failed to read HTTP body for {url}: {err}"))?;
    Ok(text)
}

fn ensure_success(url: &str, status: ureq::http::StatusCode) -> Result<(), String> {
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("HTTP GET {url} returned status {status}"))
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(HTTP_TIMEOUT_SECONDS)))
            .build()
            .into()
    });
    &AGENT
}
