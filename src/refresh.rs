use std::fs;
use std::path::{Path, PathBuf};

use crate::config::ProjectConfig;
use crate::layout::PackLayout;
use crate::metadata::ModMetadata;
use crate::operation::{Error, ErrorCode};
use crate::pathutil::join_slash;
use crate::scan::{ScanReport, is_metadata_file};
use crate::sha256::sha256_file_hex_operation;

#[derive(Debug, Clone)]
pub struct RefreshResult {
    pub index_path: PathBuf,
    pub files_written: usize,
    pub metadata_written: usize,
    pub jar_written: usize,
}

pub fn refresh(
    root: &Path,
    config: &ProjectConfig,
    layout: &PackLayout,
) -> Result<RefreshResult, String> {
    refresh_operation(root, config, layout).map_err(|error| error.detail)
}

pub fn refresh_operation(
    root: &Path,
    config: &ProjectConfig,
    layout: &PackLayout,
) -> Result<RefreshResult, Error> {
    let report = ScanReport::build_operation(root, config, layout)?;
    let included: Vec<_> = report
        .included
        .into_iter()
        .filter(|rel| !should_skip_index_entry(rel))
        .collect();
    // Hashing is disk/CPU work; avoid spawning dozens of readers on one drive.
    let jobs = config.install.jobs.min(8);
    let mut entries = crate::operation::parallel::map(
        &included,
        jobs,
        &crate::operation::Control::default(),
        "hashing",
        |rel| {
            let file_path = join_slash(root, &rel);
            let hash = sha256_file_hex_operation(&file_path)?;
            let metafile = is_metadata_file(&rel, layout);
            if metafile {
                let metadata = ModMetadata::load_operation(&file_path)?;
                if let Some(crate::metadata::Side::Unknown(side)) = metadata.side {
                    return Err(Error::named(
                        ErrorCode::Failed,
                        "metadata_unknown_side",
                        format!("metadata has unsupported side: {rel}: {side}"),
                    )
                    .context(format!("{rel}: {side}")));
                }
            }
            Ok(IndexEntry {
                path: rel.clone(),
                hash,
                metafile,
            })
        },
    )?;
    let metadata_written = entries.iter().filter(|entry| entry.metafile).count();
    let jar_written = entries
        .iter()
        .filter(|entry| {
            !entry.metafile
                && entry.path.to_ascii_lowercase().ends_with(".jar")
                && crate::pathutil::is_under_slash(&entry.path, &layout.jar_root.to_string_lossy())
        })
        .count();

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let index_path = root.join("index.toml");
    crate::pathutil::write_atomic_operation(&index_path, write_index(&entries))?;
    update_pack_index_hash(root, &index_path)?;

    Ok(RefreshResult {
        index_path,
        files_written: entries.len(),
        metadata_written,
        jar_written,
    })
}

#[derive(Debug, Clone)]
struct IndexEntry {
    path: String,
    hash: String,
    metafile: bool,
}

fn should_skip_index_entry(rel: &str) -> bool {
    rel == "pack.toml" || rel == "index.toml" || rel == "packwiz.json" || rel.starts_with(".pw/")
}

fn write_index(entries: &[IndexEntry]) -> String {
    let mut out = String::from("hash-format = \"sha256\"\n\n");
    for (idx, entry) in entries.iter().enumerate() {
        if idx > 0 {
            out.push_str("\n[[files]]\n");
        } else {
            out.push_str("[[files]]\n");
        }
        out.push_str("file = \"");
        out.push_str(&escape_toml_string(&entry.path));
        out.push_str("\"\n");
        out.push_str("hash = \"");
        out.push_str(&entry.hash);
        out.push_str("\"\n");
        if entry.metafile {
            out.push_str("metafile = true\n");
        }
    }
    out
}

fn escape_toml_string(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

fn update_pack_index_hash(root: &Path, index_path: &Path) -> Result<(), Error> {
    let pack_path = root.join("pack.toml");
    if !pack_path.exists() {
        return Ok(());
    }

    let index_hash = sha256_file_hex_operation(index_path)?;
    let text = fs::read_to_string(&pack_path).map_err(|err| {
        Error::named(
            ErrorCode::Failed,
            "read_file_failed",
            format!("failed to read {}: {err}", pack_path.display()),
        )
        .context(format!("{}: {err}", pack_path.display()))
    })?;
    let updated = set_index_hash(&text, &index_hash);
    crate::pathutil::write_atomic_operation(&pack_path, updated)?;
    Ok(())
}

fn set_index_hash(text: &str, hash: &str) -> String {
    let mut out = String::new();
    let mut in_index = false;
    let mut saw_index = false;
    let mut file_replaced = false;
    let mut replaced = false;
    let mut format_replaced = false;
    let mut inserted = false;

    for raw in text.lines() {
        let trimmed = crate::pathutil::strip_comment(raw).trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_index && !file_replaced {
                out.push_str("file = \"index.toml\"\n");
                file_replaced = true;
            }
            if in_index && !format_replaced {
                out.push_str("hash-format = \"sha256\"\n");
                format_replaced = true;
            }
            if in_index && !replaced && !inserted {
                out.push_str("hash = \"");
                out.push_str(hash);
                out.push_str("\"\n");
                inserted = true;
            }
            in_index = section_name(trimmed) == Some("index");
            saw_index |= in_index;
        }

        if in_index && trimmed.starts_with("file") {
            let after = &trimmed["file".len()..];
            if after.trim_start().starts_with('=') {
                out.push_str("file = \"index.toml\"\n");
                file_replaced = true;
                continue;
            }
        }

        if in_index && trimmed.starts_with("hash-format") {
            let after = &trimmed["hash-format".len()..];
            if after.trim_start().starts_with('=') {
                out.push_str("hash-format = \"sha256\"\n");
                format_replaced = true;
                continue;
            }
        }

        if in_index && trimmed.starts_with("hash") {
            let after = &trimmed["hash".len()..];
            if after.trim_start().starts_with('=') {
                out.push_str("hash = \"");
                out.push_str(hash);
                out.push_str("\"\n");
                replaced = true;
                continue;
            }
        }

        out.push_str(raw);
        out.push('\n');
    }

    if in_index && !file_replaced {
        out.push_str("file = \"index.toml\"\n");
    }

    if in_index && !format_replaced {
        out.push_str("hash-format = \"sha256\"\n");
    }

    if in_index && !replaced && !inserted {
        out.push_str("hash = \"");
        out.push_str(hash);
        out.push_str("\"\n");
    }

    if !saw_index {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("\n[index]\nfile = \"index.toml\"\nhash-format = \"sha256\"\nhash = \"");
        out.push_str(hash);
        out.push_str("\"\n");
    }

    out
}

