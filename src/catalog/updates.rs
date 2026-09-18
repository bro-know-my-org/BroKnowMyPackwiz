use super::curseforge::{Client, File, Filter};
use crate::{
    config::ProjectConfig,
    github::GitHubFileInfo,
    layout::PackLayout,
    metadata::{ModMetadata, Side},
    operation::{Control, Error, ErrorCode, Event, Result, preview::Guard},
    scan::ScanReport,
};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
};

#[derive(Clone)]
pub enum Version {
    CurseForge(File),
    GitHub(GitHubFileInfo),
}
#[derive(Clone)]
pub struct Candidate {
    pub relative: String,
    pub name: String,
    pub side: Side,
    pub before: String,
    pub after: String,
    pub version: Version,
}
#[derive(Clone)]
pub struct Preview {
    pub guard: Guard,
    pub candidates: Vec<Candidate>,
    pub skipped: Vec<(String, &'static str)>,
}
pub trait Provider: Sync {
    fn curseforge(&self, metadata: &ModMetadata, filter: &Filter) -> Result<File>;
    fn github(&self, metadata: &ModMetadata) -> Result<GitHubFileInfo>;
}
pub struct Online<'a> {
    pub root: &'a Path,
}
impl Provider for Online<'_> {
    fn curseforge(&self, metadata: &ModMetadata, filter: &Filter) -> Result<File> {
        let id = metadata
            .curseforge_project_id
            .ok_or_else(|| invalid("missing_curseforge_project"))?;
        let client = Client::for_pack(self.root)?;
        let project = client.project(id)?;
        let mut filter = filter.clone();
        if matches!(project.class_id, 12 | 6552) {
            filter.loader = None;
        }
        client.latest_compatible(id, &filter)
    }
    fn github(&self, metadata: &ModMetadata) -> Result<GitHubFileInfo> {
        crate::github::resolve_github_release_asset_operation(
            metadata
                .github_project
                .as_deref()
                .ok_or_else(|| invalid("missing_github_project"))?,
            metadata.github_tag.as_deref(),
            metadata.github_asset.as_deref(),
            None,
            None,
        )
    }
}
/// Query only. The apply phase receives these exact candidates, never "latest".
pub fn query(
    root: &Path,
    paths: &[String],
    provider: &impl Provider,
    control: &Control,
) -> Result<Preview> {
    query_with_filter(root, paths, provider, control, Filter::default())
}

