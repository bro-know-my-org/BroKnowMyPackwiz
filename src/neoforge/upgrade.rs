use super::{Instance, Kind, Result, Target, adapter, compatibility, transaction};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Options {
    pub root: PathBuf,
    pub target: String,
    pub installer: Option<PathBuf>,
    pub java: String,
    pub scripts: Vec<String>,
    pub external: Option<PathBuf>,
    pub accept_unknown: bool,
}
pub struct Prepared {
    pub instance: Instance,
    pub target: Target,
    pub report: compatibility::Report,
    pub stage: PathBuf,
    jar: PathBuf,
    prism: Option<Value>,
    mods: BTreeMap<String, String>,
}
impl Prepared {
    pub fn summary(&self) -> Value {
        json!({"instance":self.instance,"target":self.target,"precheck":self.report,"stage":self.stage,
            "notice":"Static metadata checks do not guarantee runtime compatibility. Launch the game/server yourself after upgrade.",
            "settings":"Original Prism instance.cfg / server Java, JVM and program arguments are preserved. Review custom wrapper scripts manually."})
    }
}
fn fingerprint(game: &Path) -> Result<BTreeMap<String, String>> {
    let dir = game.join("mods");
    let mut map = BTreeMap::new();
    if !dir.exists() {
        return Ok(map);
    }
    for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("jar"))
        {
            map.insert(
                path.to_string_lossy().to_string(),
                crate::sha256::sha256_file_hex(&path)?,
            );
        }
    }
    Ok(map)
}
pub fn prepare(options: &Options) -> Result<Prepared> {
    let instance = super::detect(&options.root, &options.scripts)?;
    super::stopped(&instance)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let stage = instance
        .root
        .join(transaction::CONTROL)
        .join(format!("work_{stamp}"));
    transaction::safe_path(&stage)?;
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let jar = adapter::installer(&options.target, options.installer.as_deref(), &stage)?;
    let target = super::installer_target(&jar)?;
    if target.version != options.target {
        return Err("selected installer version differs from requested target".into());
    }
    super::same_game(&instance, &target)?;
    let prism = if instance.kind == Kind::Prism {
        Some(adapter::prism_metadata(&target)?)
    } else {
        None
    };
    let mods = fingerprint(&instance.game)?;
    let report = compatibility::inspect(&instance, &target)?;
    if mods != fingerprint(&instance.game)? {
        return Err("mods changed during precheck; retry after stopping other tools".into());
    }
    Ok(Prepared {
        instance,
        target,
        report,
        stage,
        jar,
        prism,
        mods,
    })
}
pub fn execute(plan: &Prepared, options: &Options) -> Result<Value> {
    plan.report.authorize(options.accept_unknown)?;
    super::stopped(&plan.instance)?;
    if plan.instance.version == plan.target.version {
        return Ok(json!({"status":"unchanged","version":plan.target.version}));
    }
    if plan.instance.kind == Kind::Server {
        adapter::install_server(&plan.stage, &plan.jar, &options.java, &plan.target)?;
    }
    super::stopped(&plan.instance)?;
    let current = super::detect(&plan.instance.root, &options.scripts)?;
    if current.version != plan.instance.version || current.minecraft != plan.instance.minecraft {
        return Err("installation changed since precheck".into());
    }
    if fingerprint(&plan.instance.game)? != plan.mods {
        return Err("mods changed since precheck; upgrade cancelled".into());
    }
    let changes = adapter::changes(
        &plan.instance,
        &plan.target,
        &plan.stage,
        plan.prism.as_ref(),
        options.external.as_deref(),
    )?;
    let backup = transaction::apply(&plan.instance.root, &changes, |after| {
        super::stopped(&plan.instance)?;
        if fingerprint(&plan.instance.game)? != plan.mods {
            return Err("mods changed during switch".into());
        }
        if after {
            let current = super::detect(&plan.instance.root, &options.scripts)?;
            if current.version != plan.target.version || current.minecraft != plan.target.minecraft
            {
                return Err("post-switch installation verification failed".into());
            }
        }
        Ok(())
    })?;
    Ok(
        json!({"status":"upgraded","minecraft":plan.target.minecraft,"neoforge":plan.target.version,"backup":backup,"stage":plan.stage,
        "next":"Launch the original instance yourself. If startup fails: bkmpw neoforge rollback <instance>. Backups are retained until explicit clean.","precheck":plan.report}),
    )
}
pub struct Guards {
    _pack: crate::operation::lock::WriteLocks,
    _loader: transaction::Lock,
}
pub fn guards(root: &Path, external: Option<&Path>) -> Result<Guards> {
    let loader = transaction::lock(root)?;
    let mut paths = vec![root.to_owned()];
    paths.extend(recorded_external(root)?);
    if let Some(p) = external {
        paths.push(p.to_owned());
    }
    let state = crate::operation::durable::user_state().map_err(|e| e.to_string())?;
    let pack =
        crate::operation::lock::WriteLocks::acquire(&state, &paths).map_err(|e| e.to_string())?;
    let shell = maintenance_instance(root)?;
    super::stopped(&shell)?;
    // Recovery happens before actual-install detection, since a interrupted
    // switch may temporarily have inconsistent launch files.
    transaction::recover(root)?;
    Ok(Guards {
        _pack: pack,
        _loader: loader,
    })
}
fn maintenance_instance(root: &Path) -> Result<Instance> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let prism = root.join("mmc-pack.json").exists();
    let game = if prism {
        if root.join(".minecraft").is_dir() && !root.join("minecraft").exists() {
            root.join(".minecraft")
        } else {
            root.join("minecraft")
        }
    } else {
        root.clone()
    };
    Ok(Instance {
        kind: if prism { Kind::Prism } else { Kind::Server },
        game,
        root,
        version: String::new(),
        minecraft: String::new(),
        scripts: vec![],
    })
}
fn recorded_external(root: &Path) -> Result<Vec<PathBuf>> {
    let mut external = vec![];
    for dir in transaction::backups(root)? {
        if dir.join("ready").exists() && !dir.join("rolled-back").exists() {
            let journal = super::json(&dir.join("journal.json"))?;
            if let Some(entries) = journal["entries"].as_array() {
                for entry in entries {
                    if let Some(path) = entry["path"].as_str() {
                        let path = PathBuf::from(path);
                        if !path.starts_with(root) {
                            external.push(path);
                        }
                    }
                }
            }
        }
    }
    Ok(external)
}
pub fn maintain(command: &str, root: &Path, name: Option<&str>) -> Result<Value> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    // Also lock explicitly recorded external pack files during restoration.
    let _loader = transaction::lock(&root)?;
    let mut external = recorded_external(&root)?;
    external.push(root.clone());
    let state = crate::operation::durable::user_state().map_err(|e| e.to_string())?;
    let _pack = crate::operation::lock::WriteLocks::acquire(&state, &external)
        .map_err(|e| e.to_string())?;
    super::stopped(&maintenance_instance(&root)?)?;
    match command {
        "recover" => Ok(json!({"recovered":transaction::recover(&root)?})),
        "rollback" => Ok(json!({"rolledBack":transaction::rollback(&root)?})),
        "backups" => Ok(json!({"backups":transaction::backups(&root)?})),
        "clean" => {
            transaction::clean(
                &root,
                name.ok_or("clean requires an exact old_ backup name")?,
            )?;
            Ok(json!({"cleaned":name}))
        }
        _ => Err(format!("unknown maintenance action: {command}")),
    }
}
