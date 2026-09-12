use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

struct TemporaryRoot(PathBuf);

impl TemporaryRoot {
    fn new(name: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let counter = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "bkmpw-machine-{name}-{}-{unique}-{counter}",
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

fn archive_entries(path: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut entries = std::collections::BTreeMap::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).unwrap();
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        assert!(
            entries.insert(name.clone(), bytes).is_none(),
            "duplicate entry: {name}"
        );
    }
    entries
}

fn release_fixture() -> TemporaryRoot {
    let root = TemporaryRoot::new("release with spaces");
    for dir in [".pw", "pack/PCL", "mods", "roots/server"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    for (file, text) in [
        (
            ".pw/config.toml",
            "[release]\nenabled = true\ntemplate-dir = \"pack\"\ntemplate-files = \"pack.toml,start.sh,PCL/Setup.ini\"\n",
        ),
        (
            "pack/pack.toml",
            "name = \"Release\"\nversion = \"2\"\n[versions]\nminecraft = \"1.21.1\"\nneoforge = \"21.1.242\"\n",
        ),
        ("pack/start.sh", "release-start-marker"),
        ("pack/PCL/Setup.ini", "release-launcher-marker"),
        ("pack.toml", "name = \"development\"\n"),
        ("index.toml", "development-index"),
        ("start.sh", "development-start-marker"),
        (".gitignore", "/start.sh\n/PCL/\nmods/*.jar\n/private.txt\n"),
        (".packwizignore", "pack/\n**/.gitignore\n*.zip\n"),
        ("private.txt", "private-marker"),
        (
            "mods/local.pw.toml",
            "name = \"Local\"\nfilename = \"local.jar\"\n[download]\nurl = \"https://example.invalid/local.jar\"\n",
        ),
        ("mods/local.jar", "managed-runtime-marker"),
        ("roots/server/server.txt", "server-overlay-marker"),
    ] {
        fs::write(root.path().join(file), text).unwrap();
    }
    root
}

#[test]
fn four_release_exports_share_templates_without_mutating_workspace() {
    let root = release_fixture();
    for command in [
        "export-client",
        "export-server",
        "export-curseforge",
        "export-server-installer",
    ] {
        if command == "export-server-installer" {
            fs::remove_file(root.path().join("mods/local.jar")).unwrap();
        }
        for json in [false, true] {
            let zip = root.path().join("result.zip");
            let mut process = bkmpw();
            process.arg(command).arg(root.path()).arg(&zip);
            if command == "export-client" {
                process.arg("Release");
            }
            if json {
                process.arg("--json");
            }
            let output = process.output().unwrap();
            assert!(
                output.status.success(),
                "{command}: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if json {
                let value: Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(value["data"]["releaseStaged"], true);
                assert!(value["data"]["refresh"]["indexPath"].is_null());
                assert_eq!(value["data"]["refresh"]["temporary"], true);
            }
            let entries = archive_entries(&zip);
            let prefix = match command {
                "export-client" => "Release/",
                "export-curseforge" => "overrides/",
                _ => "",
            };
            for (file, marker) in [
                ("start.sh", "release-start-marker"),
                ("PCL/Setup.ini", "release-launcher-marker"),
            ] {
                assert_eq!(
                    entries.get(&format!("{prefix}{file}")).map(Vec::as_slice),
                    Some(marker.as_bytes())
                );
            }
            for excluded in ["private.txt", "pack/start.sh", "result.zip"] {
                assert!(!entries.contains_key(&format!("{prefix}{excluded}")));
            }
            let mut expected = [
                format!("{prefix}start.sh"),
                format!("{prefix}PCL/Setup.ini"),
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
            if command != "export-server-installer" {
                expected.insert(format!("{prefix}mods/local.jar"));
            }
            let extras = match command {
                "export-server" => "server.txt",
                "export-curseforge" => {
                    "manifest.json overrides/.packwizignore overrides/roots/server/server.txt"
                }
                "export-server-installer" => {
                    ".packwizignore .pw/config.toml index.toml mods/local.pw.toml pack.toml server.txt install-server.bat install-server.sh"
                }
                _ => "",
            };
            expected.extend(extras.split_whitespace().map(str::to_owned));
            assert_eq!(
                entries
                    .keys()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>(),
                expected
            );
            let runtime = entries
                .get(&format!("{prefix}mods/local.jar"))
                .map(Vec::as_slice);
            assert_eq!(
                runtime,
                (command != "export-server-installer")
                    .then_some(b"managed-runtime-marker".as_slice())
            );
            if command == "export-server" || command == "export-server-installer" {
                assert_eq!(
                    entries.get("server.txt").map(Vec::as_slice),
                    Some(b"server-overlay-marker".as_slice())
                );
            }
            assert_eq!(
                fs::read_to_string(root.path().join("index.toml")).unwrap(),
                "development-index"
            );
            assert_eq!(
                fs::read_to_string(root.path().join("pack.toml")).unwrap(),
                "name = \"development\"\n"
            );
            assert_eq!(
                fs::read_to_string(root.path().join("start.sh")).unwrap(),
                "development-start-marker"
            );
        }
    }
}

#[test]
fn release_runtime_files_follow_expanded_metadata() {
    let root = release_fixture();
    let config_path = root.path().join(".pw/config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(
        &config_path,
        config.replace("PCL/Setup.ini", "PCL/Setup.ini,mods/local.pw.toml"),
    )
    .unwrap();
    fs::create_dir_all(root.path().join("pack/mods")).unwrap();
    let metadata = fs::read_to_string(root.path().join("mods/local.pw.toml")).unwrap();
    fs::write(
        root.path().join("pack/mods/local.pw.toml"),
        metadata.replace("local.jar", "release.jar"),
    )
    .unwrap();
    fs::write(
        root.path().join("mods/release.jar"),
        "templated-runtime-marker",
    )
    .unwrap();
    // The old metadata's runtime is irrelevant once its template replaces it.
    fs::remove_file(root.path().join("mods/local.jar")).unwrap();
    for command in [
        "export-client",
        "export-server",
        "export-curseforge",
        "export-server-installer",
    ] {
        if command == "export-server-installer" {
            fs::remove_file(root.path().join("mods/release.jar")).unwrap();
        }
        let zip = root.path().join("result.zip");
        let output = bkmpw()
            .arg(command)
            .arg(root.path())
            .arg(&zip)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let entries = archive_entries(&zip);
        let runtime_path = match command {
            "export-client" => "Release/mods/release.jar",
            "export-curseforge" => "overrides/mods/release.jar",
            _ => "mods/release.jar",
        };
        assert_eq!(
            entries.get(runtime_path).map(Vec::as_slice),
            (command != "export-server-installer")
                .then_some(b"templated-runtime-marker".as_slice())
        );
        assert_eq!(
            fs::read_to_string(root.path().join("mods/local.pw.toml")).unwrap(),
            metadata
        );
    }
}

#[test]
fn release_preflight_preserves_existing_output_and_prepare_pack_expands_templates() {
    let root = release_fixture();
    let zip = root.path().join("result.zip");
    fs::write(&zip, "previous-artifact").unwrap();
    fs::remove_file(root.path().join("pack/start.sh")).unwrap();
    let failed = bkmpw()
        .arg("export-server")
        .arg(root.path())
        .arg(&zip)
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert_eq!(fs::read_to_string(zip).unwrap(), "previous-artifact");
    fs::write(root.path().join("pack/start.sh"), "new-start").unwrap();
    let prepared = bkmpw()
        .arg("prepare-pack")
        .arg(root.path())
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stdout)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("start.sh")).unwrap(),
        "new-start"
    );
    assert!(
        !fs::read_to_string(root.path().join("index.toml"))
            .unwrap()
            .contains("development-index")
    );
}

