use super::{
    form::{Field, Form, Kind},
    i18n::Language,
};
use crate::catalog::updates::Preview;

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
