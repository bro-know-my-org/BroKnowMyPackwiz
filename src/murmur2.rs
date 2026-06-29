use std::fs::File;
use std::io::Read;
use std::path::Path;

const M: u32 = 0x5bd1e995;
const R: u32 = 24;
const SEED: u32 = 1;

pub fn curseforge_murmur2_file(path: &Path) -> Result<u32, String> {
    let len = filtered_len(path)?;
    let mut file =
        File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let mut hasher = Murmur2::new(len);
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
    Ok(hasher.finalize())
}

#[cfg(test)]
pub fn curseforge_murmur2(input: &[u8]) -> u32 {
    let len = input
        .iter()
        .filter(|byte| !matches!(*byte, b'\t' | b'\n' | b'\r' | b' '))
        .count();
    let mut hasher = Murmur2::new(len);
    hasher.update(input);
    hasher.finalize()
}

fn filtered_len(path: &Path) -> Result<usize, String> {
    let mut file =
        File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let mut len = 0usize;
    let mut buf = [0u8; 8192];
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        if read == 0 {
            break;
        }
        len += buf[..read]
            .iter()
            .filter(|byte| !matches!(*byte, b'\t' | b'\n' | b'\r' | b' '))
            .count();
    }
    Ok(len)
}

struct Murmur2 {
    h: u32,
    tail: [u8; 4],
    tail_len: usize,
}

impl Murmur2 {
    fn new(len: usize) -> Self {
        let len = u32::try_from(len).unwrap_or(u32::MAX);
        Self {
            h: SEED ^ len,
            tail: [0; 4],
            tail_len: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        for byte in data
            .iter()
            .copied()
            .filter(|byte| !matches!(*byte, b'\t' | b'\n' | b'\r' | b' '))
        {
            self.tail[self.tail_len] = byte;
            self.tail_len += 1;
            if self.tail_len == 4 {
                self.mix_chunk(self.tail);
                self.tail_len = 0;
            }
        }
    }

    fn finalize(mut self) -> u32 {
        match self.tail_len {
            3 => {
                self.h ^= (self.tail[2] as u32) << 16;
                self.h ^= (self.tail[1] as u32) << 8;
                self.h ^= self.tail[0] as u32;
                self.h = self.h.wrapping_mul(M);
            }
            2 => {
                self.h ^= (self.tail[1] as u32) << 8;
                self.h ^= self.tail[0] as u32;
                self.h = self.h.wrapping_mul(M);
            }
            1 => {
                self.h ^= self.tail[0] as u32;
                self.h = self.h.wrapping_mul(M);
            }
            _ => {}
        }

        self.h ^= self.h >> 13;
        self.h = self.h.wrapping_mul(M);
        self.h ^= self.h >> 15;
        self.h
    }

    fn mix_chunk(&mut self, chunk: [u8; 4]) {
        let mut k = u32::from_le_bytes(chunk);
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);

        self.h = self.h.wrapping_mul(M);
        self.h ^= k;
    }
}

#[cfg(test)]
fn murmur2(data: &[u8]) -> u32 {
    let len = data.len() as u32;
    let mut h = SEED ^ len;
    let mut chunks = data.chunks_exact(4);

    for chunk in &mut chunks {
        let mut k = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);

        h = h.wrapping_mul(M);
        h ^= k;
    }

    let tail = chunks.remainder();
    match tail.len() {
        3 => {
            h ^= (tail[2] as u32) << 16;
            h ^= (tail[1] as u32) << 8;
            h ^= tail[0] as u32;
            h = h.wrapping_mul(M);
        }
        2 => {
            h ^= (tail[1] as u32) << 8;
            h ^= tail[0] as u32;
            h = h.wrapping_mul(M);
        }
        1 => {
            h ^= tail[0] as u32;
            h = h.wrapping_mul(M);
        }
        _ => {}
    }

    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^= h >> 15;
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curseforge_variant_ignores_whitespace() {
        assert_eq!(
            curseforge_murmur2(b"abc"),
            curseforge_murmur2(b"a b\nc\r\t")
        );
    }

    #[test]
    fn murmur2_empty_is_seed_mixed() {
        assert_eq!(curseforge_murmur2(b""), 1540447798);
    }

    #[test]
    fn streaming_matches_slice_variant() {
        let mut hasher = Murmur2::new(6);
        hasher.update(b"ab");
        hasher.update(b" c\nd");
        hasher.update(b"ef");

        assert_eq!(hasher.finalize(), curseforge_murmur2(b"abcdef"));
    }
}
