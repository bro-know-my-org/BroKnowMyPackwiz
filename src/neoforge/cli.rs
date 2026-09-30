use super::{
    Result,
    upgrade::{self, Options},
};
use serde_json::{Value, json};
use std::path::PathBuf;

pub const HELP: &str = "Usage: bkmpw neoforge <action> <instance> [options]
Actions: inspect, candidates, plan, upgrade, tui, recover, rollback, backups, clean
Options: --target VERSION --installer JAR --java PATH --script RELATIVE_PATH (repeatable)
         --sync-source ROOT --accept-unknown --yes
CLI upgrade always requires --yes; unknown results additionally require --accept-unknown.
Use plan first to review. CLI never prompts. TUI lets you select and confirm interactively.
clean requires an exact old_ backup directory name. Other client launchers: TODO.
Instance must be stopped; close Prism before editing its component configuration.";

pub fn run(args: &[String]) -> Result<()> {
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        println!("{HELP}");
        return Ok(());
    }
    if args[0] == "tui" {
        return super::ui::run(&args[1..]);
    }
    let result = dispatch(args)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
    );
    Ok(())
}
fn parse(args: &[String]) -> Result<(String, Options, bool, Option<String>)> {
    let action = args.first().ok_or(HELP)?.clone();
    let root = args.get(1).filter(|s| !s.starts_with('-')).ok_or(HELP)?;
    let root = PathBuf::from(root)
        .canonicalize()
        .map_err(|e| format!("instance: {e}"))?;
    let mut options = Options {
        root,
        target: String::new(),
        installer: None,
        java: "java".into(),
        scripts: vec![],
        external: None,
        accept_unknown: false,
    };
    let mut yes = false;
    let mut name = None;
    let mut i = 2;
    while i < args.len() {
        let flag = &args[i];
        if flag == "--yes" {
            yes = true;
        } else if flag == "--accept-unknown" {
            options.accept_unknown = true;
        } else if flag.starts_with("--") {
            i += 1;
            let value = args
                .get(i)
                .filter(|v| !v.starts_with("--"))
                .ok_or_else(|| format!("{flag} requires a value"))?;
            match flag.as_str() {
                "--target" => options.target = value.clone(),
                "--installer" => {
                    options.installer = Some(
                        PathBuf::from(value)
                            .canonicalize()
                            .map_err(|e| format!("installer: {e}"))?,
                    )
                }
                "--java" => {
                    options.java = if value.contains('/') || value.contains('\\') {
                        PathBuf::from(value)
                            .canonicalize()
                            .map_err(|e| format!("Java: {e}"))?
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        value.clone()
                    }
                }
                "--script" => options.scripts.push(value.clone()),
                "--sync-source" => {
                    options.external = Some(
                        PathBuf::from(value)
                            .canonicalize()
                            .map_err(|e| format!("source: {e}"))?,
                    )
                }
                _ => return Err(format!("unknown NeoForge option {flag}")),
            }
        } else if action == "clean" && name.is_none() {
            name = Some(flag.clone());
        } else {
            return Err(format!("unexpected argument {flag}"));
        }
        i += 1;
    }
    Ok((action, options, yes, name))
}
pub fn dispatch(args: &[String]) -> Result<Value> {
    let (action, options, yes, name) = parse(args)?;
    match action.as_str() {
        "inspect" => Ok(json!({"instance":super::detect(&options.root, &options.scripts)?})),
        "candidates" => {
            let instance = super::detect(&options.root, &options.scripts)?;
            let index = super::get_json(&format!("{}/index.json", super::META))?;
            Ok(
                json!({"instance":instance,"candidates":super::candidates(&index, &instance.minecraft)?}),
            )
        }
        "plan" | "upgrade" => {
            if options.target.is_empty() {
                return Err(
                    "choose --target explicitly; use candidates to list stable versions".into(),
                );
            }
            if action == "upgrade" && !yes {
                return Err("upgrade requires --yes; use plan to review first. CLI never waits for interactive input".into());
            }
            let _guards = upgrade::guards(&options.root, options.external.as_deref())?;
            let instance = super::detect(&options.root, &options.scripts)?;
            if action == "upgrade" && instance.version == options.target {
                return Ok(json!({"status":"unchanged","instance":instance}));
            }
            let prepared = upgrade::prepare(&options)?;
            if action == "plan" {
                return Ok(prepared.summary());
            }
            if let Err(error) = prepared.report.authorize(options.accept_unknown) {
                return Err(format!(
                    "{error}\n{}",
                    serde_json::to_string_pretty(&prepared.report).map_err(|e| e.to_string())?
                ));
            }
            upgrade::execute(&prepared, &options)
        }
        "recover" | "rollback" | "backups" | "clean" => {
            upgrade::maintain(&action, &options.root, name.as_deref())
        }
        "tui" => Err("TUI requires a terminal and cannot use --json/--json-lines".into()),
        _ => Err(format!("unknown NeoForge action {action}\n{HELP}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_noninteractive_confirmation_fails_before_network_or_mutation() {
        let args = ["upgrade", ".", "--target", "21.1.252"].map(str::to_owned);
        assert!(dispatch(&args).unwrap_err().contains("requires --yes"));
        assert!(parse(&["inspect", ".", "--force"].map(str::to_owned)).is_err());
    }
}
