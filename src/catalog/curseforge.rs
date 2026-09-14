use crate::{
    curseforge::percent_encode_path_segment as encode,
    operation::{Error, ErrorCode, Result},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

const PAGE_SIZE: usize = 30;

pub trait Transport {
    fn get(&self, path: &str) -> Result<Value>;
}
impl<F: Fn(&str) -> Result<Value>> Transport for F {
    fn get(&self, path: &str) -> Result<Value> {
        self(path)
    }
}
pub struct Http {
    key: String,
}
impl Transport for Http {
    fn get(&self, path: &str) -> Result<Value> {
        let text = crate::http::http_get_to_string_with_header(
            &format!("https://api.curseforge.com/v1/{path}"),
            "x-api-key",
            &self.key,
        )
        .map_err(Error::from)?;
        serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))
    }
}
pub struct Client<T = Http> {
    pub transport: T,
}
impl Client<Http> {
    pub fn for_pack(root: &Path) -> Result<Self> {
        let config = crate::config::ProjectConfig::load(root).map_err(Error::from)?;
        let key = crate::curseforge::api_key(&config)
            .ok_or_else(|| invalid("curseforge_api_key_required"))?;
        Ok(Self {
            transport: Http { key },
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Filter {
    pub minecraft: Option<String>,
    pub loader: Option<String>,
}
impl Filter {
    pub fn for_pack(root: &Path) -> Result<Self> {
        let pack = crate::packinfo::PackInfo::load(root).map_err(Error::from)?;
        Ok(Self {
            loader: pack.loader_name().map(str::to_string),
            minecraft: pack.minecraft,
        })
    }
    fn query(&self) -> Result<String> {
        let mut query = String::new();
        if let Some(mc) = self.minecraft.as_ref().filter(|v| !v.is_empty()) {
            query.push_str(&format!("&gameVersion={}", encode(mc)));
        }
        if let Some(loader) = self.loader.as_ref().filter(|v| !v.is_empty()) {
            let id = crate::curseforge::loader_type(loader)
                .ok_or_else(|| invalid("unsupported loader"))?;
            query.push_str(&format!("&modLoaderType={id}"));
        }
        Ok(query)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: u64,
    pub name: String,
    pub summary: String,
    pub website: String,
    pub class_id: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dependency {
    pub project_id: u64,
    pub relation: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct File {
    pub project_id: u64,
    pub id: u64,
    pub name: String,
    pub filename: String,
    pub versions: Vec<String>,
    pub date: String,
    pub release_type: u64,
    pub size: u64,
    pub sha1: String,
    pub dependencies: Vec<Dependency>,
}
impl File {
    pub fn compatible(&self, filter: &Filter) -> bool {
        filter
            .minecraft
            .as_ref()
            .is_none_or(|mc| mc.is_empty() || self.versions.iter().any(|v| v == mc))
            && filter.loader.as_ref().is_none_or(|loader| {
                loader.is_empty()
                    || self.versions.iter().any(|v| {
                        v.replace('-', "")
                            .eq_ignore_ascii_case(&loader.replace('-', ""))
                    })
            })
    }
}
#[derive(Clone, Debug)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub offset: usize,
}
impl<T> Page<T> {
    pub fn has_more(&self) -> bool {
        self.offset + self.items.len() < self.total
    }
}

impl<T: Transport> Client<T> {
    pub fn search(
        &self,
        query: &str,
        filter: &Filter,
        class_id: u64,
        page: usize,
    ) -> Result<Page<Project>> {
        let offset = page
            .checked_mul(PAGE_SIZE)
            .ok_or_else(|| invalid("page overflow"))?;
        let path = format!(
            "mods/search?gameId=432&classId={class_id}&searchFilter={}&index={offset}&pageSize={PAGE_SIZE}&sortField=2&sortOrder=desc{}",
            encode(query),
            filter.query()?
        );
        let value = self.transport.get(&path)?;
        let items = array(&value)?
            .iter()
            .map(project)
            .collect::<Result<Vec<_>>>()?;
        Ok(page_result(&value, items, offset))
    }
    pub fn project(&self, id: u64) -> Result<Project> {
        let value = self.transport.get(&format!("mods/{id}"))?;
        let result = project(
            value
                .get("data")
                .ok_or_else(|| invalid("missing project data"))?,
        )?;
        if result.id != id {
            return Err(invalid("project identity mismatch"));
        }
        Ok(result)
    }
    pub fn files(&self, project_id: u64, filter: &Filter, page: usize) -> Result<Page<File>> {
        let offset = page
            .checked_mul(PAGE_SIZE)
            .ok_or_else(|| invalid("page overflow"))?;
        let value = self.transport.get(&format!(
            "mods/{project_id}/files?index={offset}&pageSize={PAGE_SIZE}{}",
            filter.query()?
        ))?;
        let items = array(&value)?
            .iter()
            .map(|v| file(v, project_id))
            .collect::<Result<Vec<_>>>()?;
        Ok(page_result(&value, items, offset))
    }
    pub fn file(&self, project_id: u64, file_id: u64) -> Result<File> {
        let value = self
            .transport
            .get(&format!("mods/{project_id}/files/{file_id}"))?;
        let result = file(
            value
                .get("data")
                .ok_or_else(|| invalid("missing file data"))?,
            project_id,
        )?;
        if result.id != file_id {
            return Err(invalid("file identity mismatch"));
        }
        Ok(result)
    }
    pub fn latest_compatible(&self, project_id: u64, filter: &Filter) -> Result<File> {
        let mut page = 0;
        loop {
            let response = self.files(project_id, filter, page)?;
            if let Some(file) = response.items.iter().find(|f| f.compatible(filter)) {
                return Ok(file.clone());
            }
            if !response.has_more() || response.items.is_empty() {
                return Err(Error::new(
                    ErrorCode::Conflict,
                    format!("no compatible file: {project_id}"),
                ));
            }
            page += 1;
        }
    }
}

fn array(value: &Value) -> Result<&Vec<Value>> {
    value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("missing data array"))
}
fn number(value: &Value, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .ok_or_else(|| invalid(format!("missing {key}")))
}
fn string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}
fn project(value: &Value) -> Result<Project> {
    Ok(Project {
        id: number(value, "id")?,
        name: string(value, "name"),
        summary: string(value, "summary"),
        website: value
            .pointer("/links/websiteUrl")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        class_id: value.get("classId").and_then(Value::as_u64).unwrap_or(6),
    })
}
fn file(value: &Value, project_id: u64) -> Result<File> {
    if value
        .get("modId")
        .and_then(Value::as_u64)
        .is_some_and(|v| v != project_id)
    {
        return Err(invalid("file project mismatch"));
    }
    let filename = string(value, "fileName");
    crate::pathutil::safe_filename(&filename).map_err(Error::from)?;
    let sha1 = value
        .get("hashes")
        .and_then(Value::as_array)
        .and_then(|hashes| {
            hashes
                .iter()
                .find(|hash| hash.get("algo").and_then(Value::as_u64) == Some(1))
        })
        .map(|hash| string(hash, "value").to_ascii_lowercase())
        .ok_or_else(|| invalid("missing SHA-1"))?;
    if sha1.len() != 40 || !sha1.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(invalid("invalid SHA-1"));
    }
    let dependencies = value
        .get("dependencies")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|v| {
                    Ok(Dependency {
                        project_id: number(v, "modId")?,
                        relation: number(v, "relationType")?,
                    })
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(File {
        project_id,
        id: number(value, "id")?,
        name: string(value, "displayName"),
        filename,
        versions: value
            .get("gameVersions")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        date: string(value, "fileDate"),
        release_type: value
            .get("releaseType")
            .and_then(Value::as_u64)
            .unwrap_or(1),
        size: value.get("fileLength").and_then(Value::as_u64).unwrap_or(0),
        sha1,
        dependencies,
    })
}
fn page_result<T>(value: &Value, items: Vec<T>, offset: usize) -> Page<T> {
    let total = value
        .pointer("/pagination/totalCount")
        .and_then(Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(offset + items.len());
    Page {
        items,
        total,
        offset,
    }
}
fn invalid(detail: impl Into<String>) -> Error {
    Error::new(ErrorCode::Invalid, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn search_encodes_queries_and_preserves_pagination() {
        let requested = std::cell::RefCell::new(String::new());
        let client = Client {
            transport: |path: &str| {
                *requested.borrow_mut() = path.into();
                Ok(json!({"data":[{"id":1,"name":"Test"}],"pagination":{"totalCount":99}}))
            },
        };
        let page = client
            .search(
                "中文 &tag",
                &Filter {
                    minecraft: Some("1.21.1".into()),
                    loader: Some("neoforge".into()),
                },
                6,
                1,
            )
            .unwrap();
        assert!(
            requested
                .borrow()
                .contains("searchFilter=%E4%B8%AD%E6%96%87%20%26tag")
        );
        assert!(requested.borrow().contains("index=30"));
        assert!(requested.borrow().contains("modLoaderType=6"));
        assert_eq!(page.items[0].id, 1);
        assert!(page.has_more());
    }
    #[test]
    fn files_validate_identity_hash_and_dependencies() {
        let value = json!({"id":2,"modId":1,"fileName":"mod.jar","gameVersions":["1.21.1","NeoForge"],"hashes":[{"algo":1,"value":"a".repeat(40)}],"dependencies":[{"modId":3,"relationType":3}]});
        let parsed = file(&value, 1).unwrap();
        assert!(parsed.compatible(&Filter {
            minecraft: Some("1.21.1".into()),
            loader: Some("neoforge".into())
        }));
        assert!(!parsed.compatible(&Filter {
            minecraft: Some("1.20.1".into()),
            loader: None
        }));
        assert_eq!(parsed.dependencies[0].project_id, 3);
        assert!(file(&value, 99).is_err());
    }
}
