use super::{Instance, Kind, Result, Target};
use serde::Serialize;
use std::{cmp::Ordering, collections::BTreeMap, fs, path::Path};
use toml_edit::DocumentMut;

#[derive(Debug, Serialize)]
pub struct Finding {
    pub file: String,
    pub reason: String,
}
#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub checked: usize,
    pub blocked: Vec<Finding>,
    pub unknown: Vec<Finding>,
}
impl Report {
    fn add(&mut self, file: &Path, reason: impl Into<String>, blocked: bool) {
        let finding = Finding {
            file: file.display().to_string(),
            reason: reason.into(),
        };
        if blocked {
            self.blocked.push(finding);
        } else {
            self.unknown.push(finding);
        }
    }
    pub fn authorize(&self, accept_unknown: bool) -> Result<()> {
        if !self.blocked.is_empty() {
            return Err("incompatible mods block this upgrade".into());
        }
        if !accept_unknown && !self.unknown.is_empty() {
            return Err("unknown compatibility: cancel or explicitly use --accept-unknown after reviewing the files".into());
        }
        Ok(())
    }
}
struct ModJar {
    path: std::path::PathBuf,
    doc: DocumentMut,
}

// Conservative subset of Maven VersionRange: numeric versions, exact and union
// intervals. Unsupported qualifiers must produce Unknown, never a guessed pass.
fn numeric(v: &str) -> Result<Vec<u64>> {
    let mut numbers = v
        .split('.')
        .map(|n| {
            n.parse::<u64>()
                .map_err(|_| format!("unsupported Maven version {v}"))
        })
        .collect::<Result<Vec<_>>>()?;
    while numbers.last() == Some(&0) {
        numbers.pop();
    }
    Ok(numbers)
}
fn compare(a: &str, b: &str) -> Result<Ordering> {
    Ok(numeric(a)?.cmp(&numeric(b)?))
}
pub fn matches(range: &str, version: &str) -> Result<bool> {
    let mut tail = range.trim();
    if tail.is_empty() {
        return Err("empty version range".into());
    }
    if !tail.starts_with(['[', '(']) {
        // Maven bare versions are recommendations rather than hard restrictions.
        numeric(tail)?;
        numeric(version)?;
        return Ok(true);
    }
    let mut matched = false;
    loop {
        let open = tail.chars().next().ok_or("invalid range")?;
        if !matches!(open, '[' | '(') {
            return Err(format!("invalid version range {range}"));
        }
        let end = tail.find([']', ')']).ok_or("unterminated version range")?;
        let close = tail.as_bytes()[end] as char;
        let body = &tail[1..end];
        let result = if let Some((low, high)) = body.split_once(',') {
            let low = low.trim();
            let high = high.trim();
            if !low.is_empty() && !high.is_empty() && compare(low, high)? == Ordering::Greater {
                return Err("reversed version interval".into());
            }
            let lower = low.is_empty()
                || match compare(version, low)? {
                    Ordering::Greater => true,
                    Ordering::Equal => open == '[',
                    Ordering::Less => false,
                };
            let upper = high.is_empty()
                || match compare(version, high)? {
                    Ordering::Less => true,
                    Ordering::Equal => close == ']',
                    Ordering::Greater => false,
                };
            lower && upper
        } else {
            if open != '[' || close != ']' || body.is_empty() {
                return Err("invalid exact version range".into());
            }
            if body == version {
                true
            } else {
                compare(version, body)? == Ordering::Equal
            }
        };
        matched |= result;
        tail = tail[end + 1..].trim();
        if tail.is_empty() {
            break;
        }
        tail = tail.strip_prefix(',').ok_or("invalid range union")?.trim();
    }
    Ok(matched)
}
fn check(report: &mut Report, path: &Path, label: &str, range: &str, version: &str) {
    match matches(range, version) {
        Ok(true) => {}
        Ok(false) => report.add(
            path,
            format!("{label} requires {range}, target/installed is {version}"),
            true,
        ),
        Err(e) => report.add(
            path,
            format!("{label}: {range} against {version}: {e}"),
            false,
        ),
    }
}
fn manifest_version(path: &Path) -> Option<String> {
    let text = super::zip_text(path, "META-INF/MANIFEST.MF").ok()?;
    // Manifest continuation lines are prefixed by one space.
    let unfolded = text.replace("\r\n ", "").replace("\n ", "");
    unfolded
        .lines()
        .take_while(|l| !l.is_empty())
        .find_map(|line| {
            line.strip_prefix("Implementation-Version: ")
                .map(|v| v.trim().to_owned())
        })
}

