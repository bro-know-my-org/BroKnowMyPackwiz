use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256 as Sha256Hasher};

pub fn sha256_file_hex(path: &Path) -> Result<String, String> {
    sha256_file_hex_operation(path).map_err(|error| error.detail)
}

pub fn sha256_file_hex_operation(path: &Path) -> crate::operation::Result<String> {
    use crate::operation::{Error, ErrorCode};
    let mut file = File::open(path).map_err(|err| {
        Error::named(
            ErrorCode::Failed,
            "open_file_failed",
            format!("failed to open {}: {err}", path.display()),
        )
        .context(format!("{}: {err}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let read = file.read(&mut buf).map_err(|err| {
            Error::named(
                ErrorCode::Failed,
                "read_file_failed",
                format!("failed to read {}: {err}", path.display()),
            )
            .context(format!("{}: {err}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(to_hex(&hasher.finalize()))
}

#[derive(Clone)]
pub struct Sha256 {
    inner: Sha256Hasher,
}

impl Sha256 {
    pub fn new() -> Self {
        Self {
            inner: Sha256Hasher::new(),
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finalize(self) -> [u8; 32] {
        self.inner.finalize().into()
    }
}

fn to_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_empty() {
        let hash = Sha256::new().finalize();
        assert_eq!(
            to_hex(&hash),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_abc() {
        let mut sha = Sha256::new();
        sha.update(b"abc");
        assert_eq!(
            to_hex(&sha.finalize()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
