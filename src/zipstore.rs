use std::fs;
use std::io::{Read, Seek, Write};
use std::path::Path;

#[derive(Debug, Clone)]
struct CentralEntry {
    name: String,
    crc32: u32,
    size: u32,
    offset: u32,
}

pub struct ZipStore<W: Write + Seek> {
    writer: W,
    entries: Vec<CentralEntry>,
}

impl<W: Write + Seek> ZipStore<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            entries: Vec::new(),
        }
    }

    pub fn add_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let crc32 = crc32(bytes);
        let size =
            u32::try_from(bytes.len()).map_err(|_| format!("zip entry too large: {name}"))?;
        self.start_entry(name, crc32, size)?;
        self.writer.write_all(bytes).map_err(write_err)?;
        Ok(())
    }

    pub fn add_file(&mut self, name: &str, path: &Path) -> Result<(), String> {
        let (crc32, size) = crc32_file(path)?;
        let size = u32::try_from(size)
            .map_err(|_| format!("zip entry too large: {} ({})", name, path.display()))?;
        self.start_entry(name, crc32, size)?;
        let mut file = fs::File::open(path)
            .map_err(|err| format!("failed to open {}: {err}", path.display()))?;
        std::io::copy(&mut file, &mut self.writer)
            .map_err(|err| format!("failed to write {} to zip: {err}", path.display()))?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<(), String> {
        let central_start = self.pos_u32()?;
        for entry in &self.entries {
            let name_bytes = entry.name.as_bytes();
            let name_len = u16::try_from(name_bytes.len())
                .map_err(|_| format!("zip filename too long: {}", entry.name))?;
            write_u32(&mut self.writer, 0x0201_4b50)?;
            write_u16(&mut self.writer, 20)?;
            write_u16(&mut self.writer, 20)?;
            write_u16(&mut self.writer, 0x0800)?;
            write_u16(&mut self.writer, 0)?;
            write_u16(&mut self.writer, 0)?;
            write_u16(&mut self.writer, 0)?;
            write_u32(&mut self.writer, entry.crc32)?;
            write_u32(&mut self.writer, entry.size)?;
            write_u32(&mut self.writer, entry.size)?;
            write_u16(&mut self.writer, name_len)?;
            write_u16(&mut self.writer, 0)?;
            write_u16(&mut self.writer, 0)?;
            write_u16(&mut self.writer, 0)?;
            write_u16(&mut self.writer, 0)?;
            write_u32(&mut self.writer, 0)?;
            write_u32(&mut self.writer, entry.offset)?;
            self.writer.write_all(name_bytes).map_err(write_err)?;
        }
        let central_end = self.pos_u32()?;
        let central_size = central_end
            .checked_sub(central_start)
            .ok_or_else(|| "invalid zip central directory size".to_string())?;
        let entry_count = u16::try_from(self.entries.len())
            .map_err(|_| "too many files for simple zip writer".to_string())?;
        write_u32(&mut self.writer, 0x0605_4b50)?;
        write_u16(&mut self.writer, 0)?;
        write_u16(&mut self.writer, 0)?;
        write_u16(&mut self.writer, entry_count)?;
        write_u16(&mut self.writer, entry_count)?;
        write_u32(&mut self.writer, central_size)?;
        write_u32(&mut self.writer, central_start)?;
        write_u16(&mut self.writer, 0)?;
        self.writer.flush().map_err(write_err)?;
        Ok(())
    }

    fn pos_u32(&mut self) -> Result<u32, String> {
        let pos = self
            .writer
            .stream_position()
            .map_err(|err| format!("failed to read zip position: {err}"))?;
        u32::try_from(pos).map_err(|_| "zip is too large for simple writer".to_string())
    }

    fn start_entry(&mut self, name: &str, crc32: u32, size: u32) -> Result<(), String> {
        validate_zip_name(name)?;
        if self.entries.iter().any(|entry| entry.name == name) {
            return Err(format!("duplicate zip entry: {name}"));
        }
        let offset = self.pos_u32()?;
        let name_bytes = name.as_bytes();
        let name_len = u16::try_from(name_bytes.len())
            .map_err(|_| format!("zip filename too long: {name}"))?;

        write_u32(&mut self.writer, 0x0403_4b50)?;
        write_u16(&mut self.writer, 20)?;
        write_u16(&mut self.writer, 0x0800)?;
        write_u16(&mut self.writer, 0)?;
        write_u16(&mut self.writer, 0)?;
        write_u16(&mut self.writer, 0)?;
        write_u32(&mut self.writer, crc32)?;
        write_u32(&mut self.writer, size)?;
        write_u32(&mut self.writer, size)?;
        write_u16(&mut self.writer, name_len)?;
        write_u16(&mut self.writer, 0)?;
        self.writer.write_all(name_bytes).map_err(write_err)?;
        self.entries.push(CentralEntry {
            name: name.to_string(),
            crc32,
            size,
            offset,
        });
        Ok(())
    }
}

