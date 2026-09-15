//! Cancellable streaming into private staging files, without terminal output.
use super::{Control, Error, ErrorCode, Event, Result};
use sha2::Digest;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::{Duration, Instant},
};

pub struct Hashes {
    pub sha1: String,
    pub sha256: String,
    pub size: u64,
}
impl Hashes {
    pub fn verify(&self, format: &str, expected: &str) -> Result<()> {
        let actual = match format {
            "sha1" => &self.sha1,
            "sha256" => &self.sha256,
            _ => return Err(Error::key(ErrorCode::Invalid, "unsupported_hash_format")),
        };
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(Error::key(ErrorCode::Failed, "download_hash_mismatch"));
        }
        Ok(())
    }
}
pub fn hash(path: &Path, control: &Control) -> Result<Hashes> {
    let mut file = fs::File::open(path)?;
    let length = file.metadata()?.len();
    stream(&mut file, &mut std::io::sink(), Some(length), control)
}
pub fn copy(source: &Path, target: &Path, control: &Control) -> Result<Hashes> {
    control.check()?;
    let mut source = fs::File::open(source)?;
    let length = source.metadata()?.len();
    staged(target, |file| {
        stream(&mut source, file, Some(length), control)
    })
}
pub fn download(
    url: &str,
    target: &Path,
    expected: Option<(&str, &str)>,
    control: &Control,
) -> Result<Hashes> {
    control.check()?;
    let uri = url
        .parse::<ureq::http::Uri>()
        .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?;
    if !matches!(uri.scheme_str(), Some("https" | "http")) || uri.authority().is_none() {
        return Err(Error::key(ErrorCode::Invalid, "http_url_required"));
    }
    let mut response = crate::http::agent()
        .get(url)
        .header("User-Agent", "bkmpw")
        .call()
        .map_err(|e| Error::new(ErrorCode::Failed, e.to_string()))?;
    control.check()?;
    if !response.status().is_success() {
        return Err(Error::new(
            ErrorCode::Failed,
            format!("HTTP {}", response.status()),
        ));
    }
    let length = response
        .headers()
        .get("Content-Length")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.parse().ok());
    staged(target, |file| {
        let hashes = stream(&mut response.body_mut().as_reader(), file, length, control)?;
        if let Some((format, hash)) = expected {
            hashes.verify(format, hash)?;
        }
        Ok(hashes)
    })
}
fn staged(target: &Path, work: impl FnOnce(&mut fs::File) -> Result<Hashes>) -> Result<Hashes> {
    // Never overwrite an existing staging object, including a symlink.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let permissions = file.metadata()?.permissions();
    let result = work(&mut file).and_then(|hashes| {
        file.sync_all()?;
        super::artifacts::record_payload(target, &hashes.sha256, &permissions);
        Ok(hashes)
    });
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(target);
    }
    result
}
pub fn stream(
    reader: &mut impl Read,
    writer: &mut impl Write,
    total: Option<u64>,
    control: &Control,
) -> Result<Hashes> {
    let mut sha1 = sha1::Sha1::new();
    let mut sha256 = sha2::Sha256::new();
    let mut bytes = [0; 64 * 1024];
    let mut current = 0u64;
    let mut last = Instant::now();
    loop {
        control.check()?;
        let count = reader.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        control.check()?;
        writer.write_all(&bytes[..count])?;
        sha1.update(&bytes[..count]);
        sha256.update(&bytes[..count]);
        current = current
            .checked_add(count as u64)
            .ok_or_else(|| Error::key(ErrorCode::Invalid, "file_size_overflow"))?;
        if last.elapsed() >= Duration::from_millis(100) {
            control.emit(Event::Progress {
                label: "transfer_bytes".into(),
                current,
                total,
            });
            last = Instant::now();
        }
    }
    control.check()?;
    if total.is_some_and(|length| length != current) {
        return Err(Error::key(ErrorCode::Failed, "download_size_mismatch"));
    }
    control.emit(Event::Progress {
        label: "transfer_bytes".into(),
        current,
        total,
    });
    Ok(Hashes {
        sha1: hex(&sha1.finalize()),
        sha256: hex(&sha256.finalize()),
        size: current,
    })
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_checks_hashes_length_and_cancellation_at_chunk_boundaries() {
        let hashes = stream(
            &mut &b"abc"[..],
            &mut std::io::sink(),
            Some(3),
            &Control::default(),
        )
        .unwrap();
        hashes
            .verify(
                "sha256",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            )
            .unwrap();
        hashes
            .verify("sha1", "a9993e364706816aba3e25717850c26c9cd0d89d")
            .unwrap();
        assert_eq!(hashes.size, 3);
        assert!(
            stream(
                &mut &b"abc"[..],
                &mut std::io::sink(),
                Some(9),
                &Control::default()
            )
            .is_err()
        );
        struct Cancels {
            control: Control,
        }
        impl Read for Cancels {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                buffer[0] = b'a';
                self.control.cancel();
                Ok(1)
            }
        }
        let control = Control::default();
        let mut output = Vec::new();
        assert!(
            stream(
                &mut Cancels {
                    control: control.clone()
                },
                &mut output,
                None,
                &control
            )
            .is_err()
        );
        assert!(output.is_empty());
    }
    #[test]
    fn failed_transfer_removes_its_own_partial_file_and_preserves_existing_files() {
        let dir = std::env::temp_dir().join(super::super::durable::unique_id());
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("asset");
        let result = staged(&target, |file| {
            file.write_all(b"partial")?;
            Err(Error::key(ErrorCode::Failed, "injected"))
        });
        assert!(result.is_err() && !target.exists());
        fs::write(&target, b"original").unwrap();
        assert!(staged(&target, |_| unreachable!()).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"original");
        fs::remove_dir_all(dir).unwrap();
    }
}
