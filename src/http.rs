use std::fs;
use std::io::{Read, Write, copy};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use crate::tempfiles;

const HTTP_TIMEOUT_SECONDS: u64 = 120;
const DOWNLOAD_BUFFER_SIZE: usize = 64 * 1024;

pub trait DownloadProgress: Send + Sync {
    fn reset(&self);
    fn set_total(&self, total: u64);
    fn add_bytes(&self, bytes: u64);
}

#[derive(Clone)]
pub struct HttpDownloadOptions {
    pub split_min_bytes: u64,
    pub split_chunks: usize,
    pub progress: Option<Arc<dyn DownloadProgress>>,
}

impl Default for HttpDownloadOptions {
    fn default() -> Self {
        Self {
            split_min_bytes: u64::MAX,
            split_chunks: 1,
            progress: None,
        }
    }
}

pub fn http_get_to_file(url: &str, path: &Path) -> Result<(), String> {
    http_get_to_file_with_options(url, path, &HttpDownloadOptions::default())
}

pub fn http_get_to_file_with_options(
    url: &str,
    path: &Path,
    options: &HttpDownloadOptions,
) -> Result<(), String> {
    if should_try_split(options)
        && let Some(length) = probe_content_length(url)
        && length > options.split_min_bytes
    {
        match split_http_get_to_file(url, path, length, options.split_chunks, &options.progress) {
            Ok(()) => return Ok(()),
            Err(_) => {
                if let Some(progress) = &options.progress {
                    progress.reset();
                }
            }
        }
    }
    single_http_get_to_file(url, path, &options.progress)
}

fn should_try_split(options: &HttpDownloadOptions) -> bool {
    options.split_chunks > 1 && options.split_min_bytes < u64::MAX
}

fn single_http_get_to_file(
    url: &str,
    path: &Path,
    progress: &Option<Arc<dyn DownloadProgress>>,
) -> Result<(), String> {
    let mut response = agent()
        .get(url)
        .header("User-Agent", "bkmpw")
        .call()
        .map_err(|err| format!("HTTP GET failed for {url}: {err}"))?;
    ensure_success(url, response.status())?;
    if let Some(total) = response_content_length(&response) {
        if let Some(progress) = progress {
            progress.set_total(total);
        }
    }
    let mut body = response.body_mut().as_reader();
    let tmp = unique_tmp_path(path);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|err| format!("failed to create {}: {err}", tmp.display()))?;
    if let Err(err) = copy_with_progress(&mut body, &mut file, progress.as_deref()) {
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

fn probe_content_length(url: &str) -> Option<u64> {
    let response = agent()
        .head(url)
        .header("User-Agent", "bkmpw")
        .call()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response_content_length(&response)
}

fn response_content_length(response: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    response
        .headers()
        .get("Content-Length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
}

fn split_http_get_to_file(
    url: &str,
    path: &Path,
    length: u64,
    requested_chunks: usize,
    progress: &Option<Arc<dyn DownloadProgress>>,
) -> Result<(), String> {
    let ranges = byte_ranges(length, requested_chunks);
    if ranges.len() < 2 {
        return single_http_get_to_file(url, path, progress);
    }
    if let Some(progress) = progress {
        progress.set_total(length);
    }

    let tmp = unique_tmp_path(path);
    let chunk_paths = ranges
        .iter()
        .map(|_| unique_tmp_path(path))
        .collect::<Vec<_>>();

    let mut results = Vec::new();
    thread::scope(|scope| {
        let mut handles = Vec::new();
        for (idx, ((start, end), chunk_path)) in ranges.iter().zip(chunk_paths.iter()).enumerate() {
            let chunk_path = chunk_path.clone();
            let progress = progress.clone();
            handles.push(scope.spawn(move || {
                download_range_to_file(url, &chunk_path, *start, *end, &progress)
                    .map_err(|err| format!("chunk {} ({}-{}): {err}", idx + 1, start, end))
            }));
        }
        for handle in handles {
            results.push(
                handle
                    .join()
                    .unwrap_or_else(|_| Err("split download worker panicked".to_string())),
            );
        }
    });

    if let Some(err) = results.into_iter().find_map(Result::err) {
        cleanup_paths(&chunk_paths);
        return Err(err);
    }

    let assemble_result = assemble_chunks(&chunk_paths, &tmp, length);
    cleanup_paths(&chunk_paths);
    if let Err(err) = assemble_result {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }

    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!(
            "failed to move download into {}: {err}",
            path.display()
        ));
    }
    Ok(())
}