#[test]
fn release_rejects_invalid_metadata_and_unsafe_template_paths() {
    let root = release_fixture();
    fs::write(
        root.path().join("mods/duplicate.pw.toml"),
        "filename = \"local.jar\"\n",
    )
    .unwrap();
    let failed = bkmpw()
        .arg("export-server")
        .arg(root.path())
        .arg("--json")
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stdout).contains("duplicate target"));
    fs::remove_file(root.path().join("mods/duplicate.pw.toml")).unwrap();
    let outside = root.path().with_extension("outside");
    let outside_name = outside.file_name().unwrap().to_str().unwrap();
    fs::write(root.path().join(outside_name), "valid-source").unwrap();
    fs::write(&outside, "outside-original").unwrap();
    for file in [
        format!("../{outside_name}"),
        outside.to_string_lossy().replace('\\', "/"),
    ] {
        fs::write(
            root.path().join(".pw/config.toml"),
            format!(
                "[release]\nenabled = true\ntemplate-dir = \"pack\"\ntemplate-files = \"{file}\"\n"
            ),
        )
        .unwrap();
        let failed = bkmpw()
            .arg("prepare-pack")
            .arg(root.path())
            .output()
            .unwrap();
        assert!(!failed.status.success());
        assert!(String::from_utf8_lossy(&failed.stderr).contains("unsafe relative path"));
        assert_eq!(fs::read_to_string(&outside).unwrap(), "outside-original");
    }
    fs::remove_file(outside).unwrap();
}

