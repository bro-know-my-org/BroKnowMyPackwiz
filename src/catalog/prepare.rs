use super::{
    curseforge::{Client, File, Filter, Transport},
    dependencies::{self, Action, Installed, Source},
};
use crate::{
    config::ProjectConfig,
    layout::PackLayout,
    metadata::{ModMetadata, Side},
    operation::{
        Control, Error, ErrorCode, Result,
        edit::{self, Draft},
        preview::Guard,
    },
    scan::ScanReport,
};
use std::{collections::BTreeMap, path::Path};
use toml_edit::{DocumentMut, Value};

pub struct Row {
    pub name: String,
    pub old: Option<u64>,
    pub file: File,
    pub side: Side,
    pub action: Action,
}
pub struct Preview {
    pub rows: Vec<Row>,
    pub drafts: Vec<Draft>,
    pub guard: Guard,
    pub downloads: Vec<crate::operation::download::Download>,
    pub download: bool,
}
struct Existing {
    path: String,
    document: DocumentMut,
    expected: Option<String>,
    metadata: ModMetadata,
    target: Option<String>,
}

struct CatalogSource<'a, T> {
    client: &'a Client<T>,
    pack_filter: &'a Filter,
}
impl<T: Transport> Source for CatalogSource<'_, T> {
    fn compatible(&self, file: &File, _: &Filter) -> Result<bool> {
        let project = self.client.project(file.project_id)?;
        Ok(file.compatible(&class_filter(self.pack_filter, project.class_id)?))
    }
    fn latest(&self, id: u64, _: &Filter) -> Result<File> {
        let project = self.client.project(id)?;
        self.client
            .latest_compatible(id, &class_filter(self.pack_filter, project.class_id)?)
    }
}
fn class_filter(filter: &Filter, class: u64) -> Result<Filter> {
    let mut filter = filter.clone();
    match class {
        6 => {}
        12 | 6552 => filter.loader = None,
        _ => {
            return Err(Error::key(
                ErrorCode::Invalid,
                "unsupported_curseforge_class",
            ));
        }
    }
    Ok(filter)
}
fn collision(path: &str) -> Error {
    Error::key(ErrorCode::Conflict, "file_collision").context(path)
}

/// No pack writes or latest-version resolution happens after this preview.
pub fn curseforge<T: Transport>(
    root: &Path,
    state: &Path,
    client: &Client<T>,
    selected: File,
    side: Side,
    relaxed: bool,
    control: &Control,
) -> Result<Preview> {
    curseforge_many(
        root,
        state,
        client,
        vec![(selected, side)],
        relaxed,
        control,
    )
}