fn validate_zip_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains('\\')
        || name.contains(':')
        || name.chars().any(char::is_control)
    {
        return Err(format!("unsafe zip entry name: {name}"));
    }
    for part in name.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(format!("unsafe zip entry name: {name}"));
        }
    }
    Ok(())
}

fn write_u16<W: Write>(writer: &mut W, value: u16) -> Result<(), String> {
    writer.write_all(&value.to_le_bytes()).map_err(write_err)
}

fn write_u32<W: Write>(writer: &mut W, value: u32) -> Result<(), String> {
    writer.write_all(&value.to_le_bytes()).map_err(write_err)
}

fn write_err(err: std::io::Error) -> String {
    format!("failed to write zip: {err}")
}

fn crc32(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes)
}

fn crc32_file(path: &Path) -> Result<(u32, u64), String> {
    let mut file =
        fs::File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let mut hasher = crc32fast::Hasher::new();
    let mut size = 0u64;
    let mut buf = [0u8; 8192];
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hasher.update(&buf[..read]);
    }
    Ok((hasher.finalize(), size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_value() {
        assert_eq!(crc32(b"abc"), 0x3524_41c2);
    }

    #[test]
    fn rejects_unsafe_zip_names() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        let mut zip = ZipStore::new(&mut cursor);

        assert!(zip.add_bytes("../evil.txt", b"x").is_err());
        assert!(zip.add_bytes("overrides\\evil.txt", b"x").is_err());
    }

    #[test]
    fn rejects_duplicate_zip_entries() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        let mut zip = ZipStore::new(&mut cursor);

        zip.add_bytes("manifest.json", b"{}").unwrap();
        assert!(zip.add_bytes("manifest.json", b"{}").is_err());
    }

    #[test]
    fn add_file_streams_file_into_zip() {
        let root = temp_root("zip-add-file");
        let file = root.join("payload.txt");
        fs::write(&file, b"payload").unwrap();
        let mut cursor = std::io::Cursor::new(Vec::new());

        ZipStore::new(&mut cursor)
            .tap(|zip| zip.add_file("payload.txt", &file).unwrap())
            .finish()
            .unwrap();
        let bytes = cursor.into_inner();

        assert!(
            bytes
                .windows("payload.txt".len())
                .any(|w| w == b"payload.txt")
        );
        assert!(bytes.windows("payload".len()).any(|w| w == b"payload"));

        let _ = fs::remove_dir_all(root);
    }

    trait Tap: Sized {
        fn tap(self, f: impl FnOnce(&mut Self)) -> Self;
    }

    impl<T> Tap for T {
        fn tap(mut self, f: impl FnOnce(&mut Self)) -> Self {
            f(&mut self);
            self
        }
    }

    fn temp_root(prefix: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "{}-{}",
            prefix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