#[test]
#[cfg(unix)]
fn prepare_pack_rejects_symlink_destination_without_touching_target() {
    let root = release_fixture();
    fs::remove_file(root.path().join("start.sh")).unwrap();
    std::os::unix::fs::symlink("private.txt", root.path().join("start.sh")).unwrap();
    let failed = bkmpw()
        .arg("prepare-pack")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert_eq!(
        fs::read_to_string(root.path().join("private.txt")).unwrap(),
        "private-marker"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("pack.toml")).unwrap(),
        "name = \"development\"\n"
    );
}

#[test]
fn prepare_pack_requires_root_in_text_and_machine_modes() {
    for format in [None, Some("--json"), Some("--json-lines")] {
        let mut command = bkmpw();
        command.arg("prepare-pack");
        if let Some(format) = format {
            command.arg(format);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        assert!(
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
            .contains("usage: bkmpw prepare-pack <pack-root>")
        );
    }
}

#[test]
#[cfg(unix)]
fn release_accepts_system_temp_directory_reached_through_a_symlink() {
    let root = release_fixture();
    let temp = TemporaryRoot::new("linked-system-temp");
    fs::create_dir_all(temp.path().join("real")).unwrap();
    let link = temp.path().join("link");
    std::os::unix::fs::symlink(temp.path().join("real"), &link).unwrap();
    let output = bkmpw()
        .env("TMPDIR", &link)
        .arg("export-client")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_dir(temp.path().join("real")).unwrap().count(), 0);
    let parent_link = temp.path().join("workspace-parent");
    std::os::unix::fs::symlink(root.path().parent().unwrap(), &parent_link).unwrap();
    let output = bkmpw()
        .arg("export-client")
        .arg(parent_link.join(root.path().file_name().unwrap()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn prepare_pack_restores_files_when_refresh_fails() {
    let root = release_fixture();
    fs::write(
        root.path().join("mods/local.pw.toml"),
        "filename = \"local.jar\"\nside = \"invalid\"\n",
    )
    .unwrap();
    let failed = bkmpw()
        .arg("prepare-pack")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert_eq!(
        fs::read_to_string(root.path().join("start.sh")).unwrap(),
        "development-start-marker"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("pack.toml")).unwrap(),
        "name = \"development\"\n"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("index.toml")).unwrap(),
        "development-index"
    );
    assert!(!root.path().join("PCL/Setup.ini").exists());
    assert!(!root.path().join("PCL").exists());
    fs::remove_file(root.path().join("start.sh")).unwrap();
    fs::create_dir(root.path().join("start.sh")).unwrap();
    let failed = bkmpw()
        .arg("prepare-pack")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert_eq!(
        fs::read_to_string(root.path().join("pack.toml")).unwrap(),
        "name = \"development\"\n"
    );
}

#[test]
fn release_rejects_overlapping_templates_and_output() {
    let root = release_fixture();
    for output in ["start.sh", "pack/start.sh"] {
        let failed = bkmpw()
            .arg("export-client")
            .arg(root.path())
            .arg(root.path().join(output))
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&failed.stderr).contains("output path collides"));
    }
    for dir in [".pw", "mods", "roots", "MoDs"] {
        fs::write(root.path().join(".pw/config.toml"), format!("[release]\nenabled = true\ntemplate-dir = \"{dir}\"\ntemplate-files = \"pack.toml\"\n")).unwrap();
        let failed = bkmpw()
            .arg("export-client")
            .arg(root.path())
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&failed.stderr).contains("overlaps protected pack path"));
    }
    for file in [".PW/config.toml", ".GitIgnore", "TaRgEt/file"] {
        fs::write(
            root.path().join(".pw/config.toml"),
            format!(
                "[release]\nenabled = true\ntemplate-dir = \"pack\"\ntemplate-files = \"{file}\"\n"
            ),
        )
        .unwrap();
        let failed = bkmpw()
            .arg("prepare-pack")
            .arg(root.path())
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&failed.stderr)
                .contains("reserved release template destination")
        );
    }
    fs::write(root.path().join(".pw/config.toml"), "[layout]\nmetadata-roots = \"metadata\"\nserver-meta = \"sides/server\"\n[release]\nenabled = true\ntemplate-dir = \"sides\"\ntemplate-files = \"pack.toml\"\n").unwrap();
    let failed = bkmpw()
        .arg("export-client")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&failed.stderr).contains("overlaps protected pack path"));
}

