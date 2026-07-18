use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

struct TemporaryRoot(PathBuf);

impl TemporaryRoot {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "bkmpw-machine-{name}-{}-{unique}",
            std::process::id()
        )))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TemporaryRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bkmpw() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bkmpw"))
}

#[test]
#[cfg(windows)]
fn repairs_a_quoted_pack_root_split_by_a_launcher() {
    let parent = TemporaryRoot::new("split-quoted-root");
    let root = parent.path().join("pack with spaces");
    let root = root.to_str().unwrap();
    let initialized = bkmpw().args(["init", root]).output().unwrap();
    assert!(
        initialized.status.success(),
        "{}",
        String::from_utf8_lossy(&initialized.stderr)
    );
    fs::write(
        PathBuf::from(root).join("mods/path-repair-fixture.pw.toml"),
        "[option]\noptional = true\ndefault = false\n",
    )
    .unwrap();

    let split_at = root.find(' ').unwrap();
    let first = format!("\"{}", &root[..split_at]);
    let second = format!("{}\"", &root[split_at + 1..]);

    let output = bkmpw()
        .args(["install-files-headless", &first, &second, "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.ends_with("pack with spaces"));
    assert!(PathBuf::from(root).join("pack.toml").is_file());
}

#[test]
fn protocol_version_is_discoverable_as_json() {
    let output = bkmpw()
        .args(["protocol-version", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["protocolVersion"], 1);
    assert_eq!(value["data"]["protocolVersion"], 1);
    assert_eq!(value["ok"], true);
}

#[test]
fn json_lines_are_individually_parseable_and_sequenced() {
    let root = TemporaryRoot::new("json-lines");
    let output = bkmpw()
        .args(["init", root.path().to_str().unwrap(), "--json-lines"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let events = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["event"], "started");
    assert_eq!(events[1]["event"], "progress");
    assert_eq!(events[2]["event"], "completed");
    assert_eq!(events[0]["sequence"], 1);
    assert_eq!(events[1]["sequence"], 2);
    assert_eq!(events[2]["sequence"], 3);
    assert_eq!(events[2]["ok"], true);
}

#[test]
fn failed_check_keeps_structured_data_and_exit_status() {
    let root = TemporaryRoot::new("failed-check");
    fs::create_dir_all(root.path()).unwrap();
    let output = bkmpw()
        .args(["check", root.path().to_str().unwrap(), "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["data"]["valid"], false);
    assert!(value["data"]["errors"].as_array().unwrap().len() >= 4);
}

#[test]
fn unsupported_machine_command_never_leaks_human_output() {
    let output = bkmpw()
        .args(["self-update", "--dry-run", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
}
