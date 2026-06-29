use std::env;

use serde_json::Value;

use crate::config::ProjectConfig;
use crate::http::http_get_to_string_with_header;

#[derive(Debug, Clone)]
pub struct CurseForgeFileInfo {
    pub project_id: u64,
    pub file_id: u64,
    pub name: Option<String>,
    pub filename: String,
    pub hash_format: String,
    pub hash: String,
}

pub fn api_key(config: &ProjectConfig) -> Option<String> {
    config
        .curseforge
        .api_key
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| env::var("CURSEFORGE_API_KEY").ok())
        .filter(|value| !value.is_empty())
}

pub fn resolve_download_url(
    api_key: Option<&str>,
    project_id: u64,
    file_id: u64,
) -> Result<String, String> {
    let api_key = api_key.ok_or_else(|| {
        "CurseForge metadata needs [curseforge] api-key or CURSEFORGE_API_KEY".to_string()
    })?;
    let url = format!("https://api.curseforge.com/v1/mods/{project_id}/files/{file_id}");
    let json = http_get_to_string_with_header(&url, "x-api-key", &api_key)?;
    let value = parse_json(&json, "CurseForge file response")?;
    download_url_from_value(&value).ok_or_else(|| {
        format!("CurseForge response did not include downloadUrl for {project_id}/{file_id}")
    })
}

pub fn resolve_project_and_file(
    api_key: Option<&str>,
    project: &str,
    file_id: Option<u64>,
    minecraft_version: Option<&str>,
    loader: Option<&str>,
) -> Result<CurseForgeFileInfo, String> {
    let api_key = api_key.ok_or_else(|| {
        "add-curseforge by slug/url needs [curseforge] api-key or CURSEFORGE_API_KEY".to_string()
    })?;
    let (project_id, name) = if let Some(id) = project.parse::<u64>().ok() {
        (id, None)
    } else {
        search_project(api_key, project)?
    };
    let file_id = match file_id {
        Some(value) => value,
        None => latest_file_id(api_key, project_id, minecraft_version, loader)?,
    };
    let mut file = get_file_info(api_key, project_id, file_id)?;
    file.name = file.name.or(name);
    Ok(file)
}

pub fn get_file_info(
    api_key: &str,
    project_id: u64,
    file_id: u64,
) -> Result<CurseForgeFileInfo, String> {
    let url = format!("https://api.curseforge.com/v1/mods/{project_id}/files/{file_id}");
    let json = http_get_to_string_with_header(&url, "x-api-key", api_key)?;
    let value = parse_json(&json, "CurseForge file response")?;
    let data = data_value(&value);
    let filename = json_string(data, "fileName")
        .or_else(|| json_string(data, "displayName"))
        .ok_or_else(|| {
            format!("CurseForge file {project_id}/{file_id} did not include fileName")
        })?;
    let (hash_format, hash) = json_sha1_hash(data).ok_or_else(|| {
        format!("CurseForge file {project_id}/{file_id} did not include sha1 hash")
    })?;
    Ok(CurseForgeFileInfo {
        project_id,
        file_id,
        name: None,
        filename,
        hash_format,
        hash,
    })
}

fn search_project(api_key: &str, project: &str) -> Result<(u64, Option<String>), String> {
    let slug = project_slug(project);
    let url = format!(
        "https://api.curseforge.com/v1/mods/search?gameId=432&slug={}&pageSize=1",
        percent_encode_path_segment(&slug)
    );
    let json = http_get_to_string_with_header(&url, "x-api-key", api_key)?;
    let value = parse_json(&json, "CurseForge search response")?;
    let data = first_data_object_value(&value)
        .ok_or_else(|| format!("CurseForge project not found for slug/url: {project}"))?;
    let id = json_u64_any(Some(data), &["id"])
        .ok_or_else(|| format!("CurseForge project response did not include id: {project}"))?;
    let name = json_string(Some(data), "name");
    Ok((id, name))
}

pub fn latest_file_id(
    api_key: &str,
    project_id: u64,
    minecraft_version: Option<&str>,
    loader: Option<&str>,
) -> Result<u64, String> {
    let mut url = format!("https://api.curseforge.com/v1/mods/{project_id}/files?pageSize=1");
    if let Some(version) = minecraft_version.filter(|value| !value.is_empty()) {
        url.push_str("&gameVersion=");
        url.push_str(&percent_encode_path_segment(version));
    }
    if let Some(loader) = loader.and_then(loader_type) {
        url.push_str("&modLoaderType=");
        url.push_str(&loader.to_string());
    }
    let json = http_get_to_string_with_header(&url, "x-api-key", api_key)?;
    latest_file_id_from_json(project_id, &json)
}

fn latest_file_id_from_json(project_id: u64, json: &str) -> Result<u64, String> {
    let value = parse_json(json, "CurseForge files response")?;
    let data = first_data_object_value(&value)
        .ok_or_else(|| format!("CurseForge project {project_id} did not return files"))?;
    json_u64_any(Some(data), &["id", "fileId"]).ok_or_else(|| {
        format!("CurseForge project {project_id} file response did not include id/fileId")
    })
}

