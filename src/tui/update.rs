use super::{
    form::{Field, Form, Kind},
    i18n::Language,
};
use crate::catalog::updates::Preview;

pub struct VersionEditor {
    path: String,
    curseforge: Option<u64>,
    github: String,
}
impl VersionEditor {
    pub fn open(entry: &super::files::Entry) -> crate::operation::Result<(Self, Form)> {
        use crate::operation::{Error, ErrorCode};
        if entry.metadata.pin {
            return Err(Error::key(ErrorCode::Invalid, "version_unpin_first"));
        }
        let curseforge = if entry.metadata.updates_via_curseforge() {
            Some(
                entry
                    .metadata
                    .curseforge_project_id
                    .ok_or_else(|| Error::key(ErrorCode::Invalid, "missing_curseforge_project"))?,
            )
        } else {
            None
        };
        if curseforge.is_none() && entry.metadata.github_project.is_none() {
            return Err(Error::key(ErrorCode::Invalid, "update_no_provider"));
        }
        let fields = if curseforge.is_some() {
            vec![Field::new("version", "version_file_id", "", Kind::Number)]
        } else {
            vec![
                Field::new("version", "version_tag", "", Kind::Text),
                Field::new(
                    "asset",
                    "github_update_filter",
                    entry.metadata.github_asset.clone().unwrap_or_default(),
                    Kind::Text,
                ),
            ]
        };
        let mut form = Form::new("switch_version", fields);
        // Keep the title translatable; the exact path is shown as a read-only field.
        form.fields.insert(
            0,
            Field::new("target", &entry.path, "", Kind::Choice(Vec::new())),
        );
        form.focus = 1;
        Ok((
            Self {
                path: entry.path.clone(),
                curseforge,
                github: entry.metadata.github_project.clone().unwrap_or_default(),
            },
            form,
        ))
    }
    pub fn submit(
        &self,
        form: &Form,
    ) -> crate::operation::Result<(String, crate::catalog::updates::TargetVersion)> {
        use crate::{
            catalog::updates::TargetVersion,
            operation::{Error, ErrorCode},
        };
        let value = form.value("version").trim();
        let target = if let Some(project) = self.curseforge {
            TargetVersion::CurseForge {
                project,
                file: value
                    .parse::<u64>()
                    .ok()
                    .filter(|id| *id > 0)
                    .ok_or_else(|| Error::key(ErrorCode::Invalid, "version_file_required"))?,
            }
        } else {
            if value.is_empty() || value.eq_ignore_ascii_case("latest") {
                return Err(Error::key(ErrorCode::Invalid, "version_tag_required"));
            }
            TargetVersion::GitHub {
                project: self.github.clone(),
                tag: value.into(),
                asset: form.value("asset").trim().into(),
            }
        };
        Ok((self.path.clone(), target))
    }
}