fn byte_ranges(length: u64, requested_chunks: usize) -> Vec<(u64, u64)> {
    if length == 0 || requested_chunks <= 1 {
        return Vec::new();
    }
    let chunks =
        usize::try_from(length).map_or(requested_chunks, |length| requested_chunks.min(length));
    let chunk_size = length.div_ceil(chunks as u64);
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < length {
        let end = start.saturating_add(chunk_size - 1).min(length - 1);
        ranges.push((start, end));
        start = end + 1;
    }
    ranges
}

fn download_range_to_file(
    url: &str,
    path: &Path,
    start: u64,
    end: u64,
    progress: &Option<Arc<dyn DownloadProgress>>,
) -> Result<(), String> {
    let range = format!("bytes={start}-{end}");
    let mut response = agent()
        .get(url)
        .header("User-Agent", "bkmpw")
        .header("Range", &range)
        .call()
        .map_err(|err| format!("HTTP Range GET failed for {url}: {err}"))?;
    if response.status().as_u16() != 206 {
        return Err(format!(
            "server returned {} instead of 206 Partial Content",
            response.status()
        ));
    }
    let mut body = response.body_mut().as_reader();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| format!("failed to create {}: {err}", path.display()))?;
    let written = copy_with_progress(&mut body, &mut file, progress.as_deref()).map_err(|err| {
        format!(
            "failed to download range {range} to {}: {err}",
            path.display()
        )
    })?;
    let expected = end - start + 1;
    if written != expected {
        return Err(format!(
            "range {range} wrote {written} bytes, expected {expected}"
        ));
    }
    file.sync_all()
        .map_err(|err| format!("failed to flush {}: {err}", path.display()))
}

fn copy_with_progress(
    reader: &mut impl Read,
    writer: &mut impl Write,
    progress: Option<&dyn DownloadProgress>,
) -> std::io::Result<u64> {
    let mut buffer = [0u8; DOWNLOAD_BUFFER_SIZE];
    let mut written = 0;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        written += read as u64;
        if let Some(progress) = progress {
            progress.add_bytes(read as u64);
        }
    }
    Ok(written)
}

fn assemble_chunks(chunk_paths: &[PathBuf], tmp: &Path, expected_len: u64) -> Result<(), String> {
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp)
        .map_err(|err| format!("failed to create {}: {err}", tmp.display()))?;
    let mut written = 0;
    for chunk in chunk_paths {
        let mut input = fs::File::open(chunk)
            .map_err(|err| format!("failed to open {}: {err}", chunk.display()))?;
        written += copy(&mut input, &mut out)
            .map_err(|err| format!("failed to assemble {}: {err}", chunk.display()))?;
    }
    if written != expected_len {
        return Err(format!(
            "assembled download wrote {written} bytes, expected {expected_len}"
        ));
    }
    out.sync_all()
        .map_err(|err| format!("failed to flush {}: {err}", tmp.display()))
}

fn cleanup_paths(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn unique_tmp_path(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let mut extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map_or(String::new(), |value| format!("{value}."));
    extension.push_str(tempfiles::download_temp_marker());
    extension.push_str(tempfiles::run_id());
    extension.push('-');
    extension.push_str(&COUNTER.fetch_add(1, Ordering::Relaxed).to_string());
    path.with_extension(extension)
}

pub fn http_get_to_string_with_header(
    url: &str,
    header_name: &str,
    header_value: &str,
) -> Result<String, String> {
    read_text(url, Some((header_name, header_value)))
}

pub fn http_get_to_string(url: &str) -> Result<String, String> {
    read_text(url, None)
}

fn read_text(url: &str, header: Option<(&str, &str)>) -> Result<String, String> {
    // Metadata GETs are idempotent. Retry transient transport/body errors, not bad credentials.
    for attempt in 0..3 {
        let result = (|| {
            let mut request = agent().get(url).header("User-Agent", "bkmpw");
            if let Some((name, value)) = header {
                request = request.header(name, value);
            }
            let mut response = request.call().map_err(|error| {
                let transient = matches!(
                    error,
                    ureq::Error::StatusCode(408 | 429 | 500..=599)
                        | ureq::Error::Io(_)
                        | ureq::Error::Timeout(_)
                        | ureq::Error::HostNotFound
                        | ureq::Error::ConnectionFailed
                );
                (format!("HTTP GET failed for {url}: {error}"), transient)
            })?;
            ensure_success(url, response.status()).map_err(|error| (error, false))?;
            let mut text = String::new();
            response
                .body_mut()
                .as_reader()
                .read_to_string(&mut text)
                .map_err(|error| (format!("failed to read HTTP body for {url}: {error}"), true))?;
            Ok(text)
        })();
        match result {
            Ok(text) => return Ok(text),
            Err((error, transient)) => {
                if !transient || attempt == 2 {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(250 * (1 << attempt)));
            }
        }
    }
    unreachable!("last attempt returns its result")
}

fn ensure_success(url: &str, status: ureq::http::StatusCode) -> Result<(), String> {
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("HTTP GET {url} returned status {status}"))
    }
}

