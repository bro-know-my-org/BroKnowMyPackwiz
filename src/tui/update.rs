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
            .map(|candidate| Field::new(&candidate.relative, &candidate.name, "true", Kind::Bool))
            .collect();
        fields.push(Field::new(
            "download",
            "source_download",
            "false",
            Kind::Bool,
        ));
        let mut form = Form::new("update_preview", fields);
        let mut rows: Vec<_> = preview
            .candidates
            .iter()
            .map(|candidate| {
                format!(
                    "{}\n{} → {}",
                    candidate.name, candidate.before, candidate.after
                )
            })
            .collect();
        rows.extend(
            preview
                .skipped
                .iter()
                .map(|(path, key)| format!("{path}: {}", lang.text(key))),
        );
        if rows.is_empty() {
            rows.push(lang.text("update_no_candidates").into());
        }
        form.preview = Some(rows.join("\n\n"));
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
            skipped: Vec::new(),
        };
        let (selection, mut form) = Selection::open(preview, Language::ZhCn);
        form.fields[0].value = "false".into();
        let (_, paths, download) = selection.submit(&form);
        assert_eq!(paths, vec!["mods/b.pw.toml"]);
        assert!(!download);
        assert!(form.preview.as_ref().unwrap().contains("old → new"));
        fs::remove_dir_all(root).unwrap();
    }
}