pub struct Selection {
    preview: Preview,
}
impl Selection {
    pub fn open(preview: Preview, lang: Language) -> (Self, Form) {
        let mut fields: Vec<_> = preview
            .candidates
            .iter()
            .map(|candidate| {
                Field::new(
                    &candidate.relative,
                    &format!(
                        "{}\n    {}\n  → {}",
                        candidate.name, candidate.before, candidate.after
                    ),
                    "false",
                    Kind::Bool,
                )
            })
            .collect();
        fields.push(Field::new(
            "download",
            "source_download",
            "false",
            Kind::Bool,
        ));
        let mut form = Form::new("update_preview", fields);
        form.checklist = true;
        form.title = format!(
            "{} ({})",
            lang.text("update_preview"),
            preview.candidates.len()
        );
        if preview.candidates.is_empty() {
            form.error = Some(lang.text("update_no_candidates").into());
        }
        if !preview.skipped.is_empty() {
            form.preview = Some(
                preview
                    .skipped
                    .iter()
                    .map(|(path, reason)| format!("{path}: {}", lang.text(reason)))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        (Self { preview }, form)
    }
    pub fn submit(&self, form: &Form) -> (Preview, Vec<String>, bool) {
        let paths = self
            .preview
            .candidates
            .iter()
            .filter(|candidate| form.value(&candidate.relative) == "true")
            .map(|candidate| candidate.relative.clone())
            .collect();
        (
            self.preview.clone(),
            paths,
            form.value("download") == "true",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::updates::{Candidate, Version},
        github::GitHubFileInfo,
        metadata::Side,
        operation::{Control, durable, preview::Guard},
    };
    use std::fs;
    #[test]
    fn version_form_validates_source_and_exact_version_without_changing_metadata() {
        let mut entry = super::super::files::Entry {
            path: "mods/client/a.pw.toml".into(),
            name: "A".into(),
            source: "GitHub".into(),
            side: "client".into(),
            present: true,
            metadata: crate::metadata::ModMetadata::parse(
                "[update.github]\nproject = \"owner/repo\"\nasset = \"fabric\"",
            ),
        };
        let (editor, mut form) = VersionEditor::open(&entry).unwrap();
        assert!(editor.submit(&form).is_err());
        form.fields
            .iter_mut()
            .find(|f| f.key == "version")
            .unwrap()
            .value = "latest".into();
        assert!(editor.submit(&form).is_err());
        form.fields
            .iter_mut()
            .find(|f| f.key == "version")
            .unwrap()
            .value = " v1.0 ".into();
        let (path, target) = editor.submit(&form).unwrap();
        assert_eq!(path, entry.path);
        assert!(
            matches!(target, crate::catalog::updates::TargetVersion::GitHub { project, tag, asset } if project == "owner/repo" && tag == "v1.0" && asset == "fabric")
        );
        entry.metadata = crate::metadata::ModMetadata::parse(
            "[update.curseforge]\nproject-id = 1\nfile-id = 20",
        );
        let (editor, mut form) = VersionEditor::open(&entry).unwrap();
        for value in ["", "0", "-1", "abc", "18446744073709551616"] {
            form.fields
                .iter_mut()
                .find(|f| f.key == "version")
                .unwrap()
                .value = value.into();
            assert!(editor.submit(&form).is_err());
        }
        form.fields
            .iter_mut()
            .find(|f| f.key == "version")
            .unwrap()
            .value = "10".into();
        assert!(matches!(
            editor.submit(&form).unwrap().1,
            crate::catalog::updates::TargetVersion::CurseForge {
                project: 1,
                file: 10
            }
        ));
        assert_eq!(entry.metadata.curseforge_file_id, Some(20));
        entry.metadata.pin = true;
        assert!(VersionEditor::open(&entry).is_err());
        entry.metadata =
            crate::metadata::ModMetadata::parse("[download]\nmode = \"metadata:curseforge\"");
        assert!(
            matches!(VersionEditor::open(&entry), Err(error) if error.message.as_deref() == Some("missing_curseforge_project"))
        );
        entry.metadata = crate::metadata::ModMetadata::parse(
            "[download]\nurl = \"https://example.invalid/a.jar\"",
        );
        assert!(VersionEditor::open(&entry).is_err());
    }

    #[test]
    fn update_selection_can_exclude_candidates_and_defaults_to_metadata() {
        let root = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        fs::write(root.join("pack.toml"), "name = \"Test\"").unwrap();
        let candidates = ["a", "b"]
            .into_iter()
            .map(|name| Candidate {
                relative: format!("mods/{name}.pw.toml"),
                name: name.into(),
                side: Side::Both,
                before: "old".into(),
                after: "new".into(),
                version: Version::GitHub(GitHubFileInfo {
                    name: name.into(),
                    filename: format!("{name}.jar"),
                    url: String::new(),
                    hash_format: "sha256".into(),
                    hash: "a".repeat(64),
                }),
            })
            .collect();
        let preview = Preview {
            guard: Guard::capture(&root, &Control::default()).unwrap(),
            candidates,
            skipped: vec![("mods/pinned.pw.toml".into(), "update_pinned")],
        };
        let (selection, mut form) = Selection::open(preview, Language::ZhCn);
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let key =
            |form: &mut Form, code| form.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        assert!(selection.submit(&form).1.is_empty());
        key(&mut form, KeyCode::Down);
        key(&mut form, KeyCode::Char(' '));
        let (_, paths, download) = selection.submit(&form);
        assert_eq!(paths, vec!["mods/b.pw.toml"]);
        assert!(!download);
        for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
            form.event(Event::Key(KeyEvent::new(KeyCode::Char('a'), modifiers)));
            assert_eq!(selection.submit(&form).1, paths);
        }
        let mut screen =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 12)).unwrap();
        screen
            .draw(|frame| form.draw(frame, Language::ZhCn))
            .unwrap();
        let text: String = screen
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("[x] b") && text.contains("old") && text.contains("→ new"));
        assert!(text.contains("mods/pinned.pw.toml"));
        assert!(form.preview.as_ref().unwrap().contains("已跳过"));
        form.fields[1].label = format!(
            "b\n{}old-v1.jar\n{}new-v2.jar",
            "长前缀".repeat(30),
            "长前缀".repeat(30)
        );
        screen.backend_mut().resize(30, 12);
        for _ in 0..30 {
            key(&mut form, KeyCode::Right);
        }
        screen
            .draw(|frame| form.draw(frame, Language::ZhCn))
            .unwrap();
        let text: String = screen
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("old-v1.jar") && text.contains("new-v2.jar"));
        key(&mut form, KeyCode::Left);
        screen
            .draw(|frame| form.draw(frame, Language::ZhCn))
            .unwrap();
        let text: String = screen
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!text.contains("new-v2.jar")); // No overscroll delay when reversing direction.
        assert_eq!(selection.submit(&form).1, vec!["mods/b.pw.toml"]);
        assert_eq!(
            key(&mut form, KeyCode::Enter),
            super::super::form::Action::Submit
        );
        key(&mut form, KeyCode::Char('a'));
        assert_eq!(selection.submit(&form).1.len(), 2);
        assert!(!selection.submit(&form).2);
        key(&mut form, KeyCode::Char('a'));
        assert!(selection.submit(&form).1.is_empty());
        assert_eq!(
            key(&mut form, KeyCode::Esc),
            super::super::form::Action::Cancel
        );
        fs::remove_dir_all(root).unwrap();
    }
}