fn section_name(line: &str) -> Option<&str> {
    line.strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_refresh_matches_serial_output_and_failures_do_not_publish() {
        let root = std::env::temp_dir().join(crate::operation::durable::unique_id());
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("pack.toml"), "name = \"Test\"\n").unwrap();
        for i in 0..32 {
            fs::write(root.join(format!("mods/{i}.jar")), format!("jar {i}")).unwrap();
            fs::write(
                root.join(format!("mods/{i}.pw.toml")),
                format!("name = \"Mod {i}\"\nfilename = \"{i}.jar\"\n"),
            )
            .unwrap();
        }
        let mut config = ProjectConfig::load_operation(&root).unwrap();
        let layout = PackLayout::from_config(&config);
        config.install.jobs = 1;
        let serial = refresh_operation(&root, &config, &layout).unwrap();
        let index = fs::read(root.join("index.toml")).unwrap();
        let pack = fs::read(root.join("pack.toml")).unwrap();
        config.install.jobs = 32;
        let parallel = refresh_operation(&root, &config, &layout).unwrap();
        assert_eq!(serial.files_written, parallel.files_written);
        assert_eq!(parallel.metadata_written, 32);
        assert_eq!(parallel.jar_written, 32);
        assert_eq!(fs::read(root.join("index.toml")).unwrap(), index);
        assert_eq!(fs::read(root.join("pack.toml")).unwrap(), pack);
        fs::write(root.join("mods/invalid.pw.toml"), "side = \"invalid\"\n").unwrap();
        assert!(refresh_operation(&root, &config, &layout).is_err());
        assert_eq!(fs::read(root.join("index.toml")).unwrap(), index);
        assert_eq!(fs::read(root.join("pack.toml")).unwrap(), pack);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn updates_pack_index_hash() {
        let updated = set_index_hash(
            "name = \"x\"\n\n[index]\nfile = \"index.toml\"\nhash-format = \"sha1\"\nhash = \"old\"\n\n[versions]\n",
            "new",
        );

        assert!(updated.contains("hash = \"new\""));
        assert!(updated.contains("file = \"index.toml\""));
        assert!(updated.contains("hash-format = \"sha256\""));
        assert!(!updated.contains("hash-format = \"sha1\""));
        assert!(!updated.contains("hash = \"old\""));
    }

    #[test]
    fn inserts_pack_index_hash() {
        let updated = set_index_hash(
            "name = \"x\"\n\n[index]\nhash-format = \"sha256\"\n\n[versions]\n",
            "new",
        );

        assert!(updated.contains("hash = \"new\""));
        assert!(updated.contains("file = \"index.toml\""));
        assert!(updated.contains("hash-format = \"sha256\""));
        assert!(updated.contains("[versions]"));
    }

    #[test]
    fn updates_pack_index_with_commented_section_header() {
        let updated = set_index_hash(
            "name = \"x\"\n\n[index] # generated\nfile = \"index.toml\"\nhash-format = \"sha256\"\nhash = \"old\"\n",
            "new",
        );

        assert!(updated.contains("hash = \"new\""));
        assert_eq!(updated.matches("[index]").count(), 1);
    }

    #[test]
    fn updates_pack_index_with_spaced_section_header() {
        let updated = set_index_hash(
            "name = \"x\"\n\n[ index ]\nfile = \"index.toml\"\nhash-format = \"sha256\"\nhash = \"old\"\n",
            "new",
        );

        assert!(updated.contains("hash = \"new\""));
        assert!(!updated.contains("\n[index]\n"));
    }

    #[test]
    fn appends_missing_pack_index_section() {
        let updated = set_index_hash("name = \"x\"\n", "new");

        assert!(updated.contains("[index]"));
        assert!(updated.contains("file = \"index.toml\""));
        assert!(updated.contains("hash = \"new\""));
    }

    #[test]
    fn empty_index_has_no_empty_file_table() {
        assert_eq!(write_index(&[]), "hash-format = \"sha256\"\n\n");
    }

    #[test]
    fn skips_internal_state_files() {
        assert!(should_skip_index_entry("packwiz.json"));
        assert!(should_skip_index_entry(".pw/config.toml"));
        assert!(should_skip_index_entry(".pw/state.json"));
    }
}
