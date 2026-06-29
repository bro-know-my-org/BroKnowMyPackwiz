use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha512 as Sha512Hasher};

pub fn sha512_file_hex(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let mut hasher = Sha512::new();
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
pub struct Sha512 {
    inner: Sha512Hasher,
}

impl Sha512 {
    pub fn new() -> Self {
        Self {
            inner: Sha512Hasher::new(),
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finalize(self) -> [u8; 64] {
        self.inner.finalize().into()
    }
}

fn to_hex(bytes: &[u8; 64]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(128);
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
    fn sha512_empty() {
        let hash = Sha512::new().finalize();
        assert_eq!(
            to_hex(&hash),
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce\
             47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
                .replace(' ', "")
        );
    }

    #[test]
    fn sha512_abc() {
        let mut sha = Sha512::new();
        sha.update(b"abc");
        assert_eq!(
            to_hex(&sha.finalize()),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
             2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
                .replace(' ', "")
        );
    }
}