pub(crate) fn agent() -> &'static ureq::Agent {
    static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(HTTP_TIMEOUT_SECONDS)))
            .build()
            .into()
    });
    &AGENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn metadata_get_retries_transient_failures_but_not_authentication_errors() {
        for status in [503, 0, 401] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                let expected = if status != 401 { 2 } else { 1 };
                let mut count = 0;
                while count < expected && std::time::Instant::now() < deadline {
                    let (mut stream, _) = match listener.accept() {
                        Ok(value) => value,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        Err(error) => panic!("{error}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut request = [0; 4096];
                    let size = stream.read(&mut request).unwrap();
                    assert!(
                        String::from_utf8_lossy(&request[..size])
                            .to_ascii_lowercase()
                            .contains("x-test: fixture")
                    );
                    if status == 0 && count == 0 {
                        // A truncated body reproduces the preview's unexpected EOF failure.
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nok"
                        )
                        .unwrap();
                        count += 1;
                        continue;
                    }
                    let response = if count == 0 { status } else { 200 };
                    write!(stream, "HTTP/1.1 {response} Test\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
                    count += 1;
                }
                count
            });
            let result = http_get_to_string_with_header(
                &format!("http://{address}/metadata"),
                "x-test",
                "fixture",
            );
            if status != 401 {
                assert_eq!(result.unwrap(), "ok");
            } else {
                assert!(result.unwrap_err().contains("401"));
            }
            assert_eq!(server.join().unwrap(), if status != 401 { 2 } else { 1 });
        }
    }

    #[test]
    fn byte_ranges_split_evenly_enough() {
        assert_eq!(byte_ranges(10, 3), vec![(0, 3), (4, 7), (8, 9)]);
    }

    #[test]
    fn byte_ranges_never_creates_empty_ranges() {
        assert_eq!(byte_ranges(2, 4), vec![(0, 0), (1, 1)]);
    }

    #[test]
    fn byte_ranges_handles_lengths_larger_than_usize() {
        let ranges = byte_ranges(u64::MAX, 4);

        assert_eq!(ranges.len(), 4);
        assert_eq!(ranges.first().map(|(start, _)| *start), Some(0));
        assert_eq!(ranges.last().map(|(_, end)| *end), Some(u64::MAX - 1));
    }

    #[test]
    fn split_download_uses_ranges_and_assembles_file() {
        let data = b"abcdefghijklmnopqrstuvwxyz0123456789".to_vec();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_data = data.clone();
        let server = thread::spawn(move || {
            for _ in 0..5 {
                let (mut stream, _) = listener.accept().unwrap();
                handle_test_request(&mut stream, &server_data);
            }
        });
        let path = unique_test_path("split-download.bin");

        http_get_to_file_with_options(
            &format!("http://{address}/file.bin"),
            &path,
            &HttpDownloadOptions {
                split_min_bytes: 16,
                split_chunks: 4,
                progress: None,
            },
        )
        .unwrap();

        assert_eq!(fs::read(&path).unwrap(), data);
        let _ = fs::remove_file(path);
        server.join().unwrap();
    }

    fn handle_test_request(stream: &mut TcpStream, data: &[u8]) {
        let mut request = Vec::new();
        let mut buffer = [0; 512];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&request);
        if text.starts_with("HEAD ") {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                data.len()
            )
            .unwrap();
            return;
        }
        let (start, end) = parse_test_range(&text);
        let body = &data[start..=end];
        write!(
            stream,
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
            body.len(),
            start,
            end,
            data.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
    }

    fn parse_test_range(request: &str) -> (usize, usize) {
        let line = request
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with("range:"))
            .unwrap();
        let value = line.split_once("bytes=").unwrap().1;
        let (start, end) = value.split_once('-').unwrap();
        (start.trim().parse().unwrap(), end.trim().parse().unwrap())
    }

    fn unique_test_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("bkmpw-{name}-{nanos}"))
    }
}