pub fn query_with_filter(
    root: &Path,
    paths: &[String],
    provider: &impl Provider,
    control: &Control,
    overrides: Filter,
) -> Result<Preview> {
    let mut guard = Guard::capture(root, control)?;
    let base = Filter::for_pack(root)?;
    let filter = Filter {
        minecraft: overrides.minecraft.or(base.minecraft),
        loader: overrides.loader.or(base.loader),
    };
    let config = ProjectConfig::load_operation(root)?;
    let layout = PackLayout::from_config(&config);
    let report = ScanReport::build_operation(root, &config, &layout)?;
    let requested: BTreeSet<_> = paths.iter().cloned().collect();
    for path in &requested {
        if !report.metadata.iter().any(|entry| &entry.path == path) {
            return Err(invalid("update_target_missing"));
        }
    }
    let mut candidates = Vec::new();
    let mut skipped = Vec::new();
    let mut pending = Vec::new();
    let entries: Vec<_> = report
        .metadata
        .into_iter()
        .filter(|entry| requested.is_empty() || requested.contains(&entry.path))
        .collect();
    let total = entries.len();
    for (index, entry) in entries.into_iter().enumerate() {
        control.progress("preview_installed", index, Some(total));
        control.check()?;
        let metadata = ModMetadata::load_operation(&root.join(&entry.path))?;
        if metadata.pin {
            skipped.push((entry.path, "update_pinned"));
            continue;
        }
        if let Some(filename) = &metadata.filename {
            guard.watch(
                root,
                &crate::install::resolve_pack_file_path_operation(&entry.path, filename, &layout)?,
            )?;
        }
        if !metadata.updates_via_curseforge() && metadata.github_project.is_none() {
            skipped.push((entry.path, "update_no_provider"));
            continue;
        }
        pending.push((entry, metadata));
    }
    control.progress("preview_installed", total, Some(total));
    control.emit(Event::Phase("querying_updates".into()));
    // Keep snapshot mutation on this thread; only independent provider I/O runs in parallel.
    let versions = parallel_query(
        &pending,
        config.install.jobs,
        control,
        |(entry, metadata)| {
            control.emit(Event::Log(entry.path.clone()));
            if metadata.updates_via_curseforge() {
                provider
                    .curseforge(metadata, &filter)
                    .map(Version::CurseForge)
            } else {
                provider.github(metadata).map(Version::GitHub)
            }
        },
    )?;
    for ((entry, metadata), version) in pending.into_iter().zip(versions) {
        let (version, filename, hash, before, after, unchanged) = match version {
            Version::CurseForge(file) => {
                if Some(file.project_id) != metadata.curseforge_project_id {
                    return Err(invalid("update_project_mismatch"));
                }
                let unchanged = metadata.curseforge_file_id == Some(file.id)
                    && metadata.download_hash_format.as_deref() == Some("sha1")
                    && metadata.filename.as_deref() == Some(&file.filename)
                    && metadata
                        .download_hash
                        .as_ref()
                        .is_some_and(|hash| hash.eq_ignore_ascii_case(&file.sha1));
                let before = format!(
                    "{} · {}",
                    metadata.filename.as_deref().unwrap_or("?"),
                    metadata
                        .curseforge_file_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "?".into())
                );
                let after = format!("{} · {}", file.filename, file.id);
                let (filename, hash) = (file.filename.clone(), file.sha1.clone());
                (
                    Version::CurseForge(file),
                    filename,
                    hash,
                    before,
                    after,
                    unchanged,
                )
            }
            Version::GitHub(file) => {
                let unchanged = metadata.filename.as_deref() == Some(&file.filename)
                    && metadata.download_hash_format.as_deref() == Some(file.hash_format.as_str())
                    && metadata.download_url.as_deref() == Some(&file.url)
                    && metadata
                        .download_hash
                        .as_ref()
                        .is_some_and(|hash| hash.eq_ignore_ascii_case(&file.hash));
                let before = format!(
                    "{} · {}",
                    metadata.filename.as_deref().unwrap_or("?"),
                    short(metadata.download_hash.as_deref().unwrap_or("?"))
                );
                let after = format!("{} · {}", file.filename, short(&file.hash));
                let (filename, hash) = (file.filename.clone(), file.hash.clone());
                (
                    Version::GitHub(file),
                    filename,
                    hash,
                    before,
                    after,
                    unchanged,
                )
            }
        };
        control.check()?;
        crate::operation::paths::filename(&filename)?;
        if hash.is_empty() {
            return Err(invalid("update_hash_missing"));
        }
        if unchanged {
            skipped.push((entry.path, "update_unchanged"));
            continue;
        }
        guard.watch(
            root,
            &crate::install::resolve_pack_file_path_operation(&entry.path, &filename, &layout)?,
        )?;
        candidates.push(Candidate {
            relative: entry.path.clone(),
            name: metadata.name.clone().unwrap_or(entry.path),
            side: crate::install::side_from_directory_or_metadata(entry.side_hint, &metadata),
            before,
            after,
            version,
        });
    }
    skipped.sort_by(|a, b| a.0.cmp(&b.0));
    guard.validate(root, control)?;
    Ok(Preview {
        guard,
        candidates,
        skipped,
    })
}
/// Bound API traffic even when a download configuration requests many workers.
fn parallel_query<T: Sync, R: Send>(
    items: &[T],
    jobs: usize,
    control: &Control,
    query: impl Fn(&T) -> Result<R> + Sync,
) -> Result<Vec<R>> {
    control.check()?;
    control.emit(Event::Progress {
        label: "querying_updates".into(),
        current: 0,
        total: Some(items.len() as u64),
    });
    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..jobs.clamp(1, 64).min(items.len()) {
            let (tx, next, stopped, query) = (tx.clone(), &next, &stopped, &query);
            scope.spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else { break };
                    let result = control.check().and_then(|()| query(item));
                    if result.is_err() {
                        stopped.store(true, Ordering::Release);
                    }
                    if tx.send((index, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let mut results = Vec::new();
        for result in rx {
            results.push(result);
            control.emit(Event::Progress {
                label: "querying_updates".into(),
                current: results.len() as u64,
                total: Some(items.len() as u64),
            });
        }
        control.check()?;
        results.sort_by_key(|(index, _)| *index);
        results.into_iter().map(|(_, result)| result).collect()
    })
}

fn short(value: &str) -> String {
    value.chars().take(12).collect()
}
fn invalid(key: &str) -> Error {
    Error::key(ErrorCode::Invalid, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::durable;
    use std::fs;
    struct Fake {
        calls: AtomicUsize,
    }
    impl Provider for Fake {
        fn curseforge(&self, _: &ModMetadata, _: &Filter) -> Result<File> {
            panic!("unexpected CF query")
        }
        fn github(&self, _: &ModMetadata) -> Result<GitHubFileInfo> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(GitHubFileInfo {
                name: "New".into(),
                filename: "new.jar".into(),
                url: "https://example.invalid/new.jar".into(),
                hash_format: "sha256".into(),
                hash: "a".repeat(64),
            })
        }
    }
    #[test]
    fn direct_download_with_curseforge_update_metadata_is_not_skipped() {
        struct Direct;
        impl Provider for Direct {
            fn curseforge(&self, metadata: &ModMetadata, _: &Filter) -> Result<File> {
                assert_eq!(metadata.curseforge_project_id, Some(385587));
                Ok(File {
                    project_id: 385587,
                    id: 20,
                    name: "New shaders".into(),
                    filename: "new.zip".into(),
                    versions: vec!["1.21.1".into()],
                    date: String::new(),
                    release_type: 1,
                    size: 1,
                    sha1: "a".repeat(40),
                    dependencies: vec![],
                })
            }
            fn github(&self, _: &ModMetadata) -> Result<GitHubFileInfo> {
                panic!("wrong provider")
            }
        }
        let root = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(root.join("shaderpacks")).unwrap();
        fs::write(root.join("pack.toml"), "name = \"Test\"\n").unwrap();
        let text = "name = \"Shaders\"\nfilename = \"old.zip\"\n[download]\nurl = \"https://example.invalid/old.zip\"\n[update.curseforge]\nproject-id = 385587\nfile-id = 10\n";
        fs::write(root.join("shaderpacks/shaders.pw.toml"), text).unwrap();
        let preview = query(&root, &[], &Direct, &Control::default()).unwrap();
        assert!(preview.skipped.is_empty());
        assert_eq!(preview.candidates.len(), 1);
        assert!(preview.candidates[0].after.contains("new.zip"));
        assert_eq!(
            fs::read_to_string(root.join("shaderpacks/shaders.pw.toml")).unwrap(),
            text
        );
        // Export-only CF hints must not turn a direct download into a CF updater.
        assert!(
            !ModMetadata::parse("[export.curseforge]\nproject-id = 1").updates_via_curseforge()
        );
        let mixed = ModMetadata::parse(
            "[update.github]\nproject = \"owner/repo\"\n[update.curseforge]\nproject-id = 1",
        );
        assert!(!mixed.updates_via_curseforge());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn queries_overlap_are_bounded_and_keep_input_order() {
        use std::sync::{Condvar, Mutex};
        use std::time::Duration;
        let gate = (Mutex::new(0), Condvar::new());
        let active = AtomicUsize::new(0);
        let (tx, rx) = mpsc::channel();
        let control = Control::with_events(tx);
        let items: Vec<_> = (0..128).collect();
        let results = parallel_query(&items, 100, &control, |&item| {
            assert!(active.fetch_add(1, Ordering::SeqCst) < 64);
            // Hold the first wave until all 64 workers are in flight. No timing speed assertion.
            if item < 64 {
                let mut entered = gate.0.lock().unwrap();
                *entered += 1;
                gate.1.notify_all();
                let (entered, timeout) = gate
                    .1
                    .wait_timeout_while(entered, Duration::from_secs(5), |count| *count < 64)
                    .unwrap();
                assert!(!timeout.timed_out() && *entered == 64);
            }
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(item)
        })
        .unwrap();
        assert_eq!(results, items);
        let counts: Vec<_> = rx
            .try_iter()
            .filter_map(|event| match event {
                Event::Progress { current, total, .. } => Some((current, total)),
                _ => None,
            })
            .collect();
        assert_eq!(
            counts,
            (0..=128).map(|n| (n, Some(128))).collect::<Vec<_>>()
        );
    }

    #[test]
    fn cancellation_and_errors_stop_new_queries() {
        let control = Control::default();
        let calls = AtomicUsize::new(0);
        let error = parallel_query(&[1, 2, 3], 1, &control, |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            control.cancel();
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Cancelled);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let error = parallel_query(&[1, 2, 3], 0, &Control::default(), |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Err::<(), _>(invalid("provider_failed"))
        })
        .unwrap_err();
        assert_eq!(error.detail, "provider_failed");
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert!(
            parallel_query::<(), ()>(&[], 16, &Control::default(), |_| unreachable!())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn querying_skips_pins_and_freezes_candidates_without_writes() {
        let root = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(root.join("mods")).unwrap();
        let root = fs::canonicalize(root).unwrap();
        fs::write(root.join("pack.toml"), "name = \"Test\"\n").unwrap();
        let text =
            "name = \"Old\"\nfilename = \"old.jar\"\n[update.github]\nproject = \"owner/repo\"\n";
        fs::write(root.join("mods/a.pw.toml"), text).unwrap();
        fs::write(root.join("mods/b.pw.toml"), format!("pin = true\n{text}")).unwrap();
        let provider = Fake {
            calls: AtomicUsize::new(0),
        };
        let preview = query(&root, &[], &provider, &Control::default()).unwrap();
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
        assert!(preview.candidates[0].after.contains("new.jar"));
        assert_eq!(preview.skipped[0].1, "update_pinned");
        assert_eq!(
            fs::read_to_string(root.join("mods/a.pw.toml")).unwrap(),
            text
        );
        assert!(!root.join("index.toml").exists());
        let changed_root = root.clone();
        let mut control = Control::default();
        control.checkpoint = Some(std::sync::Arc::new(move |event| {
            if matches!(event, Event::Progress { label, current: 1, .. } if label == "querying_updates")
            {
                fs::write(changed_root.join("pack.toml"), "name = 'Changed'\n").unwrap();
            }
        }));
        let error = query(&root, &[], &provider, &control).err().unwrap();
        assert_eq!(error.code, ErrorCode::Conflict);
        fs::remove_dir_all(root).unwrap();
    }
}
