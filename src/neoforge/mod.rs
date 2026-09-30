//! Loader upgrades are independent of packwiz's mod-file ownership ledger.
#[cfg(test)]
mod acceptance_tests;
pub mod adapter;
pub mod cli;
pub mod compatibility;
pub mod transaction;
pub mod ui;
pub mod upgrade;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub type Result<T> = std::result::Result<T, String>;
pub const META: &str = "https://meta.prismlauncher.org/v1/net.neoforged";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Kind {
    Server,
    Prism,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub root: PathBuf,
    pub game: PathBuf,
    pub kind: Kind,
    pub version: String,
    pub minecraft: String,
    pub scripts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub version: String,
    pub minecraft: String,
    pub fml: String,
}

pub fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}
pub fn json(path: &Path) -> Result<Value> {
    serde_json::from_str(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))
}
pub fn string(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("missing string {key}"))
}
pub fn zip_text(path: &Path, name: &str) -> Result<String> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let file = zip.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    if file.size() > 4 * 1024 * 1024 {
        return Err(format!("{name}: metadata exceeds 4 MiB"));
    }
    let mut text = String::new();
    file.take(4 * 1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 4 * 1024 * 1024 {
        return Err(format!("{name}: metadata exceeds 4 MiB"));
    }
    Ok(text)
}
pub fn argument(text: &str, flag: &str) -> Result<String> {
    let tokens: Vec<_> = text.split_whitespace().collect();
    let found: Vec<_> = tokens
        .windows(2)
        .filter(|w| w[0] == flag)
        .map(|w| w[1].trim_matches('"'))
        .collect();
    if found.len() != 1 {
        return Err(format!("expected one {flag} in actual launch arguments"));
    }
    Ok(found[0].to_owned())
}
pub fn installer_target(path: &Path) -> Result<Target> {
    let profile: Value = serde_json::from_str(&zip_text(path, "install_profile.json")?)
        .map_err(|e| e.to_string())?;
    let version: Value =
        serde_json::from_str(&zip_text(path, "version.json")?).map_err(|e| e.to_string())?;
    let args = version["arguments"]["game"]
        .as_array()
        .ok_or("unsupported installer arguments")?
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    let target = Target {
        version: argument(&args, "--fml.neoForgeVersion")?,
        minecraft: argument(&args, "--fml.mcVersion")?,
        fml: argument(&args, "--fml.fmlVersion")?,
    };
    if profile["minecraft"] != target.minecraft
        || profile["version"] != format!("neoforge-{}", target.version)
    {
        return Err("installer profile and runtime description disagree".into());
    }
    Ok(target)
}
pub fn detect(root: &Path, extra_scripts: &[String]) -> Result<Instance> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("{}: {e}", root.display()))?;
    if root.join("mmc-pack.json").is_file() {
        if !extra_scripts.is_empty() {
            return Err("--script applies only to servers".into());
        }
        let pack = json(&root.join("mmc-pack.json"))?;
        if pack["formatVersion"] != 1 {
            return Err("unsupported Prism pack format".into());
        }
        let components = pack["components"]
            .as_array()
            .ok_or("missing Prism components")?;
        let component = |uid: &str| -> Result<String> {
            let matches: Vec<_> = components.iter().filter(|c| c["uid"] == uid).collect();
            if matches.len() != 1 || matches[0]["disabled"] == true {
                return Err(format!("expected one enabled Prism {uid} component"));
            }
            let version = string(matches[0], "version")?;
            if matches[0]["cachedVersion"]
                .as_str()
                .is_some_and(|v| v != version)
            {
                return Err(format!(
                    "Prism {uid} selected/cached versions conflict; refresh component in Prism first"
                ));
            }
            Ok(version)
        };
        if components.iter().any(|c| {
            matches!(
                c["uid"].as_str(),
                Some(
                    "net.minecraftforge"
                        | "net.fabricmc.fabric-loader"
                        | "org.quiltmc.quilt-loader"
                )
            )
        }) {
            return Err("multiple loader components in Prism instance".into());
        }
        if root.join("patches/net.neoforged.json").exists()
            || root.join("patches/net.minecraft.json").exists()
        {
            return Err(
                "custom Prism component patch: migrate it manually before upgrading".into(),
            );
        }
        let version = component("net.neoforged")?;
        let minecraft = component("net.minecraft")?;
        let game = if root.join(".minecraft").is_dir() && !root.join("minecraft").exists() {
            root.join(".minecraft")
        } else {
            root.join("minecraft")
        };
        if !game.is_dir() {
            return Err("Prism game directory is missing".into());
        }
        return Ok(Instance {
            root,
            game,
            kind: Kind::Prism,
            version,
            minecraft,
            scripts: vec![],
        });
    }
    let mut scripts: Vec<String> = ["run.sh", "run.bat"]
        .iter()
        .filter(|s| root.join(s).is_file())
        .map(|s| s.to_string())
        .collect();
    for script in extra_scripts {
        crate::pathutil::safe_slash_path(script)?;
        if !scripts.contains(script) {
            scripts.push(script.clone());
        }
    }
    let mut target: Option<Target> = None;
    for script in &scripts {
        let text = read(&root.join(script))?.replace('\\', "/");
        let marker = "libraries/net/neoforged/neoforge/";
        let versions: Vec<_> = text
            .split(marker)
            .skip(1)
            .filter_map(|s| s.split_once('/').map(|v| v.0))
            .collect();
        if versions.is_empty() {
            return Err(format!(
                "{script}: no direct NeoForge args-file reference; migrate wrapper scripts manually"
            ));
        }
        for v in versions {
            crate::pathutil::safe_filename(v)?;
            let base = root.join(format!("{marker}{v}"));
            for args_name in ["unix_args.txt", "win_args.txt"] {
                let args = read(&base.join(args_name))?;
                let t = Target {
                    version: argument(&args, "--fml.neoForgeVersion")?,
                    minecraft: argument(&args, "--fml.mcVersion")?,
                    fml: argument(&args, "--fml.fmlVersion")?,
                };
                if t.version != v
                    || target
                        .as_ref()
                        .is_some_and(|old| old.version != t.version || old.minecraft != t.minecraft)
                {
                    return Err("conflicting actual server launch versions".into());
                }
                if !base.join(format!("neoforge-{v}-server.jar")).is_file() {
                    return Err("NeoForge server artifact is missing".into());
                }
                target = Some(t);
            }
        }
    }
    let t = target
        .ok_or("no supported NeoForge installation; pack.toml is not installation evidence")?;
    Ok(Instance {
        game: root.clone(),
        root,
        kind: Kind::Server,
        version: t.version,
        minecraft: t.minecraft,
        scripts,
    })
}
pub fn same_game(instance: &Instance, target: &Target) -> Result<()> {
    if instance.minecraft != target.minecraft {
        return Err(format!(
            "cross-Minecraft upgrade blocked: {} → {}",
            instance.minecraft, target.minecraft
        ));
    }
    if !stable(&target.version) {
        return Err("beta targets are unavailable in this release".into());
    }
    Ok(())
}
pub fn stable(version: &str) -> bool {
    version.split('.').count() >= 3
        && version
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}
pub fn get_json(url: &str) -> Result<Value> {
    let mut response = crate::http::agent()
        .get(url)
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}
pub fn candidates(index: &Value, mc: &str) -> Result<Vec<String>> {
    let mut versions = index["versions"]
        .as_array()
        .ok_or("missing NeoForge version index")?
        .iter()
        .filter(|v| {
            v["type"] == "release"
                && v["requires"].as_array().is_some_and(|req| {
                    req.iter()
                        .any(|r| r["uid"] == "net.minecraft" && r["equals"] == mc)
                })
        })
        .filter_map(|v| {
            v["version"]
                .as_str()
                .filter(|s| stable(s))
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    versions.sort_by_key(|v| {
        v.split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    });
    versions.reverse();
    versions.dedup();
    Ok(versions)
}

pub fn stopped(instance: &Instance) -> Result<()> {
    let system = sysinfo::System::new_all();
    for (pid, process) in system.processes() {
        let name = process.name().to_string_lossy().to_lowercase();
        if instance.kind == Kind::Prism && name.contains("prismlauncher") {
            return Err(format!(
                "close Prism Launcher before changing component files (PID {pid})"
            ));
        }
        let java = name.starts_with("java")
            || process
                .exe()
                .and_then(|p| p.file_name())
                .is_some_and(|p| p.to_string_lossy().to_lowercase().starts_with("java"));
        let root = normalized(&instance.root.to_string_lossy());
        let game = instance
            .game
            .canonicalize()
            .unwrap_or_else(|_| instance.game.clone());
        let game_text = normalized(&game.to_string_lossy());
        let cmd = process
            .cmd()
            .iter()
            .map(|a| normalized(&a.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(" ");
        let cwd = process.cwd();
        let inside = cwd.is_some_and(|p| p.starts_with(&instance.root) || p.starts_with(&game));
        if java && (inside || cmd.contains(&root) || cmd.contains(&game_text) || cwd.is_none()) {
            return Err(format!(
                "instance is running or Java process cannot be inspected (PID {pid}); stop it before upgrading"
            ));
        }
    }
    Ok(())
}
fn normalized(s: &str) -> String {
    let s = s.replace('\\', "/");
    if cfg!(windows) { s.to_lowercase() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_game_candidates_and_beta_filter() {
        let index = serde_json::json!({"versions": [
            {"version":"21.1.252","type":"release","requires":[{"uid":"net.minecraft","equals":"1.21.1"}]},
            {"version":"21.1.99-beta","type":"release","requires":[{"uid":"net.minecraft","equals":"1.21.1"}]},
            {"version":"21.1.999","type":"release","requires":[{"uid":"net.minecraft","equals":"1.21"}]}]});
        assert_eq!(candidates(&index, "1.21.1").unwrap(), ["21.1.252"]);
    }
    #[test]
    fn game_check_has_no_force_escape() {
        let i = Instance {
            root: PathBuf::new(),
            game: PathBuf::new(),
            kind: Kind::Server,
            version: "21.1.0-beta".into(),
            minecraft: "1.21.1".into(),
            scripts: vec![],
        };
        let t = Target {
            version: "21.2.1".into(),
            minecraft: "1.21.2".into(),
            fml: "4".into(),
        };
        assert!(same_game(&i, &t).unwrap_err().contains("cross-Minecraft"));
    }
}