pub fn curseforge_many<T: Transport>(
    root: &Path,
    state: &Path,
    client: &Client<T>,
    selections: Vec<(File, Side)>,
    relaxed: bool,
    control: &Control,
) -> Result<Preview> {
    let mut guard = Guard::capture(root, control)?;
    let config = ProjectConfig::load_operation(root)?;
    let layout = PackLayout::from_config(&config);
    let filter = if relaxed {
        Filter::default()
    } else {
        Filter::for_pack(root)?
    };
    let selections = selections
        .into_iter()
        .map(|(file, side)| {
            let project = client.project(file.project_id)?;
            class_filter(&filter, project.class_id)?;
            Ok((
                file,
                if project.class_id == 6 {
                    side
                } else {
                    Side::Client
                },
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let report = ScanReport::build_operation(root, &config, &layout)?;
    let mut existing = BTreeMap::new();
    let mut installed = Vec::new();
    let mut targets = BTreeMap::new();
    let mut metadata_paths = BTreeMap::new();
    for item in report.metadata {
        control.check()?;
        let (document, expected) = edit::document(root, &item.path)?;
        let metadata = ModMetadata::load_operation(&root.join(&item.path))?;
        let target = metadata
            .filename
            .as_ref()
            .map(|name| crate::install::resolve_pack_file_path_operation(&item.path, name, &layout))
            .transpose()?;
        if metadata_paths
            .insert(item.path.to_lowercase(), item.path.clone())
            .is_some()
        {
            return Err(collision(&item.path));
        }
        if let Some(target) = &target {
            if targets
                .insert(target.to_lowercase(), item.path.clone())
                .is_some()
            {
                return Err(collision(target));
            }
        }
        if let Some(id) = metadata.curseforge_project_id {
            if existing.contains_key(&id) {
                return Err(collision(&item.path));
            }
            let file_id = metadata.curseforge_file_id.ok_or_else(|| {
                Error::named(
                    ErrorCode::Invalid,
                    "missing_curseforge_file_id",
                    format!("missing_curseforge_file_id: {}", item.path),
                )
                .context(&item.path)
            })?;
            let file = client.file(id, file_id)?;
            installed.push(Installed {
                file,
                side: crate::install::side_from_directory_or_metadata(item.side_hint, &metadata),
                pinned: metadata.pin,
            });
            existing.insert(
                id,
                Existing {
                    path: item.path,
                    document,
                    expected,
                    metadata,
                    target,
                },
            );
        }
    }
    let source = CatalogSource {
        client,
        pack_filter: &filter,
    };
    let entries = dependencies::plan_many(&source, selections, &filter, &installed, control)?;
    let mut rows = Vec::new();
    let mut drafts = Vec::new();
    let mut downloads = Vec::new();
    for mut entry in entries {
        control.check()?;
        let id = entry.file.project_id;
        let project = client.project(id)?;
        if !entry
            .file
            .compatible(&class_filter(&filter, project.class_id)?)
        {
            return Err(Error::named(
                ErrorCode::Conflict,
                "dependency_version_mismatch",
                format!("dependency_version_mismatch: {id}"),
            )
            .context(id.to_string()));
        }
        if project.class_id != 6 && entry.side != Side::Client {
            return Err(Error::named(
                ErrorCode::Conflict,
                "dependency_side_mismatch",
                format!("dependency_side_mismatch: {id}"),
            )
            .context(id.to_string()));
        }
        let old = existing.get(&id);
        if entry.action == Action::Reuse
            && old.is_some_and(|old| {
                old.metadata.filename.as_deref() != Some(entry.file.filename.as_str())
                    || !old
                        .metadata
                        .download_hash
                        .as_ref()
                        .is_some_and(|hash| hash.eq_ignore_ascii_case(&entry.file.sha1))
                    || old.metadata.download_mode.as_deref() != Some("metadata:curseforge")
                    || old.metadata.download_hash_format.as_deref() != Some("sha1")
            })
        {
            entry.action = Action::Update;
        }
        let mut download_target = old.and_then(|old| old.target.clone());
        let name = old
            .and_then(|o| o.metadata.name.clone())
            .unwrap_or(project.name);
        if entry.action != Action::Reuse {
            let (path, mut document, expected) = if let Some(old) = old {
                (old.path.clone(), old.document.clone(), old.expected.clone())
            } else {
                let slug =
                    crate::ops::safe_slug(&name).unwrap_or_else(|_| format!("curseforge-{id}"));
                let directory = match project.class_id {
                    6 => layout.metadata_root.to_string_lossy().replace('\\', "/"),
                    12 => "resourcepacks".into(),
                    6552 => "shaderpacks".into(),
                    _ => unreachable!(),
                };
                let path = format!(
                    "{directory}/{}",
                    crate::ops::metadata_filename(&slug, &layout)
                );
                crate::operation::paths::relative(&path)?;
                if metadata_paths.contains_key(&path.to_lowercase()) || root.join(&path).exists() {
                    return Err(collision(&path));
                }
                (path, DocumentMut::new(), None)
            };
            crate::operation::paths::filename(&entry.file.filename)?;
            let target = crate::install::resolve_pack_file_path_operation(
                &path,
                &entry.file.filename,
                &layout,
            )?;
            if targets
                .get(&target.to_lowercase())
                .is_some_and(|owner| owner != &path)
            {
                return Err(collision(&target));
            }
            if old.and_then(|o| o.target.as_ref()) != Some(&target) && root.join(&target).exists() {
                return Err(collision(&target));
            }
            guard.watch(root, &target)?;
            download_target = Some(target.clone());
            metadata_paths.insert(path.to_lowercase(), path.clone());
            targets.insert(target.to_lowercase(), path.clone());
            for (keys, value) in [
                (vec!["name"], Value::from(name.clone())),
                (vec!["filename"], Value::from(entry.file.filename.clone())),
                (vec!["side"], Value::from(entry.side.as_str())),
                (vec!["download", "mode"], Value::from("metadata:curseforge")),
                (vec!["download", "hash-format"], Value::from("sha1")),
                (
                    vec!["download", "hash"],
                    Value::from(entry.file.sha1.clone()),
                ),
                (vec!["update", "curseforge", "project-id"], integer(id)?),
                (
                    vec!["update", "curseforge", "file-id"],
                    integer(entry.file.id)?,
                ),
            ] {
                edit::set(&mut document, &keys, value)?;
            }
            if let Some(table) = document.get_mut("download").and_then(|v| v.as_table_mut()) {
                table.remove("url");
            }
            drafts.push(edit::draft(state, &path, expected, &document)?);
        }
        if let Some(target) = download_target {
            guard.watch(root, &target)?;
            downloads.push(crate::operation::download::Download {
                expected: crate::operation::durable::fingerprint(&root.join(&target))?,
                relative: target,
                source: crate::operation::download::Source::CurseForge {
                    project: id,
                    file: entry.file.id,
                    filename: entry.file.filename.clone(),
                },
                hash_format: "sha1".into(),
                hash: entry.file.sha1.clone(),
                preserve: old.is_some_and(|old| old.metadata.preserve),
            });
        } else {
            return Err(Error::key(ErrorCode::Invalid, "missing_download_target"));
        }
        rows.push(Row {
            name,
            old: old.and_then(|o| o.metadata.curseforge_file_id),
            file: entry.file,
            side: entry.side,
            action: entry.action,
        });
    }
    guard.validate(root, control)?;
    Ok(Preview {
        rows,
        drafts,
        guard,
        downloads,
        download: false,
    })
}
fn integer(value: u64) -> Result<Value> {
    i64::try_from(value)
        .map(Value::from)
        .map_err(|_| Error::key(ErrorCode::Invalid, "identifier_overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::durable;
    use serde_json::json;
    use std::fs;
    #[test]
    fn client_mod_can_require_a_resource_pack_without_a_loader_tag() {
        let client = Client {
            transport: |path: &str| {
                if path == "mods/1" || path == "mods/2" {
                    return Ok(
                        json!({"data":{"id":if path == "mods/1" {1} else {2},"classId":if path == "mods/1" {6} else {12}}}),
                    );
                }
                assert!(path.starts_with("mods/2/files?"));
                assert!(!path.contains("modLoaderType"));
                Ok(
                    json!({"data":[{"id":200,"modId":2,"fileName":"pack.zip","gameVersions":["1.21.1"],"hashes":[{"algo":1,"value":"a".repeat(40)}]}]}),
                )
            },
        };
        let filter = Filter {
            minecraft: Some("1.21.1".into()),
            loader: Some("fabric".into()),
        };
        let source = CatalogSource {
            client: &client,
            pack_filter: &filter,
        };
        let selected = File {
            project_id: 1,
            id: 100,
            name: String::new(),
            filename: "mod.jar".into(),
            versions: vec!["1.21.1".into(), "Fabric".into()],
            date: String::new(),
            release_type: 1,
            size: 1,
            sha1: "a".repeat(40),
            dependencies: vec![super::super::curseforge::Dependency {
                project_id: 2,
                relation: 3,
            }],
        };
        let entries = dependencies::plan(
            &source,
            selected,
            Side::Client,
            &filter,
            &[],
            &Control::default(),
        )
        .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].file.filename, "pack.zip");
    }
    #[test]
    fn frozen_dependency_preview_applies_offline_as_one_transaction() {
        let base = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(base.join("pack/mods")).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let root = base.join("pack");
        fs::write(
            root.join("pack.toml"),
            "name = \"Test\"\n[versions]\nminecraft = \"1.21.1\"\nfabric = \"0.16.0\"\n",
        )
        .unwrap();
        let client = Client {
            transport: |path: &str| {
                if path == "mods/1" || path == "mods/2" {
                    return Ok(
                        json!({"data":{"id":if path == "mods/1" {1} else {2},"name":if path == "mods/1" {"Root"} else {"Dependency"},"classId":6}}),
                    );
                }
                if path.starts_with("mods/2/files?") {
                    return Ok(
                        json!({"data":[{"id":200,"modId":2,"fileName":"dep.jar","gameVersions":["1.21.1","Fabric"],"hashes":[{"algo":1,"value":"b".repeat(40)}]}]}),
                    );
                }
                panic!("unexpected request {path}");
            },
        };
        let selected = File {
            project_id: 1,
            id: 100,
            name: "Root v1".into(),
            filename: "root.jar".into(),
            versions: vec!["1.21.1".into(), "Fabric".into()],
            date: String::new(),
            release_type: 1,
            size: 10,
            sha1: "a".repeat(40),
            dependencies: vec![super::super::curseforge::Dependency {
                project_id: 2,
                relation: 3,
            }],
        };
        let control = Control::default();
        let preview = curseforge(
            &root,
            &base.join("state"),
            &client,
            selected.clone(),
            Side::Both,
            false,
            &control,
        )
        .unwrap();
        assert_eq!(preview.rows.len(), 2);
        assert!(!root.join("mods/root.pw.toml").exists());
        edit::execute(
            &root,
            &base.join("state"),
            &base.join("task"),
            &preview.drafts,
            Some(&preview.guard),
            &control,
        )
        .unwrap();
        for name in ["root", "dependency"] {
            assert!(root.join(format!("mods/{name}.pw.toml")).exists());
        }
        assert!(!root.join("mods/root.jar").exists());
        assert!(root.join("index.toml").exists());
        fs::remove_dir_all(base).unwrap();
    }
}
