use super::{
    Control, Error, ErrorCode, Event, Result, durable,
    edit::{self, Attachment, Draft},
    preview::Guard,
    transfer,
};
use crate::config::ProjectConfig;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Source {
    Url(String),
    CurseForge {
        project: u64,
        file: u64,
        filename: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Download {
    pub relative: String,
    pub expected: Option<String>,
    pub source: Source,
    pub hash_format: String,
    pub hash: String,
    pub preserve: bool,
}
pub fn execute(
    root: &Path,
    state: &Path,
    task: &Path,
    drafts: &[Draft],
    downloads: &[Download],
    guard: &Guard,
    control: &Control,
) -> Result<()> {
    guard.validate(root, control)?;
    let config = ProjectConfig::load_operation(root)?;
    let incoming = task.join("incoming");
    fs::create_dir_all(&incoming)?;
    let mut files = Vec::new();
    for (index, download) in downloads.iter().enumerate() {
        control.check()?;
        crate::operation::paths::relative(&download.relative)?;
        let current = root.join(&download.relative);
        if durable::fingerprint(&current)? != download.expected {
            return Err(Error::new(ErrorCode::Conflict, &download.relative));
        }
        if current.exists()
            && (download.preserve
                || transfer::hash(&current, control)?
                    .verify(&download.hash_format, &download.hash)
                    .is_ok())
        {
            continue;
        }
        control.emit(Event::Phase("downloading".into()));
        control.emit(Event::Log(download.relative.clone()));
        let target = incoming.join(index.to_string());
        let mut last = None;
        let mut hashes = None;
        for attempt in 0..=config.install.retries {
            control.check()?;
            let result = urls(&download.source, &config).and_then(|urls| {
                let mut error = None;
                for url in urls {
                    control.check()?;
                    match transfer::download(
                        &url,
                        &target,
                        Some((&download.hash_format, &download.hash)),
                        control,
                    ) {
                        Ok(hashes) => return Ok(hashes),
                        Err(e) if e.code == ErrorCode::Cancelled => return Err(e),
                        Err(e) => error = Some(e),
                    }
                }
                Err(error.unwrap_or_else(|| Error::key(ErrorCode::Failed, "download_url_missing")))
            });
            match result {
                Ok(value) => {
                    hashes = Some(value);
                    break;
                }
                Err(error) if error.code == ErrorCode::Cancelled => return Err(error),
                Err(error) => last = Some(error),
            }
            if attempt < config.install.retries {
                control.emit(Event::Phase("retrying".into()));
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(config.install.retry_delay_seconds) {
                    control.check()?;
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
        let hashes = hashes.ok_or_else(|| {
            last.unwrap_or_else(|| Error::key(ErrorCode::Failed, "download_failed"))
        })?;
        files.push(Attachment {
            relative: download.relative.clone(),
            source: target.clone(),
            expected: download.expected.clone(),
            prepared: durable::fingerprint(&target)?.unwrap(),
            sha256: hashes.sha256,
        });
    }
    // Revalidate under the write lock; downloads have touched only private state.
    edit::execute_files(root, state, task, drafts, &files, Some(guard), control)
}
fn urls(source: &Source, config: &ProjectConfig) -> Result<Vec<String>> {
    match source {
        Source::Url(url) => Ok(vec![url.clone()]),
        Source::CurseForge {
            project,
            file,
            filename,
        } => {
            let primary = crate::curseforge::resolve_download_url(
                crate::curseforge::api_key(config).as_deref(),
                *project,
                *file,
            );
            let fallback = config
                .curseforge
                .cdn_fallback
                .then(|| crate::curseforge::cdn_download_url(*file, filename))
                .flatten();
            match primary {
                Ok(url) => {
                    let mut urls = vec![url];
                    if let Some(fallback) = fallback {
                        if !urls.contains(&fallback) {
                            urls.push(fallback);
                        }
                    }
                    Ok(urls)
                }
                Err(_) if fallback.is_some() => Ok(vec![fallback.unwrap()]),
                Err(error) => Err(Error::from(error)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    #[test]
    fn a_later_download_failure_preserves_all_original_files_and_metadata() {
        for fail_second in [true, false] {
            let base = std::env::temp_dir().join(durable::unique_id());
            fs::create_dir_all(base.join("pack/mods")).unwrap();
            fs::create_dir_all(base.join("pack/.pw")).unwrap();
            let base = fs::canonicalize(base).unwrap();
            let root = base.join("pack");
            fs::write(root.join("pack.toml"), "name = \"Test\"\n").unwrap();
            fs::write(root.join(".pw/config.toml"), "[install]\nretries = 0\n").unwrap();
            fs::write(
                root.join("mods/a.pw.toml"),
                "name = \"Old\"\nfilename = \"a.jar\"\n",
            )
            .unwrap();
            fs::write(root.join("mods/a.jar"), b"old").unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let end = Instant::now() + Duration::from_secs(5);
                let mut served = 0;
                while served < 2 && Instant::now() < end {
                    let (mut stream, _) = match listener.accept() {
                        Ok(connection) => connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        Err(error) => panic!("{error}"),
                    };
                    // macOS may inherit the listener's nonblocking mode.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut request = [0; 4096];
                    stream.read(&mut request).unwrap();
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nnew",
                        )
                        .unwrap();
                    served += 1;
                }
                served
            });
            let control = Control::default();
            let hash = transfer::stream(&mut &b"new"[..], &mut std::io::sink(), Some(3), &control)
                .unwrap()
                .sha256;
            let mut guard = Guard::capture(&root, &control).unwrap();
            guard.watch(&root, "mods/a.jar").unwrap();
            guard.watch(&root, "mods/b.jar").unwrap();
            let (mut doc, expected) = edit::document(&root, "mods/a.pw.toml").unwrap();
            edit::set(&mut doc, &["name"], toml_edit::Value::from("New")).unwrap();
            let state = base.join("state");
            let drafts = [edit::draft(&state, "mods/a.pw.toml", expected, &doc).unwrap()];
            let downloads: Vec<_> = ["mods/a.jar", "mods/b.jar"]
                .into_iter()
                .enumerate()
                .map(|(i, path)| Download {
                    relative: path.into(),
                    expected: durable::fingerprint(&root.join(path)).unwrap(),
                    source: Source::Url(format!("http://{address}/{i}")),
                    hash_format: "sha256".into(),
                    hash: if i == 1 && fail_second {
                        "0".repeat(64)
                    } else {
                        hash.clone()
                    },
                    preserve: false,
                })
                .collect();
            let result = execute(
                &root,
                &state,
                &base.join("task"),
                &drafts,
                &downloads,
                &guard,
                &control,
            );
            assert_eq!(result.is_err(), fail_second);
            assert_eq!(server.join().unwrap(), 2);
            if fail_second {
                assert_eq!(fs::read(root.join("mods/a.jar")).unwrap(), b"old");
                assert!(!root.join("mods/b.jar").exists());
                assert!(
                    fs::read_to_string(root.join("mods/a.pw.toml"))
                        .unwrap()
                        .contains("Old")
                );
            } else {
                assert_eq!(fs::read(root.join("mods/a.jar")).unwrap(), b"new");
                assert_eq!(fs::read(root.join("mods/b.jar")).unwrap(), b"new");
            }
            fs::remove_dir_all(base).unwrap();
        }
    }
}
