use std::fs;
use std::path::Path;

use crate::config::ProjectConfig;
use crate::layout::PackLayout;
use crate::metadata::{ModMetadata, Side};
use crate::pathutil::join_slash;
use crate::scan::ScanReport;

#[derive(Debug, Clone)]
struct ModRow {
    name: String,
    filename: String,
    side: String,
    source: String,
    project_id: Option<u64>,
    file_id: Option<u64>,
}

pub fn write_modlist(root: &Path, output_dir: &Path) -> Result<(usize, String, String), String> {
    let config = ProjectConfig::load(root)?;
    let layout = PackLayout::from_config(&config);
    let report = ScanReport::build(root, &config, &layout)?;
    let mut rows = Vec::new();

    for entry in report.metadata {
        let path = join_slash(root, &entry.path);
        let metadata = ModMetadata::load(&path)?;
        let name = metadata
            .name
            .clone()
            .or_else(|| metadata.filename.clone())
            .unwrap_or_else(|| entry.path.clone());
        let filename = metadata.filename.clone().unwrap_or_default();
        let side = match entry.side_hint {
            crate::scan::SideHint::Server => Side::Server,
            crate::scan::SideHint::Client => Side::Client,
            crate::scan::SideHint::Common => Side::Both,
            crate::scan::SideHint::Unknown => metadata.side.clone().unwrap_or(Side::Both),
        }
        .as_str()
        .to_string();
        let source = if metadata.download_mode.as_deref() == Some("metadata:curseforge") {
            "curseforge"
        } else if metadata.github_project.is_some() {
            "github"
        } else if metadata
            .download_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            "url"
        } else {
            "local"
        }
        .to_string();
        rows.push(ModRow {
            name,
            filename,
            side,
            source,
            project_id: metadata.curseforge_project_id,
            file_id: metadata.curseforge_file_id,
        });
    }

    rows.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    fs::create_dir_all(output_dir)
        .map_err(|err| format!("failed to create {}: {err}", output_dir.display()))?;
    let md_path = output_dir.join("modlist.md");
    let csv_path = output_dir.join("modlist.csv");
    fs::write(&md_path, markdown(&rows))
        .map_err(|err| format!("failed to write {}: {err}", md_path.display()))?;
    fs::write(&csv_path, csv(&rows))
        .map_err(|err| format!("failed to write {}: {err}", csv_path.display()))?;
    Ok((
        rows.len(),
        md_path.to_string_lossy().to_string(),
        csv_path.to_string_lossy().to_string(),
    ))
}

fn markdown(rows: &[ModRow]) -> String {
    let mut out =
        String::from("# Mod List\n\n| Name | File | Side | Source |\n| --- | --- | --- | --- |\n");
    for row in rows {
        out.push_str("| ");
        out.push_str(&escape_md(&row.name));
        out.push_str(" | ");
        out.push_str(&escape_md(&row.filename));
        out.push_str(" | ");
        out.push_str(&escape_md(&row.side));
        out.push_str(" | ");
        out.push_str(&escape_md(&row.source));
        out.push_str(" |\n");
    }
    out
}

fn csv(rows: &[ModRow]) -> String {
    let mut out =
        String::from("name,filename,side,source,curseforge_project_id,curseforge_file_id\n");
    for row in rows {
        out.push_str(&csv_cell(&row.name));
        out.push(',');
        out.push_str(&csv_cell(&row.filename));
        out.push(',');
        out.push_str(&csv_cell(&row.side));
        out.push(',');
        out.push_str(&csv_cell(&row.source));
        out.push(',');
        if let Some(value) = row.project_id {
            out.push_str(&value.to_string());
        }
        out.push(',');
        if let Some(value) = row.file_id {
            out.push_str(&value.to_string());
        }
        out.push('\n');
    }
    out
}

fn escape_md(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

fn csv_cell(value: &str) -> String {
    let mut escaped = value.replace('"', "\"\"");
    if escaped
        .chars()
        .find(|ch| !ch.is_whitespace() && *ch != '\u{feff}')
        .is_some_and(|ch| matches!(ch, '=' | '+' | '-' | '@' | '\t' | '\r' | '\n'))
    {
        escaped.insert(0, '\'');
    }
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_cell_escapes_formula_prefixes() {
        assert_eq!(csv_cell("=cmd"), "\"'=cmd\"");
        assert_eq!(csv_cell("@link"), "\"'@link\"");
        assert_eq!(csv_cell(" \u{feff}=cmd"), "\"' \u{feff}=cmd\"");
    }
}
