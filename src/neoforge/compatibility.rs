use super::{Instance, Kind, Result, Target, jarjar, maven::compare};
use serde::Serialize;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
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
    pub(super) fn add(&mut self, file: &Path, reason: impl Into<String>, blocked: bool) {
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
    manifest: Option<String>,
}
impl ModJar {
    fn ids(&self) -> BTreeSet<String> {
        self.doc
            .get("mods")
            .and_then(|v| v.as_array_of_tables())
            .into_iter()
            .flat_map(|mods| mods.iter())
            .filter_map(|m| m.get("modId").and_then(|v| v.as_str()).map(str::to_owned))
            .collect()
    }
    fn version(&self, m: &toml_edit::Table) -> Option<String> {
        match m.get("version").and_then(|v| v.as_str()) {
            Some("${file.jarVersion}") => self.manifest.clone(),
            Some(v) if !v.contains("${") => Some(v.to_owned()),
            _ => None,
        }
    }
}

pub fn matches(range: &str, version: &str) -> Result<bool> {
    if range.len() > 4096 || range.bytes().filter(|b| matches!(b, b']' | b')')).count() > 64 {
        return Err("version range exceeds complexity limit".into());
    }
    compare(version, version)?;
    let mut tail = range.trim();
    if tail.is_empty() {
        return Err("empty version range".into());
    }
    if !tail.starts_with(['[', '(']) {
        // Maven bare versions are recommendations rather than hard restrictions.
        compare(tail, version)?;
        return Ok(true);
    }
    let mut matched = false;
    let mut previous_high: Option<String> = None;
    let mut interval_count = 0;
    loop {
        let open = tail.chars().next().ok_or("invalid range")?;
        if !matches!(open, '[' | '(') {
            return Err(format!("invalid version range {range}"));
        }
        let end = tail.find([']', ')']).ok_or("unterminated version range")?;
        let close = tail.as_bytes()[end] as char;
        let body = &tail[1..end];
        let low_bound = body.split_once(',').map_or(body, |(low, _)| low).trim();
        if interval_count > 0
            && (low_bound.is_empty()
                || previous_high.as_deref().is_none_or(|high| {
                    !matches!(
                        compare(low_bound, high),
                        Ok(Ordering::Equal | Ordering::Greater)
                    )
                }))
        {
            return Err("overlapping or unordered range union".into());
        }
        let result = if let Some((low, high)) = body.split_once(',') {
            if high.contains(',') {
                return Err("invalid version interval".into());
            }
            let low = low.trim();
            let high = high.trim();
            if !low.is_empty() && !high.is_empty() {
                let order = compare(low, high)?;
                if order == Ordering::Greater
                    || order == Ordering::Equal && (open != '[' || close != ']')
                {
                    return Err("reversed or empty version interval".into());
                }
            }
            previous_high = (!high.is_empty()).then(|| high.to_owned());
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
            previous_high = Some(body.to_owned());
            if body == version {
                true
            } else {
                compare(version, body)? == Ordering::Equal
            }
        };
        interval_count += 1;
        matched |= result;
        tail = tail[end + 1..].trim();
        if tail.is_empty() {
            break;
        }
        tail = tail.strip_prefix(',').ok_or("invalid range union")?.trim();
    }
    Ok(matched)
}
fn dependency_matches(id: &str, range: &str, version: &str) -> Result<bool> {
    if matches(range, version)? {
        return Ok(true);
    }
    // Explicit FML 4 VersionSupportMatrix aliases for Minecraft 1.21.1.
    // No generic fallback to other patch releases.
    if id == "minecraft" && version == "1.21.1" {
        return matches(range, "1.21");
    }
    if id == "neoforge" && version.starts_with("21.1.") {
        return matches(range, "21.0.166");
    }
    Ok(false)
}
fn spi_entrypoint(archive: &mut jarjar::Archive) -> Result<Option<bool>> {
    // FML 4 TransformerDiscovererConstants.SERVICES: these JARs load in the
    // service layer and do not need neoforge.mods.toml, even with Fabric metadata.
    let mut recognized = false;
    let mut dynamic_inventory = false;
    let names = archive
        .file_names()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    for service in [
        "cpw.mods.modlauncher.api.ITransformationService",
        "net.neoforged.neoforgespi.locating.IModFileCandidateLocator",
        "net.neoforged.neoforgespi.locating.IModFileReader",
        "net.neoforged.neoforgespi.locating.IDependencyLocator",
        "net.neoforged.neoforgespi.earlywindow.GraphicsBootstrapper",
        "net.neoforged.neoforgespi.earlywindow.ImmediateWindowProvider",
    ] {
        let name = format!("META-INF/services/{service}");
        if !names.contains(&name) {
            continue;
        }
        let text = jarjar::text(archive, &name)?;
        let providers = text
            .lines()
            .map(|line| line.split('#').next().unwrap().trim())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>();
        if providers.is_empty()
            || providers
                .iter()
                .any(|p| !names.contains(&format!("{}.class", p.replace('.', "/"))))
        {
            return Err(format!("{service}: missing SPI provider class"));
        }
        recognized = true;
        dynamic_inventory |= matches!(
            service,
            "net.neoforged.neoforgespi.locating.IModFileCandidateLocator"
                | "net.neoforged.neoforgespi.locating.IModFileReader"
                | "net.neoforged.neoforgespi.locating.IDependencyLocator"
        );
    }
    Ok(recognized.then_some(dynamic_inventory))
}
fn check(
    report: &mut Report,
    path: &Path,
    label: &str,
    range: &str,
    version: &str,
    authoritative: bool,
) {
    match dependency_matches(label, range, version) {
        Ok(true) => {}
        Ok(false) => report.add(
            path,
            if authoritative {
                format!("{label} requires {range}, target/installed is {version}")
            } else {
                format!("{label}: incomplete inventory; cannot resolve {range} against {version}")
            },
            authoritative,
        ),
        Err(e) => report.add(
            path,
            format!("{label}: {range} against {version}: {e}"),
            false,
        ),
    }
}
fn manifest_version(archive: &mut jarjar::Archive) -> Option<String> {
    let text = jarjar::text(archive, "META-INF/MANIFEST.MF").ok()?;
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
    let mut primary = BTreeMap::<String, Vec<ModJar>>::new();
    let mut ambiguous = BTreeSet::<String>::new();
    let mut occupied = BTreeMap::<String, (std::path::PathBuf, BTreeSet<String>)>::new();
    let mut versions: BTreeMap<String, Option<String>> = BTreeMap::new();
    versions.insert("minecraft".into(), Some(target.minecraft.clone()));
    versions.insert("neoforge".into(), Some(target.version.clone()));
    let paths = paths
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("jar")))
        .collect();
    let (archives, mut incomplete_inventory) = jarjar::inventory(paths, &mut report);
    for jar in archives {
        let path = jar.path.clone();
        let nested = jar.nested;
        let mut archive = match jar.open() {
            Ok(a) => a,
            Err(e) => {
                report.add(&path, format!("unreadable JAR: {e}"), false);
                incomplete_inventory = true;
                continue;
            }
        };
        let names = archive.file_names().map(str::to_owned).collect::<Vec<_>>();
        let spi = spi_entrypoint(&mut archive);
        match &spi {
            Ok(Some(dynamic)) => incomplete_inventory |= dynamic,
            Err(e) => {
                report.add(&path, e, false);
                incomplete_inventory = true;
            }
            _ => {}
        }
        let metadata = ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"]
            .into_iter()
            .find(|n| names.iter().any(|s| s == n));
        let Some(metadata) = metadata else {
            if !matches!(spi, Ok(None)) {
                continue;
            }
            if nested {
                // JarJar readers treat metadata-free contents as libraries;
                // Fabric metadata does not turn a bundled library into a mod.
                continue;
            }
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
        let doc = match jarjar::text(&mut archive, metadata)
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
        let jar = ModJar {
            path,
            doc,
            manifest: manifest_version(&mut archive),
        };
        let first = jar
            .doc
            .get("mods")
            .and_then(|v| v.as_array_of_tables())
            .and_then(|mods| mods.get(0));
        let id = first.and_then(|m| m.get("modId")).and_then(|v| v.as_str());
        if let Some(id) = id {
            if matches!(id, "minecraft" | "neoforge") {
                jars.push(jar);
                continue;
            }
            if ambiguous.contains(id) {
                continue;
            }
            primary.entry(id.to_owned()).or_default().push(jar);
        } else {
            jars.push(jar);
        }
    }
    let uncertain_ids = ambiguous.clone();
    for (id, mut group) in primary {
        if uncertain_ids.contains(&id) {
            continue;
        }
        if group.len() == 1 {
            jars.push(group.pop().unwrap());
            continue;
        }
        let mut best: Option<(String, ModJar)> = None;
        let mut unresolved = vec![];
        for jar in group {
            let first = jar.doc["mods"]
                .as_array_of_tables()
                .unwrap()
                .get(0)
                .unwrap();
            match jar.version(first).filter(|v| compare(v, v).is_ok()) {
                Some(version) => {
                    if best
                        .as_ref()
                        .is_none_or(|(old, _)| compare(&version, old) == Ok(Ordering::Greater))
                    {
                        best = Some((version, jar));
                    }
                }
                None => unresolved.push(jar),
            }
        }
        if unresolved.is_empty() {
            jars.push(best.unwrap().1);
            continue;
        }
        if let Some((_, jar)) = best {
            unresolved.push(jar);
        }
        let path = unresolved[0].path.clone();
        let mut ids = unresolved[0].ids();
        for jar in &unresolved[1..] {
            let candidate = jar.ids();
            ids.retain(|id| candidate.contains(id));
        }
        report.add(
            &path,
            format!("{id}: unresolved duplicate version selection"),
            false,
        );
        incomplete_inventory = true;
        ambiguous.extend(unresolved.iter().flat_map(ModJar::ids));
        occupied.insert(id, (path, ids));
    }
    let mut occupied_ids = BTreeSet::new();
    for (path, ids) in occupied.values() {
        for id in ids {
            if !occupied_ids.insert(id.clone()) || versions.contains_key(id) {
                report.add(path, format!("duplicate/reserved modId {id}"), true);
            }
        }
    }
    for jar in &jars {
        let path = &jar.path;
        let doc = &jar.doc;
        match doc.get("modLoader").and_then(|v| v.as_str()) {
            Some("javafml" | "lowcodefml") => {
                match doc.get("loaderVersion").and_then(|v| v.as_str()) {
                    Some(range) => check(
                        &mut report,
                        &path,
                        "FML loaderVersion (not NeoForge)",
                        range,
                        &target.fml,
                        true,
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
                    let version = jar.version(m).filter(|_| !ambiguous.contains(id));
                    if version.is_none() {
                        report.add(&path, format!("{id}: unresolved mod version"), false);
                    }
                    if versions.contains_key(id) || occupied_ids.contains(id) {
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
                let type_name = dependency
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(str::to_ascii_lowercase);
                let ty = type_name
                    .as_deref()
                    .or_else(|| (!dependency.contains_key("type")).then_some("required"));
                if ty == Some("optional") || ty == Some("discouraged") {
                    continue;
                }
                if !matches!(ty, Some("required" | "incompatible")) {
                    report.add(&jar.path, "unknown dependency requirement type", false);
                    continue;
                }
                let Some(id) = dependency.get("modId").and_then(|v| v.as_str()) else {
                    report.add(&jar.path, "dependency has no modId", false);
                    continue;
                };
                let incompatible = ty == Some("incompatible");
                let range = dependency
                    .get("versionRange")
                    .map_or(Some("*"), |v| v.as_str());
                match versions.get(id) {
                    Some(Some(version)) => {
                        let authoritative =
                            !incomplete_inventory || matches!(id, "minecraft" | "neoforge");
                        if let Some(range) = range {
                            if incompatible {
                                match dependency_matches(id, range, version) {
                                    Ok(true) => report.add(
                                        &jar.path,
                                        format!(
                                            "incompatible with {id} {range}; installed {version}"
                                        ),
                                        authoritative,
                                    ),
                                    Ok(false) => {}
                                    Err(e) => report.add(&jar.path, format!("{id}: {e}"), false),
                                }
                            } else {
                                check(&mut report, &jar.path, id, range, version, authoritative);
                            }
                        } else {
                            report.add(&jar.path, format!("{id}: invalid versionRange"), false);
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
        assert!(matches("[1-beta,)", "1").unwrap());
        assert!(matches("[3,1]", "2").is_err());
    }
    #[test]
    fn fml_121_aliases() {
        assert!(dependency_matches("minecraft", "[1.21,1.21.1)", "1.21.1").unwrap());
        assert!(!dependency_matches("minecraft", "[1.20,1.21)", "1.21.1").unwrap());
        assert!(!dependency_matches("lib", "[1.21]", "1.21.1").unwrap());
        assert!(dependency_matches("neoforge", "[21.0,21.1)", "21.1.251").unwrap());
        assert!(!dependency_matches("neoforge", "[21.0,21.1)", "21.2.1").unwrap());
    }
    #[test]
    fn maven_qualifiers() {
        for (range, version, expected) in [
            ("*", "2101.7.2-build.377", true),
            (
                "[2101.7.2-build.376,2101.7.2-build.378)",
                "2101.7.2-build.377",
                true,
            ),
            ("[2,3)", "2.1+mc1.21.1", true),
            ("[1]", "1-release", true),
            ("[1,)", "1-rc1", false),
            ("[1-beta2]", "1b2", true),
            // Maven 3.8.5 intentionally differs from 3.9.9 for .alpha1.
            ("[1-alpha1]", "1.alpha1", false),
            ("[1,2)", "999999999999999999999", false),
        ] {
            assert_eq!(
                matches(range, version).unwrap(),
                expected,
                "{range}: {version}"
            );
        }
        for range in [
            "[2,1]",
            "[1,1)",
            "[1,2,3]",
            "[1,3],[2,4]",
            "[1],",
            "[${version}]",
        ] {
            assert!(matches(range, "1").is_err(), "{range}");
        }
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
