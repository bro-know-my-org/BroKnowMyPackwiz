use std::fs::File;
use std::io::Read;
use std::path::Path;

use ::sha1::{Digest, Sha1 as Sha1Hasher};

pub fn sha1_file_hex(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let mut hasher = Sha1::new();
    let mut buf = [0u8; 8192];
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(to_hex(&hasher.finalize()))
}

#[derive(Clone)]
pub struct Sha1 {
    inner: Sha1Hasher,
}

impl Sha1 {
    pub fn new() -> Self {
        Self {
            inner: Sha1Hasher::new(),
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finalize(self) -> [u8; 20] {
        self.inner.finalize().into()
    }
}

fn to_hex(bytes: &[u8; 20]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(40);
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
    fn sha1_empty() {
        let hash = Sha1::new().finalize();
        assert_eq!(to_hex(&hash), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn sha1_abc() {
        let mut sha = Sha1::new();
        sha.update(b"abc");
        assert_eq!(
            to_hex(&sha.finalize()),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }
}