#[test]
fn release_checks_template_directory_case_aliases() {
    let root = release_fixture();
    let upper = root.path().join("Pack");
    let dir = if upper.exists() { "PACK" } else { "pack" };
    fs::create_dir_all(&upper).unwrap();
    fs::write(upper.join("private.txt"), "must not publish").unwrap();
    fs::write(root.path().join(".packwizignore"), "*.zip\n").unwrap();
    fs::write(root.path().join(".pw/config.toml"), format!("[release]\nenabled = true\ntemplate-dir = \"{dir}\"\ntemplate-files = \"pack.toml,start.sh,PCL/Setup.ini\"\n")).unwrap();
    let failed = bkmpw()
        .arg("export-client")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&failed.stderr)
            .contains("template directory overlaps publish input")
    );
}

#[test]
fn explicit_runtime_templates_override_workspace_files() {
    let root = release_fixture();
    let config_path = root.path().join(".pw/config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(
        &config_path,
        config.replace("PCL/Setup.ini", "PCL/Setup.ini,mods/local.jar"),
    )
    .unwrap();
    fs::create_dir_all(root.path().join("pack/mods")).unwrap();
    fs::write(
        root.path().join("pack/mods/local.jar"),
        "explicit-runtime-marker",
    )
    .unwrap();
    for command in ["export-client", "export-server", "export-curseforge"] {
        let zip = root.path().join("result.zip");
        let output = bkmpw()
            .arg(command)
            .arg(root.path())
            .arg(&zip)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let entries = archive_entries(&zip);
        let runtime_path = match command {
            "export-client" => "Release/mods/local.jar",
            "export-curseforge" => "overrides/mods/local.jar",
            _ => "mods/local.jar",
        };
        assert_eq!(
            entries.get(runtime_path).map(Vec::as_slice),
            Some(b"explicit-runtime-marker".as_slice())
        );
        assert_eq!(
            fs::read_to_string(root.path().join("mods/local.jar")).unwrap(),
            "managed-runtime-marker"
        );
    }
    for wildcard in ["literal?.json", "literal*.json"] {
        fs::write(&config_path, config.replace("PCL/Setup.ini", wildcard)).unwrap();
        let output = bkmpw()
            .arg("prepare-pack")
            .arg(root.path())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("contains wildcard"));
    }
}

#[test]
fn prepare_pack_replaces_hardlinks_without_modifying_outside_files() {
    for fail in [false, true] {
        let root = release_fixture();
        let outside = TemporaryRoot::new("outside-hardlink");
        fs::create_dir_all(outside.path()).unwrap();
        let private = outside.path().join("private.txt");
        fs::write(&private, "outside-original").unwrap();
        let target = root.path().join("start.sh");
        fs::remove_file(&target).unwrap();
        fs::hard_link(&private, &target).unwrap();
        if fail {
            fs::write(
                root.path().join("mods/local.pw.toml"),
                "filename = \"local.jar\"\nside = \"invalid\"\n",
            )
            .unwrap();
        }
        let output = bkmpw()
            .arg("prepare-pack")
            .arg(root.path())
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            !fail,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read_to_string(&private).unwrap(), "outside-original");
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            if fail {
                "outside-original"
            } else {
                "release-start-marker"
            }
        );
    }
}