pub fn inspect(instance: &Instance, target: &Target) -> Result<Report> {
    let dir = instance.game.join("mods");
    let mut report = Report::default();
    if !dir.exists() {
        return Ok(report);
    }
    let mut paths = fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .map(|entry| entry.map(|e| e.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>>>()?;
    paths.sort();
    let mut jars = Vec::new();
    let mut versions: BTreeMap<String, Option<String>> = BTreeMap::new();
    versions.insert("minecraft".into(), Some(target.minecraft.clone()));
    versions.insert("neoforge".into(), Some(target.version.clone()));
    let mut incomplete_inventory = false;
    for path in paths
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("jar")))
    {
        report.checked += 1;
        let file = fs::File::open(&path).map_err(|e| e.to_string())?;
        let mut archive = match zip::ZipArchive::new(file) {
            Ok(z) => z,
            Err(e) => {
                report.add(&path, format!("unreadable JAR: {e}"), false);
                incomplete_inventory = true;
                continue;
            }
        };
        let names = archive.file_names().map(str::to_owned).collect::<Vec<_>>();
        if names.iter().any(|n| {
            n == "META-INF/jarjar/metadata.json"
                || n.starts_with("META-INF/jars/") && n.ends_with(".jar")
        }) {
            report.add(
                &path,
                "nested JAR dependencies require manual review",
                false,
            );
            incomplete_inventory = true;
        }
        let metadata = ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"]
            .into_iter()
            .find(|n| names.iter().any(|s| s == n));
        let Some(metadata) = metadata else {
            let foreign = names
                .iter()
                .any(|n| n == "fabric.mod.json" || n == "quilt.mod.json");
            report.add(
                &path,
                if foreign {
                    "declares Fabric/Quilt but no NeoForge metadata"
                } else {
                    "no supported loader metadata"
                },
                foreign,
            );
            incomplete_inventory = true;
            continue;
        };
        // Opening by name also validates decompression and CRC.
        drop(archive.by_name(metadata));
        let doc = match super::zip_text(&path, metadata)
            .and_then(|text| text.parse::<DocumentMut>().map_err(|e| e.to_string()))
        {
            Ok(doc) => doc,
            Err(e) => {
                report.add(&path, format!("{metadata}: {e}"), false);
                incomplete_inventory = true;
                continue;
            }
        };
        if metadata == "META-INF/mods.toml" {
            report.add(
                &path,
                "legacy mods.toml: verify this NeoForge generation supports the metadata",
                false,
            );
        }
        match doc.get("modLoader").and_then(|v| v.as_str()) {
            Some("javafml" | "lowcodefml") => {
                match doc.get("loaderVersion").and_then(|v| v.as_str()) {
                    Some(range) => check(
                        &mut report,
                        &path,
                        "FML loaderVersion (not NeoForge)",
                        range,
                        &target.fml,
                    ),
                    None => report.add(&path, "missing FML loaderVersion", false),
                }
            }
            loader => report.add(
                &path,
                format!("custom/unknown language loader {loader:?}"),
                false,
            ),
        }
        match doc.get("mods").and_then(|v| v.as_array_of_tables()) {
            Some(mods) if !mods.is_empty() => {
                for m in mods {
                    let Some(id) = m.get("modId").and_then(|v| v.as_str()) else {
                        incomplete_inventory = true;
                        report.add(&path, "missing modId", false);
                        continue;
                    };
                    let version = m.get("version").and_then(|v| v.as_str()).and_then(|v| {
                        if v == "${file.jarVersion}" {
                            manifest_version(&path)
                        } else if v.contains("${") {
                            None
                        } else {
                            Some(v.to_owned())
                        }
                    });
                    if version.is_none() {
                        report.add(&path, format!("{id}: unresolved mod version"), false);
                    }
                    if versions.contains_key(id) {
                        report.add(&path, format!("duplicate/reserved modId {id}"), true);
                    } else {
                        versions.insert(id.to_owned(), version);
                    }
                }
            }
            _ => {
                report.add(&path, "missing mods list", false);
                incomplete_inventory = true;
            }
        }
        jars.push(ModJar { path, doc });
    }
    for jar in jars {
        let Some(deps) = jar.doc.get("dependencies") else {
            continue;
        };
        let Some(deps) = deps.as_table_like() else {
            report.add(&jar.path, "invalid dependencies table", false);
            continue;
        };
        for (_, list) in deps.iter() {
            let Some(list) = list.as_array_of_tables() else {
                report.add(&jar.path, "invalid dependency list", false);
                continue;
            };
            for dependency in list {
                match dependency
                    .get("side")
                    .and_then(|v| v.as_str())
                    .unwrap_or("BOTH")
                {
                    "CLIENT" if instance.kind == Kind::Server => continue,
                    "SERVER" if instance.kind == Kind::Prism => continue,
                    "CLIENT" | "SERVER" | "BOTH" => {}
                    other => {
                        report.add(&jar.path, format!("unknown dependency side {other}"), false);
                        continue;
                    }
                }
                let mandatory = dependency.get("mandatory").and_then(|v| v.as_bool());
                let ty = dependency.get("type").and_then(|v| v.as_str());
                if ty == Some("optional")
                    || ty == Some("discouraged")
                    || ty.is_none() && mandatory == Some(false)
                {
                    continue;
                }
                if !matches!(ty, Some("required" | "incompatible")) && mandatory != Some(true) {
                    report.add(&jar.path, "unknown dependency requirement type", false);
                    continue;
                }
                let Some(id) = dependency.get("modId").and_then(|v| v.as_str()) else {
                    report.add(&jar.path, "dependency has no modId", false);
                    continue;
                };
                let incompatible = ty == Some("incompatible");
                let range = dependency.get("versionRange").and_then(|v| v.as_str());
                match versions.get(id) {
                    Some(Some(version)) => {
                        if let Some(range) = range {
                            if incompatible {
                                match matches(range, version) {
                                    Ok(true) => report.add(
                                        &jar.path,
                                        format!(
                                            "incompatible with {id} {range}; installed {version}"
                                        ),
                                        true,
                                    ),
                                    Ok(false) => {}
                                    Err(e) => report.add(&jar.path, format!("{id}: {e}"), false),
                                }
                            } else {
                                check(&mut report, &jar.path, id, range, version);
                            }
                        } else {
                            report.add(&jar.path, format!("{id}: missing versionRange"), false);
                        }
                    }
                    Some(None) => report.add(
                        &jar.path,
                        format!("{id}: installed version unresolved"),
                        false,
                    ),
                    None if incompatible => {}
                    None => report.add(
                        &jar.path,
                        format!(
                            "required dependency {id} missing{}",
                            if incomplete_inventory {
                                " or supplied by unresolved metadata/nested JAR"
                            } else {
                                ""
                            }
                        ),
                        !incomplete_inventory || id == "forge",
                    ),
                }
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maven_numeric_intervals_and_unions() {
        assert!(matches("[21.1.242,21.1.252]", "21.1.252").unwrap());
        assert!(!matches("[21.1.242,21.1.252)", "21.1.252").unwrap());
        assert!(matches("(,1.0],[1.2,)", "1.3").unwrap());
        assert!(!matches("(,1.0],[1.2,)", "1.1").unwrap());
        assert!(matches("[4,)", "4.0.44").unwrap());
        assert!(!matches("[21,)", "4.0.44").unwrap());
        assert!(matches("[1.0]", "1").unwrap());
        assert!(matches("[1-beta,)", "1").is_err());
        assert!(matches("[3,1]", "2").is_err());
    }
    #[test]
    fn unknown_confirmation_never_overrides_blockers() {
        let mut r = Report::default();
        r.add(Path::new("manual.jar"), "unknown", false);
        assert!(r.authorize(false).is_err());
        assert!(r.authorize(true).is_ok());
        r.add(Path::new("bad.jar"), "incompatible", true);
        assert!(r.authorize(true).is_err());
    }
}
