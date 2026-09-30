use super::{
    Instance, Kind, Result, Target,
    transaction::{self, Change},
};
use crate::sha256::sha256_file_hex as hash;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub fn installer(version: &str, local: Option<&Path>, cache: &Path) -> Result<PathBuf> {
    if !super::stable(version) {
        return Err("only stable NeoForge targets are supported".into());
    }
    fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    let name = format!("neoforge-{version}-installer.jar");
    let url =
        format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/{name}");
    let expected = crate::http::http_get_to_string(&format!("{url}.sha256"))?;
    let expected = expected
        .split_whitespace()
        .next()
        .ok_or("empty official checksum")?;
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid official SHA-256".into());
    }
    let dst = cache.join(name);
    if let Some(path) = local {
        if hash(path)? != expected.to_lowercase() {
            return Err(format!("{}: official SHA-256 mismatch", path.display()));
        }
        fs::copy(path, &dst).map_err(|e| e.to_string())?;
    } else if !dst.is_file() || hash(&dst)? != expected.to_lowercase() {
        crate::http::http_get_to_file(&url, &dst)?;
    }
    if hash(&dst)? != expected.to_lowercase() {
        return Err("downloaded installer SHA-256 mismatch".into());
    }
    Ok(dst)
}
pub fn prism_metadata(target: &Target) -> Result<Value> {
    let index = super::get_json(&format!("{}/index.json", super::META))?;
    let entry = index["versions"]
        .as_array()
        .ok_or("invalid Prism index")?
        .iter()
        .find(|v| v["version"] == target.version)
        .ok_or("target absent from Prism metadata")?;
    let text =
        crate::http::http_get_to_string(&format!("{}/{}.json", super::META, target.version))?;
    let mut sha = crate::sha256::Sha256::new();
    sha.update(text.as_bytes());
    let digest = sha
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if entry["sha256"] != digest {
        return Err("Prism component metadata SHA-256 mismatch".into());
    }
    let meta: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if meta["uid"] != "net.neoforged"
        || meta["version"] != target.version
        || !meta["requires"].as_array().is_some_and(|req| {
            req.iter()
                .any(|r| r["uid"] == "net.minecraft" && r["equals"] == target.minecraft)
        })
    {
        return Err("Prism target does not require the exact installed Minecraft version".into());
    }
    if super::argument(
        meta["minecraftArguments"]
            .as_str()
            .ok_or("missing Prism arguments")?,
        "--fml.fmlVersion",
    )? != target.fml
    {
        return Err("Prism FML version differs from official installer".into());
    }
    Ok(meta)
}
pub fn switch_prism(instance: &Instance, target: &Target, meta: &Value) -> Result<Vec<u8>> {
    let mut pack = super::json(&instance.root.join("mmc-pack.json"))?;
    let component = pack["components"]
        .as_array_mut()
        .ok_or("invalid components")?
        .iter_mut()
        .find(|c| c["uid"] == "net.neoforged")
        .ok_or("missing NeoForge component")?;
    component["version"] = json!(target.version);
    let obj = component.as_object_mut().ok_or("invalid component")?;
    for key in [
        "cachedVersion",
        "cachedRequires",
        "cachedConflicts",
        "cachedVolatile",
        "cachedName",
    ] {
        obj.remove(key);
    }
    component["cachedVersion"] = json!(target.version);
    component["cachedName"] = json!("NeoForge");
    component["cachedRequires"] = meta["requires"].clone();
    serde_json::to_vec_pretty(&pack).map_err(|e| e.to_string())
}
pub fn migrated_script(text: &str, old: &str, new: &str) -> Result<String> {
    let unix = format!("libraries/net/neoforged/neoforge/{old}/");
    let windows = unix.replace('/', "\\");
    if !text.contains(&unix) && !text.contains(&windows) {
        return Err(
            "script is not a directly migratable NeoForge launch script; migrate manually".into(),
        );
    }
    Ok(text
        .replace(&unix, &format!("libraries/net/neoforged/neoforge/{new}/"))
        .replace(
            &windows,
            &format!("libraries\\net\\neoforged\\neoforge\\{new}\\"),
        ))
}
pub fn files(dir: &Path) -> Result<Vec<PathBuf>> {
    transaction::safe_path(dir)?;
    let mut paths = vec![];
    for e in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = e.map_err(|e| e.to_string())?.path();
        transaction::safe_path(&path)?;
        if path.is_dir() {
            paths.extend(files(&path)?);
        } else if path.is_file() {
            paths.push(path);
        } else {
            return Err(format!("unsupported file type: {}", path.display()));
        }
    }
    paths.sort();
    Ok(paths)
}
/// Seed only files with ownership evidence from the official target profile.
/// The installer verifies Minecraft's raw server artifact against Mojang again.
pub fn seed_server(instance: &Instance, stage: &Path, jar: &Path, target: &Target) -> Result<()> {
    let profile: Value = serde_json::from_str(&super::zip_text(jar, "install_profile.json")?)
        .map_err(|e| e.to_string())?;
    for library in profile["libraries"]
        .as_array()
        .ok_or("missing installer libraries")?
    {
        let artifact = &library["downloads"]["artifact"];
        let (Some(path), Some(expected)) = (artifact["path"].as_str(), artifact["sha1"].as_str())
        else {
            continue;
        };
        crate::pathutil::safe_slash_path(path)?;
        let rel = format!("libraries/{path}");
        let src = instance.root.join(&rel);
        if src.is_file() && crate::sha1::sha1_file_hex(&src)? == expected {
            let dst = stage.join(&rel);
            fs::create_dir_all(dst.parent().ok_or("missing library parent")?)
                .map_err(|e| e.to_string())?;
            fs::copy(&src, dst).map_err(|e| e.to_string())?;
        }
    }
    let rel = format!(
        "libraries/net/minecraft/server/{0}/server-{0}.jar",
        target.minecraft
    );
    crate::pathutil::safe_slash_path(&rel)?;
    let src = instance.root.join(&rel);
    if src.is_file() {
        let dst = stage.join(&rel);
        fs::create_dir_all(dst.parent().ok_or("missing Minecraft parent")?)
            .map_err(|e| e.to_string())?;
        fs::copy(src, dst).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn install_server(stage: &Path, jar: &Path, java: &str, target: &Target) -> Result<()> {
    let output = Command::new(java)
        .arg("-version")
        .output()
        .map_err(|e| format!("Java {java}: {e}; select --java explicitly"))?;
    if !output.status.success() {
        return Err("Java preflight failed".into());
    }
    let version_text = String::from_utf8_lossy(&output.stderr).to_string()
        + &String::from_utf8_lossy(&output.stdout);
    let major = version_text
        .split('"')
        .nth(1)
        .and_then(|v| v.split('.').next())
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or("cannot determine Java major version")?;
    let mc = target
        .minecraft
        .split('.')
        .map(|v| v.parse::<u32>().unwrap_or(0))
        .collect::<Vec<_>>();
    let minimum = if mc.as_slice() >= [1, 20, 5].as_slice() {
        21
    } else {
        17
    };
    if major < minimum {
        return Err(format!("Java {minimum}+ required; selected Java {major}"));
    }
    let log = fs::File::create(stage.join("installer.log")).map_err(|e| e.to_string())?;
    let stderr = log.try_clone().map_err(|e| e.to_string())?;
    let status = Command::new(java)
        .arg("-jar")
        .arg(jar)
        .arg("--installServer")
        .arg(stage)
        .current_dir(stage)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .status()
        .map_err(|e| format!("installer: {e}"))?;
    if !status.success() {
        return Err(format!(
            "installer failed ({status}); original installation unchanged; log {}",
            stage.join("installer.log").display()
        ));
    }
    validate_server(stage, target)
}
pub fn validate_server(stage: &Path, target: &Target) -> Result<()> {
    let base = stage.join(format!(
        "libraries/net/neoforged/neoforge/{}",
        target.version
    ));
    for name in ["unix_args.txt", "win_args.txt"] {
        let args = super::read(&base.join(name))?;
        for (flag, value) in [
            ("--fml.neoForgeVersion", &target.version),
            ("--fml.mcVersion", &target.minecraft),
            ("--fml.fmlVersion", &target.fml),
        ] {
            if super::argument(&args, flag)? != *value {
                return Err(format!("installed {name} has wrong {flag}"));
            }
        }
        // All path references must resolve inside the isolated installation.
        for token in args.split([' ', '\n', '\r', ':', ';', ',', '"', '=']) {
            if token.starts_with("libraries/") {
                crate::pathutil::safe_slash_path(token)?;
                if !stage.join(token).exists() {
                    return Err(format!("installed argument references missing {token}"));
                }
            }
        }
    }
    for classifier in ["server", "universal"] {
        let path = base.join(format!("neoforge-{}-{classifier}.jar", target.version));
        let file = fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        if archive.is_empty() {
            return Err("empty installed NeoForge artifact".into());
        }
    }
    Ok(())
}
fn staged_bytes(stage: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    let path = stage.join(name);
    fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}
pub fn pack_bytes(path: &Path, target: &Target) -> Result<Vec<u8>> {
    let mut doc = super::read(path)?
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(mc) = doc
        .get("versions")
        .and_then(|v| v.get("minecraft"))
        .and_then(|v| v.as_str())
    {
        if mc != target.minecraft {
            return Err(format!(
                "{} declares Minecraft {mc}; actual {}",
                path.display(),
                target.minecraft
            ));
        }
    }
    doc["versions"]["neoforge"] = toml_edit::value(&target.version);
    Ok(doc.to_string().into_bytes())
}
pub fn changes(
    instance: &Instance,
    target: &Target,
    stage: &Path,
    prism: Option<&Value>,
    external: Option<&Path>,
) -> Result<Vec<Change>> {
    let mut changes = vec![];
    let mut managed = BTreeMap::new();
    if instance.kind == Kind::Server {
        validate_server(stage, target)?;
        let ledger_path = instance.root.join(transaction::CONTROL).join("state.json");
        let previous = if ledger_path.exists() {
            super::json(&ledger_path)?
        } else {
            json!({})
        };
        for source in files(&stage.join("libraries"))? {
            let rel = source.strip_prefix(stage).map_err(|e| e.to_string())?;
            let key = crate::pathutil::to_slash(rel);
            let path = instance.root.join(rel);
            transaction::safe_path(&path)?;
            let after = hash(&source)?;
            managed.insert(key.clone(), after.clone());
            if path.exists() {
                let before = hash(&path)?;
                if before == after {
                    continue;
                }
                if previous["managed"][&key] != before {
                    return Err(format!(
                        "unowned conflicting library {}; no overwrite; reconcile manually",
                        path.display()
                    ));
                }
            }
            changes.push(Change { path, source });
        }
        for (i, script) in instance.scripts.iter().enumerate() {
            let path = instance.root.join(script);
            let bytes = migrated_script(&super::read(&path)?, &instance.version, &target.version)?;
            let source = staged_bytes(stage, &format!("script-{i}"), bytes.as_bytes())?;
            managed.insert(script.clone(), hash(&source)?);
            changes.push(Change { path, source });
        }
    } else {
        let path = instance.root.join("mmc-pack.json");
        let bytes = switch_prism(
            instance,
            target,
            prism.ok_or("missing verified Prism metadata")?,
        )?;
        let source = staged_bytes(stage, "prism-pack.json", &bytes)?;
        managed.insert("mmc-pack.json".into(), hash(&source)?);
        changes.push(Change { path, source });
    }
    let mut packs = vec![instance.game.join("pack.toml")];
    if instance.root != instance.game {
        packs.push(instance.root.join("pack.toml"));
    }
    if let Some(external) = external {
        let root = external
            .canonicalize()
            .map_err(|e| format!("external pack root: {e}"))?;
        packs.push(root.join("pack.toml"));
    }
    packs.sort();
    packs.dedup();
    for (i, path) in packs.into_iter().filter(|p| p.exists()).enumerate() {
        transaction::safe_path(&path)?;
        let source = staged_bytes(
            stage,
            &format!("pack-{i}.toml"),
            &pack_bytes(&path, target)?,
        )?;
        changes.push(Change { path, source });
    }
    let state = json!({"format":1,"status":"installed","kind":instance.kind,"minecraft":target.minecraft,"neoforge":target.version,"fml":target.fml,"scripts":instance.scripts,"managed":managed});
    changes.push(Change {
        path: instance.root.join(transaction::CONTROL).join("state.json"),
        source: staged_bytes(
            stage,
            "state.json",
            &serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?,
        )?,
    });
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scripts_keep_java_memory_and_custom_arguments_on_all_platforms() {
        for text in [
            "\"C:\\Java 21\\java.exe\" -Xmx8G @user_jvm_args.txt @libraries\\net\\neoforged\\neoforge\\21.1.242\\win_args.txt nogui %*\r\n",
            "\"/Applications/Java 21/bin/java\" -Xmx8G @user_jvm_args.txt @libraries/net/neoforged/neoforge/21.1.242/unix_args.txt nogui \"$@\"\n",
        ] {
            let result = migrated_script(text, "21.1.242", "21.1.252").unwrap();
            assert_eq!(result, text.replace("21.1.242", "21.1.252"));
        }
        assert!(migrated_script("exec other-launcher", "21.1.242", "21.1.252").is_err());
    }
}
