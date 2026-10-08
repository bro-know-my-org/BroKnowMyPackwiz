use super::*;
use std::{
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "bkmpw neoforge acceptance {}-{}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("mods")).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn instance(&self, kind: Kind) -> Instance {
        Instance {
            root: self.0.clone(),
            game: self.0.clone(),
            kind,
            version: "21.1.242".into(),
            minecraft: "1.21.1".into(),
            scripts: vec!["run.sh".into()],
        }
    }
    fn jar(&self, name: &str, entries: &[(&str, &str)]) {
        let file = fs::File::create(self.0.join("mods").join(name)).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, content) in entries {
            zip.start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn target() -> Target {
    Target {
        version: "21.1.252".into(),
        minecraft: "1.21.1".into(),
        fml: "4.0.44".into(),
    }
}
fn metadata(id: &str, version: &str, loader: &str, deps: &str) -> String {
    format!(
        "modLoader=\"javafml\"\nloaderVersion=\"{loader}\"\n[[mods]]\nmodId=\"{id}\"\nversion=\"{version}\"\n{deps}"
    )
}
fn dependency(owner: &str, id: &str, range: &str, side: &str) -> String {
    format!(
        "[[dependencies.{owner}]]\nmodId=\"{id}\"\ntype=\"required\"\nversionRange=\"{range}\"\nside=\"{side}\"\n"
    )
}

#[test]
fn compressed_manual_jars_and_fml_are_checked_independently() {
    let f = Fixture::new();
    let good = metadata(
        "manual",
        "1.0",
        "[4,5)",
        &dependency("manual", "neoforge", "[21.1.242,)", "BOTH"),
    );
    f.jar("hand-added.jar", &[("META-INF/neoforge.mods.toml", &good)]);
    let report = compatibility::inspect(&f.instance(Kind::Server), &target()).unwrap();
    assert_eq!(report.checked, 1);
    assert!(report.blocked.is_empty());
    assert!(report.unknown.is_empty());
    let bad = metadata("bad", "1.0", "[21,)", "");
    f.jar("bad-fml.jar", &[("META-INF/neoforge.mods.toml", &bad)]);
    let report = compatibility::inspect(&f.instance(Kind::Server), &target()).unwrap();
    assert!(
        report
            .blocked
            .iter()
            .any(|f| f.file.ends_with("bad-fml.jar") && f.reason.contains("FML loaderVersion"))
    );
}
#[test]
fn required_mod_dependencies_versions_and_sides() {
    let f = Fixture::new();
    f.jar(
        "lib.jar",
        &[(
            "META-INF/neoforge.mods.toml",
            &metadata("lib", "1.0", "[4,)", ""),
        )],
    );
    let deps = dependency("main", "lib", "[2,)", "CLIENT")
        + &dependency("main", "minecraft", "[1.21.1]", "BOTH");
    f.jar(
        "main.jar",
        &[(
            "META-INF/neoforge.mods.toml",
            &metadata("main", "1", "[4,)", &deps),
        )],
    );
    assert!(
        compatibility::inspect(&f.instance(Kind::Server), &target())
            .unwrap()
            .blocked
            .is_empty()
    );
    assert!(
        compatibility::inspect(&f.instance(Kind::Prism), &target())
            .unwrap()
            .blocked
            .iter()
            .any(|f| f.reason.contains("lib requires"))
    );
}
#[test]
fn unsupported_metadata_and_nested_jars_are_unknown_with_file_reasons() {
    let f = Fixture::new();
    f.jar(
        "nested.jar",
        &[
            (
                "META-INF/neoforge.mods.toml",
                &metadata("nested", "${unknown}", "[4,)", ""),
            ),
            ("META-INF/jarjar/metadata.json", "{}"),
        ],
    );
    f.jar("manual-library.jar", &[("unrelated", "library")]);
    let report = compatibility::inspect(&f.instance(Kind::Server), &target()).unwrap();
    assert!(report.blocked.is_empty());
    assert!(report.unknown.len() >= 3);
    assert!(
        report
            .unknown
            .iter()
            .any(|f| f.reason.contains("nested JAR"))
    );
    assert!(
        report
            .unknown
            .iter()
            .any(|f| f.file.ends_with("manual-library.jar"))
    );
    assert!(report.authorize(false).is_err());
    assert!(report.authorize(true).is_ok());
}
#[test]
fn foreign_loader_missing_dependencies_and_target_caps_block() {
    let f = Fixture::new();
    f.jar("fabric-only.jar", &[("fabric.mod.json", "{}")]);
    let deps = dependency("limited", "neoforge", "[21.1.242,21.1.250]", "BOTH")
        + &dependency("limited", "forge", "[1,)", "BOTH");
    f.jar(
        "limited.jar",
        &[(
            "META-INF/neoforge.mods.toml",
            &metadata("limited", "1", "[4,)", &deps),
        )],
    );
    let report = compatibility::inspect(&f.instance(Kind::Server), &target()).unwrap();
    assert_eq!(report.blocked.len(), 3);
    assert!(report.authorize(true).is_err());
}
#[test]
fn manifest_placeholder_and_multiple_mods_resolve_required_versions() {
    let f = Fixture::new();
    let multi = metadata("lib", "${file.jarVersion}", "[4,)", "")
        + "\n[[mods]]\nmodId=\"second\"\nversion=\"2\"\n";
    f.jar(
        "multi.jar",
        &[
            ("META-INF/neoforge.mods.toml", &multi),
            (
                "META-INF/MANIFEST.MF",
                "Manifest-Version: 1.0\r\nImplementation-Version: 1.\r\n 5\r\n\r\n",
            ),
        ],
    );
    let deps =
        dependency("main", "lib", "[1.5]", "BOTH") + &dependency("main", "second", "[2]", "BOTH");
    f.jar(
        "main.jar",
        &[(
            "META-INF/neoforge.mods.toml",
            &metadata("main", "1", "[4,)", &deps),
        )],
    );
    let report = compatibility::inspect(&f.instance(Kind::Server), &target()).unwrap();
    assert!(report.blocked.is_empty());
    assert!(report.unknown.is_empty());
}
fn prism_fixture(f: &Fixture, game: &str) {
    fs::create_dir(f.0.join(game)).unwrap();
    fs::write(
        f.0.join("instance.cfg"),
        "JavaPath=/Java With Spaces/bin/java\nMaxMemAlloc=8192\nJvmArgs=-Dcustom=true\n",
    )
    .unwrap();
    fs::write(f.0.join("mmc-pack.json"), serde_json::to_vec(&serde_json::json!({"formatVersion":1,"components":[{"uid":"net.minecraft","version":"1.21.1","important":true},{"uid":"net.neoforged","version":"21.1.242","cachedVersion":"21.1.242","cachedRequires":[{"uid":"net.minecraft","equals":"1.21.1"}],"important":true},{"uid":"org.lwjgl3","version":"3.3.3"}]})).unwrap()).unwrap();
}
#[test]
fn prism_original_instance_and_settings_survive_switch_and_rollback() {
    for game in ["minecraft", ".minecraft"] {
        let f = Fixture::new();
        prism_fixture(&f, game);
        let instance = detect(&f.0, &[]).unwrap();
        assert_eq!(instance.game, f.0.join(game));
        let settings = read(&f.0.join("instance.cfg")).unwrap();
        let before = read(&f.0.join("mmc-pack.json")).unwrap();
        fs::write(
            instance.game.join("pack.toml"),
            "# preserve\n[versions]\nminecraft=\"1.21.1\"\nneoforge=\"21.1.242\"\n",
        )
        .unwrap();
        let stage = f.0.join("stage");
        fs::create_dir(&stage).unwrap();
        let meta = serde_json::json!({"requires":[{"uid":"net.minecraft","equals":"1.21.1"}]});
        let changes = adapter::changes(&instance, &target(), &stage, Some(&meta), None).unwrap();
        let _lock = transaction::lock(&f.0).unwrap();
        let backup = transaction::apply(&f.0, &changes, |_| Ok(())).unwrap();
        assert_eq!(detect(&f.0, &[]).unwrap().version, "21.1.252");
        assert!(
            read(&instance.game.join("pack.toml"))
                .unwrap()
                .contains("21.1.252")
        );
        assert_eq!(read(&f.0.join("instance.cfg")).unwrap(), settings);
        assert!(json(&f.0.join("mmc-pack.json")).unwrap()["components"][1]["important"] == true);
        transaction::rollback(&f.0).unwrap();
        assert_eq!(read(&f.0.join("mmc-pack.json")).unwrap(), before);
        assert!(
            read(&instance.game.join("pack.toml"))
                .unwrap()
                .contains("21.1.242")
        );
        assert!(backup.exists());
        drop(_lock);
    }
}
#[test]
fn pack_sync_missing_file_not_created_and_explicit_source_is_atomic() {
    let f = Fixture::new();
    prism_fixture(&f, "minecraft");
    let instance = detect(&f.0, &[]).unwrap();
    let stage = f.0.join("stage");
    fs::create_dir(&stage).unwrap();
    let source = Fixture::new();
    fs::write(
        source.0.join("pack.toml"),
        "[versions]\nminecraft=\"1.21.1\"\nneoforge=\"21.1.242\"\n",
    )
    .unwrap();
    let meta = serde_json::json!({"requires":[{"uid":"net.minecraft","equals":"1.21.1"}]});
    let changes =
        adapter::changes(&instance, &target(), &stage, Some(&meta), Some(&source.0)).unwrap();
    let _lock = transaction::lock(&f.0).unwrap();
    let error = transaction::apply(&f.0, &changes, |after| {
        if after {
            Err("config sync failure".into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert!(error.contains("restored"));
    assert!(!instance.game.join("pack.toml").exists());
    assert!(
        read(&source.0.join("pack.toml"))
            .unwrap()
            .contains("21.1.242")
    );
    fs::write(
        source.0.join("pack.toml"),
        "[versions]\nminecraft=\"1.21\"\n",
    )
    .unwrap();
    assert!(adapter::changes(&instance, &target(), &stage, Some(&meta), Some(&source.0)).is_err());
    drop(_lock);
}
#[test]
fn prism_custom_patch_and_conflicting_cache_are_blocked() {
    let f = Fixture::new();
    prism_fixture(&f, "minecraft");
    let mut pack = json(&f.0.join("mmc-pack.json")).unwrap();
    pack["components"][1]["cachedVersion"] = serde_json::json!("21.1.240");
    fs::write(f.0.join("mmc-pack.json"), pack.to_string()).unwrap();
    assert!(detect(&f.0, &[]).unwrap_err().contains("conflict"));
    pack["components"][1]["cachedVersion"] = serde_json::json!("21.1.242");
    fs::write(f.0.join("mmc-pack.json"), pack.to_string()).unwrap();
    fs::create_dir(f.0.join("patches")).unwrap();
    fs::write(f.0.join("patches/net.neoforged.json"), "{}").unwrap();
    assert!(detect(&f.0, &[]).unwrap_err().contains("custom"));
}
#[test]
fn pack_declaration_alone_is_not_an_installation() {
    let f = Fixture::new();
    fs::write(f.0.join("pack.toml"), "[versions]\nneoforge=\"21.1.242\"\n").unwrap();
    assert!(
        detect(&f.0, &[])
            .unwrap_err()
            .contains("not installation evidence")
    );
}
#[test]
fn world_os_lock_blocks_even_without_a_visible_java_process() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("world")).unwrap();
    let file = fs::File::create(f.0.join("world/session.lock")).unwrap();
    fs2::FileExt::lock_exclusive(&file).unwrap();
    assert!(
        stopped(&f.instance(Kind::Server))
            .unwrap_err()
            .contains("world is running")
    );
    fs2::FileExt::unlock(&file).unwrap();
}
#[test]
fn adapter_never_claims_or_deletes_unrelated_libraries() {
    let f = Fixture::new();
    let instance = f.instance(Kind::Server);
    let target = target();
    let stage = f.0.join("stage");
    let lib = stage.join("libraries/net/neoforged/neoforge/21.1.252");
    fs::create_dir_all(&lib).unwrap();
    let args = "--fml.neoForgeVersion 21.1.252 --fml.mcVersion 1.21.1 --fml.fmlVersion 4.0.44\n";
    for name in ["unix_args.txt", "win_args.txt"] {
        fs::write(lib.join(name), args).unwrap();
    }
    f.jar("artifact.jar", &[("a", "artifact")]);
    for classifier in ["server", "universal"] {
        fs::copy(
            f.0.join("mods/artifact.jar"),
            lib.join(format!("neoforge-21.1.252-{classifier}.jar")),
        )
        .unwrap();
    }
    fs::write(stage.join("libraries/shared.jar"), "target library").unwrap();
    fs::create_dir_all(f.0.join("libraries")).unwrap();
    fs::write(
        f.0.join("libraries/shared.jar"),
        "unowned different contents",
    )
    .unwrap();
    fs::write(f.0.join("libraries/manual.jar"), "unowned keep").unwrap();
    fs::write(
        f.0.join("run.sh"),
        "java -Xmx8G @libraries/net/neoforged/neoforge/21.1.242/unix_args.txt nogui\n",
    )
    .unwrap();
    assert!(
        adapter::changes(&instance, &target, &stage, None, None)
            .err()
            .unwrap()
            .contains("unowned conflicting")
    );
    fs::remove_file(f.0.join("libraries/shared.jar")).unwrap();
    let changes = adapter::changes(&instance, &target, &stage, None, None).unwrap();
    let _lock = transaction::lock(&f.0).unwrap();
    transaction::apply(&f.0, &changes, |_| Ok(())).unwrap();
    assert_eq!(
        read(&f.0.join("libraries/manual.jar")).unwrap(),
        "unowned keep"
    );
    assert!(
        json(&f.0.join(".bkmpw-neoforge/state.json")).unwrap()["managed"]
            .get("libraries/manual.jar")
            .is_none()
    );
    transaction::rollback(&f.0).unwrap();
    assert!(!f.0.join("libraries/shared.jar").exists());
    assert_eq!(
        read(&f.0.join("libraries/manual.jar")).unwrap(),
        "unowned keep"
    );
    drop(_lock);
}
#[test]
fn corrupted_backup_refuses_restore_and_temp_collisions_are_preserved() {
    let f = Fixture::new();
    let _lock = transaction::lock(&f.0).unwrap();
    fs::write(f.0.join("original"), "before").unwrap();
    fs::write(f.0.join("new"), "after").unwrap();
    let changes = [transaction::Change {
        path: f.0.join("original"),
        source: f.0.join("new"),
    }];
    fs::write(f.0.join("original.bkmpw-loader-tmp"), "manual").unwrap();
    assert!(transaction::apply(&f.0, &changes, |_| Ok(())).is_err());
    assert_eq!(
        read(&f.0.join("original.bkmpw-loader-tmp")).unwrap(),
        "manual"
    );
    fs::remove_file(f.0.join("original.bkmpw-loader-tmp")).unwrap();
    let backup = transaction::apply(&f.0, &changes, |_| Ok(())).unwrap();
    fs::write(backup.join("files/0"), "broken").unwrap();
    assert!(
        transaction::rollback(&f.0)
            .unwrap_err()
            .contains("damaged backup")
    );
    assert_eq!(read(&f.0.join("original")).unwrap(), "after");
    drop(_lock);
}
#[cfg(unix)]
#[test]
fn symlink_paths_cannot_escape_transaction_boundary() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let outside = Fixture::new();
    let _lock = transaction::lock(&f.0).unwrap();
    fs::write(f.0.join("source"), "new").unwrap();
    fs::write(outside.0.join("keep"), "keep").unwrap();
    symlink(&outside.0, f.0.join("linked")).unwrap();
    let changes = [transaction::Change {
        path: f.0.join("linked/keep"),
        source: f.0.join("source"),
    }];
    assert!(transaction::apply(&f.0, &changes, |_| Ok(())).is_err());
    assert_eq!(read(&outside.0.join("keep")).unwrap(), "keep");
    drop(_lock);
}
