use super::curseforge::Transport;
use crate::operation::{Error, ErrorCode, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const PAGE_SIZE: usize = 30;
pub struct Http;
impl Transport for Http {
    fn get(&self, path: &str) -> Result<Value> {
        let url = format!("https://api.github.com/{path}");
        let response = match std::env::var("GITHUB_TOKEN").ok().filter(|v| !v.is_empty()) {
            Some(token) => crate::http::http_get_to_string_with_header(
                &url,
                "Authorization",
                &format!("Bearer {token}"),
            ),
            None => crate::http::http_get_to_string(&url),
        }
        .map_err(Error::from)?;
        serde_json::from_str(&response).map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))
    }
}
pub struct Client<T = Http> {
    pub transport: T,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    pub id: u64,
    pub name: String,
    pub tag: String,
    pub prerelease: bool,
    pub published: String,
    pub body: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub id: u64,
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: Option<String>,
}
pub struct Page<T> {
    pub items: Vec<T>,
    pub has_more: bool,
}
impl<T: Transport> Client<T> {
    pub fn releases(&self, repository: &str, page: usize) -> Result<Page<Release>> {
        let repo = crate::github::normalize_project(repository).map_err(Error::from)?;
        let path = format!(
            "repos/{repo}/releases?per_page={PAGE_SIZE}&page={}",
            api_page(page)?
        );
        let value = self.transport.get(&path)?;
        let items = array(&value)?;
        let has_more = items.len() == PAGE_SIZE;
        let items = items
            .iter()
            .filter(|item| item.get("draft").and_then(Value::as_bool) != Some(true))
            .map(release)
            .collect::<Result<Vec<_>>>()?;
        Ok(Page { items, has_more })
    }
    pub fn assets(&self, repository: &str, release_id: u64, page: usize) -> Result<Page<Asset>> {
        if release_id == 0 {
            return Err(invalid("missing_release_id"));
        }
        let repo = crate::github::normalize_project(repository).map_err(Error::from)?;
        let path = format!(
            "repos/{repo}/releases/{release_id}/assets?per_page={PAGE_SIZE}&page={}",
            api_page(page)?
        );
        let value = self.transport.get(&path)?;
        let items = array(&value)?;
        let has_more = items.len() == PAGE_SIZE;
        Ok(Page {
            items: items.iter().map(asset).collect::<Result<_>>()?,
            has_more,
        })
    }
    pub fn asset(&self, repository: &str, id: u64) -> Result<Asset> {
        if id == 0 {
            return Err(invalid("missing_asset_id"));
        }
        let repo = crate::github::normalize_project(repository).map_err(Error::from)?;
        let result = asset(
            &self
                .transport
                .get(&format!("repos/{repo}/releases/assets/{id}"))?,
        )?;
        if result.id != id {
            return Err(invalid("asset_identity_mismatch"));
        }
        Ok(result)
    }
}
fn api_page(page: usize) -> Result<usize> {
    page.checked_add(1).ok_or_else(|| invalid("page_overflow"))
}
fn array(value: &Value) -> Result<&Vec<Value>> {
    value.as_array().ok_or_else(|| invalid("missing_array"))
}
fn string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .into()
}
fn id(value: &Value) -> Result<u64> {
    value
        .get("id")
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)
        .ok_or_else(|| invalid("missing_id"))
}
fn release(value: &Value) -> Result<Release> {
    let tag = string(value, "tag_name");
    if tag.is_empty() {
        return Err(invalid("missing_release_tag"));
    }
    Ok(Release {
        id: id(value)?,
        name: string(value, "name"),
        tag,
        prerelease: value
            .get("prerelease")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        published: string(value, "published_at"),
        body: string(value, "body"),
    })
}
fn asset(value: &Value) -> Result<Asset> {
    let name = string(value, "name");
    crate::operation::paths::filename(&name)?;
    let url = string(value, "browser_download_url");
    let parsed = url
        .parse::<ureq::http::Uri>()
        .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?;
    if parsed.scheme_str() != Some("https") || parsed.authority().is_none() {
        return Err(invalid("invalid_asset_url"));
    }
    let digest = string(value, "digest");
    let sha256 = digest
        .strip_prefix("sha256:")
        .map(|s| s.to_ascii_lowercase());
    if sha256
        .as_ref()
        .is_some_and(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(invalid("invalid_asset_digest"));
    }
    Ok(Asset {
        id: id(value)?,
        name,
        url,
        size: value
            .get("size")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid("missing_asset_size"))?,
        sha256,
    })
}
fn invalid(key: &str) -> Error {
    Error::key(ErrorCode::Invalid, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn paginates_releases_and_omits_drafts_without_losing_next_page() {
        let client = Client {
            transport: |path: &str| {
                assert_eq!(path, "repos/owner/repo/releases?per_page=30&page=2");
                Ok(Value::Array((1..=30).map(|id| json!({"id":id,"tag_name":format!("v{id}"),"draft": id == 1,"prerelease":id == 2})).collect()))
            },
        };
        let page = client
            .releases("https://github.com/owner/repo/releases", 1)
            .unwrap();
        assert!(page.has_more);
        assert_eq!(page.items.len(), 29);
        assert!(page.items[0].prerelease);
    }
    #[test]
    fn assets_validate_ids_filenames_urls_and_optional_digests() {
        let mut value = json!({"id":7,"name":"中文.jar","browser_download_url":"https://github.com/owner/repo/releases/download/v1/mod.jar","size":10,"digest":format!("sha256:{}","A".repeat(64))});
        assert_eq!(asset(&value).unwrap().sha256, Some("a".repeat(64)));
        value["digest"] = Value::Null;
        assert!(asset(&value).unwrap().sha256.is_none());
        value["name"] = "../escape.jar".into();
        assert!(asset(&value).is_err());
        value["name"] = "mod.jar".into();
        value["browser_download_url"] = "file:///tmp/file".into();
        assert!(asset(&value).is_err());
        let client = Client {
            transport: |_: &str| {
                Ok(
                    json!({"id":8,"name":"mod.jar","browser_download_url":"https://github.com/mod.jar","size":1}),
                )
            },
        };
        assert!(client.asset("owner/repo", 7).is_err());
    }
    #[test]
    fn release_assets_use_the_separate_paginated_endpoint() {
        let client = Client {
            transport: |path: &str| {
                assert_eq!(
                    path,
                    "repos/owner/repo/releases/123/assets?per_page=30&page=1"
                );
                Ok(json!([]))
            },
        };
        let page = client.assets("owner/repo", 123, 0).unwrap();
        assert!(page.items.is_empty() && !page.has_more);
    }
}
