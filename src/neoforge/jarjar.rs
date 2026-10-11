use super::{
    Result,
    compatibility::{Report, matches},
    maven::compare,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Cursor, Read, Seek},
    path::PathBuf,
};

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
#[derive(Deserialize)]
struct Metadata {
    jars: Vec<Dependency>,
}
#[derive(Deserialize)]
struct Dependency {
    identifier: Identifier,
    version: Version,
    path: String,
}
#[derive(Deserialize)]
struct Identifier {
    group: String,
    artifact: String,
}
#[derive(Deserialize)]
struct Version {
    range: String,
    #[serde(rename = "artifactVersion")]
    artifact: String,
}
type Coordinate = (String, String);
pub struct Jar {
    pub path: PathBuf,
    pub nested: bool,
    data: Option<Vec<u8>>,
}
impl Jar {
    pub fn open(&self) -> Result<Archive<'_>> {
        let input: Box<dyn Input + '_> = match &self.data {
            Some(data) => Box::new(Cursor::new(data.as_slice())),
            None => Box::new(fs::File::open(&self.path).map_err(|e| e.to_string())?),
        };
        zip::ZipArchive::new(input).map_err(|e| e.to_string())
    }
}
struct Edge {
    id: Coordinate,
    version: Version,
    node: usize,
}
struct Node {
    jar: Jar,
    depth: usize,
    children: Option<Vec<Edge>>,
    errors: Vec<String>,
    unresolved: BTreeSet<Coordinate>,
    unknown_coordinates: bool,
    readable: bool,
}
fn expand(index: usize, nodes: &mut Vec<Node>, budget: &mut u64) {
    if nodes[index].children.is_some() {
        return;
    }
    let node = &mut nodes[index];
    let mut children = vec![];
    let result = (|| -> Result<()> {
        let mut archive = node.jar.open().map_err(|e| {
            node.readable = false;
            format!("unreadable JAR: {e}")
        })?;
        if !archive
            .file_names()
            .any(|n| n == "META-INF/jarjar/metadata.json")
        {
            return Ok(());
        }
        let metadata: Metadata =
            serde_json::from_str(&text(&mut archive, "META-INF/jarjar/metadata.json")?)
                .map_err(|e| e.to_string())?;
        for dep in metadata.jars {
            let result = (|| -> Result<Vec<u8>> {
                if node.depth == 8 {
                    return Err("nested JAR depth exceeds limit".into());
                }
                crate::pathutil::safe_slash_path(&dep.path)?;
                compare(&dep.version.artifact, &dep.version.artifact)?;
                matches(&dep.version.range, &dep.version.artifact)?;
                let data = bytes(&mut archive, &dep.path, (*budget).min(32 * 1024 * 1024))?;
                *budget -= data.len() as u64;
                Ok(data)
            })();
            match result {
                Ok(data) => children.push((dep, data)),
                Err(e) => {
                    node.errors.push(e);
                    node.unresolved
                        .insert((dep.identifier.group, dep.identifier.artifact));
                }
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        node.errors.push(e);
        node.unknown_coordinates = true;
    }
    let path = node.jar.path.clone();
    let depth = node.depth + 1;
    let mut edges = vec![];
    for (dep, data) in children {
        let jar = Jar {
            path: PathBuf::from(format!("{}!/{}", path.display(), dep.path)),
            nested: true,
            data: Some(data),
        };
        if let Err(e) = jar.open() {
            nodes[index].errors.push(format!("{}: {e}", dep.path));
            nodes[index]
                .unresolved
                .insert((dep.identifier.group, dep.identifier.artifact));
            continue;
        }
        let child = nodes.len();
        nodes.push(Node {
            jar,
            depth,
            children: None,
            errors: vec![],
            unresolved: BTreeSet::new(),
            unknown_coordinates: false,
            readable: true,
        });
        edges.push(Edge {
            id: (dep.identifier.group, dep.identifier.artifact),
            version: dep.version,
            node: child,
        });
    }
    nodes[index].children = Some(edges);
}

// Resolve the reachable graph to a fixed point. A changed selection can add or
// remove parent constraints, so do not publish provisional nested inventories.
pub fn inventory(paths: Vec<PathBuf>, report: &mut Report) -> (Vec<Jar>, bool) {
    let mut nodes = vec![];
    let mut incomplete = false;
    for path in paths {
        report.checked += 1;
        nodes.push(Node {
            jar: Jar {
                path,
                nested: false,
                data: None,
            },
            depth: 0,
            children: None,
            errors: vec![],
            unresolved: BTreeSet::new(),
            unknown_coordinates: false,
            readable: true,
        });
    }
    let roots = nodes.len();
    let mut selected = BTreeMap::<Coordinate, usize>::new();
    let mut budget = 128 * 1024 * 1024;
    let mut active = BTreeSet::new();
    let mut stable = false;
    for _ in 0..32 {
        active.clear();
        let mut pending = (0..roots).collect::<Vec<_>>();
        while let Some(index) = pending.pop() {
            if !active.insert(index) {
                continue;
            }
            expand(index, &mut nodes, &mut budget);
            for edge in nodes[index].children.as_ref().unwrap() {
                if let Some(child) = selected.get(&edge.id) {
                    pending.push(*child);
                }
            }
        }
        let mut candidates = BTreeMap::<Coordinate, Vec<&Edge>>::new();
        for &index in &active {
            for edge in nodes[index].children.as_ref().unwrap() {
                candidates.entry(edge.id.clone()).or_default().push(edge);
            }
        }
        let mut next = BTreeMap::new();
        let mut unresolved = vec![];
        for (id, group) in candidates {
            let uncertain = active
                .iter()
                .any(|i| nodes[*i].unknown_coordinates || nodes[*i].unresolved.contains(&id));
            let chosen = group
                .iter()
                .filter(|c| {
                    !uncertain
                        && group
                            .iter()
                            .all(|r| matches(&r.version.range, &c.version.artifact) == Ok(true))
                })
                .max_by(|a, b| compare(&a.version.artifact, &b.version.artifact).unwrap());
            if let Some(chosen) = chosen {
                next.insert(id, chosen.node);
            } else {
                unresolved.push((nodes[group[0].node].jar.path.clone(), id));
            }
        }
        if next == selected {
            for (path, id) in unresolved {
                report.add(
                    &path,
                    format!("nested JAR: unresolved selection for {}:{}", id.0, id.1),
                    false,
                );
                incomplete = true;
            }
            stable = true;
            break;
        }
        selected = next;
    }
    if !stable {
        for node in &nodes[..roots] {
            report.add(
                &node.jar.path,
                "nested JAR: version selection did not converge",
                false,
            );
        }
        incomplete = true;
        active = (0..roots).collect();
    }
    let output = nodes
        .into_iter()
        .enumerate()
        .filter_map(|(index, node)| {
            if !active.contains(&index) {
                return None;
            }
            for error in node.errors {
                report.add(&node.jar.path, format!("nested JAR: {error}"), false);
                incomplete = true;
            }
            node.readable.then_some(node.jar)
        })
        .collect();
    (output, incomplete)
}
