use super::Result;
use std::io::{Read, Seek};

pub trait Input: Read + Seek {}
impl<T: Read + Seek> Input for T {}
pub type Archive<'a> = zip::ZipArchive<Box<dyn Input + 'a>>;
pub fn bytes(archive: &mut Archive, name: &str, limit: u64) -> Result<Vec<u8>> {
    let file = archive.by_name(name).map_err(|e| e.to_string())?;
    if file.size() > limit {
        return Err(format!("{name}: exceeds size limit"));
    }
    let mut data = vec![];
    file.take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > limit {
        return Err(format!("{name}: exceeds size limit"));
    }
    Ok(data)
}
pub fn text(archive: &mut Archive, name: &str) -> Result<String> {
    String::from_utf8(bytes(archive, name, 4 * 1024 * 1024)?).map_err(|e| e.to_string())
}