pub fn cdn_download_url(file_id: u64, filename: &str) -> Option<String> {
    let filename = filename.trim();
    if file_id == 0
        || filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
    {
        return None;
    }
    let bucket = file_id / 1000;
    let tail = file_id % 1000;
    Some(format!(
        "https://edge.forgecdn.net/files/{bucket}/{tail:03}/{}",
        percent_encode_path_segment(filename)
    ))
}

pub fn percent_encode_path_segment(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

fn project_slug(project: &str) -> String {
    let trimmed = project.trim().trim_end_matches('/');
    let slug = trimmed.rsplit('/').next().unwrap_or(trimmed).trim();
    slug.split(['?', '#'])
        .next()
        .unwrap_or(slug)
        .trim()
        .to_string()
}

fn loader_type(loader: &str) -> Option<u64> {
    match loader.to_ascii_lowercase().as_str() {
        "forge" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" | "neo-forge" => Some(6),
        _ => None,
    }
}

fn parse_json(json: &str, context: &str) -> Result<Value, String> {
    serde_json::from_str(json).map_err(|err| format!("failed to parse {context}: {err}"))
}

fn data_value(value: &Value) -> Option<&Value> {
    value.get("data")
}

fn first_data_object_value(value: &Value) -> Option<&Value> {
    data_value(value)?
        .as_array()?
        .iter()
        .find(|entry| entry.is_object())
}

fn json_string(value: Option<&Value>, field: &str) -> Option<String> {
    value?.get(field)?.as_str().map(str::to_string)
}

fn download_url_from_value(value: &Value) -> Option<String> {
    json_string(data_value(value), "downloadUrl").filter(|value| !value.trim().is_empty())
}

fn json_u64_any(value: Option<&Value>, fields: &[&str]) -> Option<u64> {
    let value = value?;
    fields.iter().find_map(|field| {
        let field_value = value.get(*field)?;
        field_value
            .as_u64()
            .or_else(|| field_value.as_str()?.parse::<u64>().ok())
    })
}

fn json_sha1_hash(value: Option<&Value>) -> Option<(String, String)> {
    let hashes = value?.get("hashes")?.as_array()?;
    hashes.iter().find_map(|hash| {
        let algo = json_u64_any(Some(hash), &["algo"])?;
        if algo != 1 {
            return None;
        }
        let value = json_string(Some(hash), "value")?;
        Some(("sha1".to_string(), value.to_ascii_lowercase()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_download_url() {
        let json = r#"{"data":{"downloadUrl":"https:\/\/edge.forgecdn.net\/x.jar"}}"#;
        let value = parse_json(json, "test response").unwrap();

        assert_eq!(
            download_url_from_value(&value).as_deref(),
            Some("https://edge.forgecdn.net/x.jar")
        );
    }

    #[test]
    fn null_download_url_is_none() {
        let json = r#"{"data":{"downloadUrl":null}}"#;
        let value = parse_json(json, "test response").unwrap();

        assert_eq!(download_url_from_value(&value), None);
    }

    #[test]
    fn empty_download_url_is_rejected() {
        let json = r#"{"data":{"downloadUrl":""}}"#;
        let value = parse_json(json, "test response").unwrap();

        assert_eq!(download_url_from_value(&value), None);
    }

    #[test]
    fn latest_file_id_accepts_id_field() {
        let json =
            r#"{"data":[{"id":6822250,"fileName":"emi.jar"}],"pagination":{"resultCount":1}}"#;

        assert_eq!(latest_file_id_from_json(633287, json).unwrap(), 6822250);
    }

    #[test]
    fn latest_file_id_accepts_file_id_field() {
        let json = r#"{"data":[{"fileId":"6822250","fileName":"emi.jar"}],"pagination":{"resultCount":1}}"#;

        assert_eq!(latest_file_id_from_json(633287, json).unwrap(), 6822250);
    }

    #[test]
    fn builds_forgecdn_url_from_file_id() {
        assert_eq!(
            cdn_download_url(6498183, "AlwaysEat-neoforge-1.0.0.jar").as_deref(),
            Some("https://edge.forgecdn.net/files/6498/183/AlwaysEat-neoforge-1.0.0.jar")
        );
    }

    #[test]
    fn encodes_forgecdn_filename() {
        assert_eq!(
            cdn_download_url(1234005, "a file+name.jar").as_deref(),
            Some("https://edge.forgecdn.net/files/1234/005/a%20file%2Bname.jar")
        );
    }

    #[test]
    fn rejects_unsafe_forgecdn_filename() {
        assert_eq!(cdn_download_url(1234005, "../evil.jar"), None);
        assert_eq!(cdn_download_url(1234005, "nested/evil.jar"), None);
        assert_eq!(cdn_download_url(1234005, r"nested\evil.jar"), None);
    }

    #[test]
    fn project_slug_strips_url_query_and_fragment() {
        assert_eq!(
            project_slug("https://www.curseforge.com/minecraft/mc-mods/example?foo=1#files"),
            "example"
        );
    }
}