#[test]
fn disabled_release_preserves_json_lines_refresh_event() {
    let root = release_fixture();
    fs::write(
        root.path().join(".pw/config.toml"),
        "[release]\nenabled = false\n",
    )
    .unwrap();
    fs::copy(
        root.path().join("pack/pack.toml"),
        root.path().join("pack.toml"),
    )
    .unwrap();
    for command in [
        "export-client",
        "export-server",
        "export-curseforge",
        "export-server-installer",
    ] {
        let output = bkmpw()
            .arg(command)
            .arg(root.path())
            .arg(root.path().join("result.zip"))
            .arg("--json-lines")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let events: Vec<Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events[1]["event"], "progress");
        assert_eq!(events[1]["phase"], "refreshing");
        assert_eq!(events[1]["message"], "rebuilding pack index");
        let data = &events.last().unwrap()["data"];
        assert_eq!(data["releaseStaged"], false);
        assert_eq!(data["refresh"]["temporary"], false);
        assert_eq!(
            fs::canonicalize(data["refresh"]["indexPath"].as_str().unwrap()).unwrap(),
            fs::canonicalize(root.path().join("index.toml")).unwrap()
        );
    }
}

#[test]
fn release_rejects_case_aliased_runtime_templates() {
    let root = release_fixture();
    let config_path = root.path().join(".pw/config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(
        &config_path,
        config.replace("PCL/Setup.ini", "PCL/Setup.ini,mods/LOCAL.jar"),
    )
    .unwrap();
    fs::create_dir_all(root.path().join("pack/mods")).unwrap();
    fs::write(root.path().join("pack/mods/LOCAL.jar"), "template-runtime").unwrap();
    for present in [true, false] {
        if !present {
            fs::remove_file(root.path().join("mods/local.jar")).unwrap();
        }
        for command in [
            "export-client",
            "export-server",
            "export-curseforge",
            "export-server-installer",
        ] {
            let zip = root.path().join("result.zip");
            fs::write(&zip, "previous-artifact").unwrap();
            let output = bkmpw()
                .arg(command)
                .arg(root.path())
                .arg(&zip)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("case differs from managed runtime target")
            );
            assert_eq!(fs::read_to_string(&zip).unwrap(), "previous-artifact");
        }
    }
    fs::write(
        &config_path,
        config.replace(
            "PCL/Setup.ini",
            "PCL/Setup.ini,mods/LOCAL.jar,mods/local.jar",
        ),
    )
    .unwrap();
    let output = bkmpw()
        .arg("prepare-pack")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("duplicate release template destination")
    );
}

#[test]
fn release_rejects_output_shadowing_root_overlay_target() {
    let root = release_fixture();
    fs::write(
        root.path().join("roots/server/server.properties"),
        "release-properties",
    )
    .unwrap();
    for name in ["server.properties", "SERVER.PROPERTIES"] {
        let output_path = root.path().join(name);
        fs::write(&output_path, "previous-artifact").unwrap();
        for command in [
            "export-client",
            "export-server",
            "export-curseforge",
            "export-server-installer",
        ] {
            let output = bkmpw()
                .arg(command)
                .arg(root.path())
                .arg(&output_path)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("collides with release overlay target")
            );
            assert_eq!(
                fs::read_to_string(&output_path).unwrap(),
                "previous-artifact"
            );
        }
    }
}

#[test]
fn release_rejects_publish_input_alias_of_exact_runtime_template() {
    let root = release_fixture();
    let config_path = root.path().join(".pw/config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(
        &config_path,
        config.replace("PCL/Setup.ini", "PCL/Setup.ini,mods/Local.jar"),
    )
    .unwrap();
    fs::create_dir_all(root.path().join("pack/mods")).unwrap();
    fs::write(root.path().join("pack/mods/Local.jar"), "template-runtime").unwrap();
    let metadata_path = root.path().join("mods/local.pw.toml");
    let metadata = fs::read_to_string(&metadata_path).unwrap();
    fs::write(&metadata_path, metadata.replace("local.jar", "Local.jar")).unwrap();
    let ignore_path = root.path().join(".packwizignore");
    let ignores = fs::read_to_string(&ignore_path).unwrap();
    fs::write(&ignore_path, format!("{ignores}\n!/mods/local.jar\n")).unwrap();
    for command in [
        "export-client",
        "export-server",
        "export-curseforge",
        "export-server-installer",
    ] {
        let zip = root.path().join("result.zip");
        fs::write(&zip, "previous-artifact").unwrap();
        let output = bkmpw()
            .arg(command)
            .arg(root.path())
            .arg(&zip)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("template case differs from publish input")
        );
        assert_eq!(fs::read_to_string(&zip).unwrap(), "previous-artifact");
        assert_eq!(
            fs::read_to_string(root.path().join("mods/local.jar")).unwrap(),
            "managed-runtime-marker"
        );
    }
}
