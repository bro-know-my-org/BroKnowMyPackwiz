use super::{
    form::{Field, Form, Kind},
    i18n::Language,
    preferences::Preferences,
};
use crate::operation::{
    Error, ErrorCode, Result,
    edit::{self, Draft},
};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Value};

pub const SETTINGS: [&str; 4] = ["pack_info", "project_config", "preferences", "open_pack"];
enum Purpose {
    GitHub(super::github::Picker),
    Source(super::source::Editor),
    CurseForge(super::catalog::Selection),
    Prepared {
        request: crate::operation::queue::Request,
        label: String,
    },
    Edit {
        relative: String,
        document: Box<DocumentMut>,
        expected: Option<String>,
    },
    Open,
    Preferences,
    Resolve {
        index: usize,
        keep: bool,
    },
}
pub struct Dialog {
    pub form: Form,
    purpose: Purpose,
}
pub enum Submission {
    GitHub(super::github::Action),
    Source {
        input: crate::catalog::source::Input,
        options: crate::catalog::source::Options,
    },
    CurseForge {
        selected: super::catalog::Selection,
        side: crate::metadata::Side,
    },
    Prepared {
        request: crate::operation::queue::Request,
        label: String,
    },
    Edit(Draft),
    Open(PathBuf),
    Preferences(Preferences),
    Resolve {
        index: usize,
        keep: bool,
    },
}

impl Dialog {
    pub fn github(picker: super::github::Picker, mut form: Form, lang: Language) -> Self {
        picker.preview(&mut form, lang);
        Self {
            form,
            purpose: Purpose::GitHub(picker),
        }
    }
    pub fn refresh_preview(&mut self, lang: Language) {
        if let Purpose::GitHub(picker) = &self.purpose {
            picker.preview(&mut self.form, lang);
        }
    }
    pub fn source(input: crate::catalog::source::Input) -> Self {
        let (editor, form) = super::source::Editor::open(input);
        Self {
            form,
            purpose: Purpose::Source(editor),
        }
    }
    pub fn source_prepared(prepared: crate::catalog::source::Prepared, lang: Language) -> Self {
        let mut form = Form::new("review_changes", Vec::new());
        form.preview = Some(format!(
            "{}\n{}\nSHA-256: {}\n{}: {}",
            prepared.name,
            prepared.filename,
            prepared.sha256,
            lang.text("source_download"),
            lang.text(if prepared.download { "yes" } else { "no" })
        ));
        Self {
            form,
            purpose: Purpose::Prepared {
                request: prepared.request,
                label: "source_add".into(),
            },
        }
    }
    pub fn curseforge(selected: super::catalog::Selection) -> Self {
        let sides: Vec<String> = if selected.class_id == 6 {
            vec!["both".into(), "client".into(), "server".into()]
        } else {
            vec!["client".into()]
        };
        let mut form = Form::new(
            "cf_plan",
            vec![Field::new(
                "side",
                "declared_side",
                sides[0].clone(),
                Kind::Choice(sides),
            )],
        );
        form.preview = Some(format!(
            "{}\n{}\n{}",
            selected.file.name,
            selected.file.filename,
            selected.file.versions.join(", ")
        ));
        Self {
            form,
            purpose: Purpose::CurseForge(selected),
        }
    }
    pub fn prepared(preview: crate::catalog::prepare::Preview, lang: Language) -> Self {
        use crate::catalog::dependencies::Action;
        let mut form = Form::new("cf_confirm", Vec::new());
        form.preview = Some(
            preview
                .rows
                .iter()
                .map(|row| {
                    format!(
                        "{} · {} · {}\n{} → {}\n{}",
                        lang.text(match row.action {
                            Action::Add => "add",
                            Action::Update => "cf_update",
                            Action::Reuse => "cf_reuse",
                        }),
                        row.name,
                        row.side.as_str(),
                        row.old
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "—".into()),
                        row.file.id,
                        row.file.filename
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
        );
        Self {
            form,
            purpose: Purpose::Prepared {
                request: crate::operation::queue::Request::PreparedEdit {
                    drafts: preview.drafts,
                    guard: preview.guard,
                },
                label: "cf_add_task".into(),
            },
        }
    }
    pub fn conflicts(
        queue: &crate::operation::queue::Queue,
        index: usize,
        keep: bool,
        lang: Language,
    ) -> Result<Self> {
        let files = queue.conflict_files(index)?;
        let mut form = Form::new(
            if keep {
                "keep_external"
            } else {
                "restore_original"
            },
            Vec::new(),
        );
        form.preview = Some(
            files
                .iter()
                .map(|file| {
                    format!(
                        "{}: {}\n{}: {}\n{}: {}",
                        lang.text("current_file"),
                        file.target.display(),
                        lang.text("original_backup"),
                        file.backup.display(),
                        lang.text("proposed_file"),
                        file.staged.display()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
        );
        if files.is_empty() {
            form.preview = Some(lang.text("conflict_no_writes").into());
        }
        Ok(Self {
            form,
            purpose: Purpose::Resolve { index, keep },
        })
    }
    pub fn review(
        relative: String,
        expected: Option<String>,
        document: DocumentMut,
        preview: String,
    ) -> Self {
        let mut form = Form::new("review_changes", Vec::new());
        form.preview = Some(preview);
        Self {
            form,
            purpose: Purpose::Edit {
                relative,
                document: Box::new(document),
                expected,
            },
        }
    }
    pub fn metadata(root: &Path, relative: &str) -> Result<Self> {
        Self::edit(
            root,
            relative,
            "edit_metadata",
            vec![
                ("name", "name", Kind::Text),
                ("filename", "filename", Kind::Text),
                (
                    "side",
                    "declared_side",
                    Kind::Choice(vec!["both".into(), "client".into(), "server".into()]),
                ),
                ("pin", "pin", Kind::Bool),
                ("preserve", "preserve", Kind::Bool),
                ("option.optional", "optional", Kind::Bool),
                ("option.default", "default_enabled", Kind::Bool),
                ("download.url", "url", Kind::Text),
                ("download.mode", "download_mode", Kind::Text),
                ("download.hash-format", "hash_format", Kind::Text),
                ("download.hash", "hash", Kind::Text),
                (
                    "export.curseforge.project-id",
                    "cf_export_project",
                    Kind::Number,
                ),
                ("export.curseforge.file-id", "cf_export_file", Kind::Number),
                ("export.curseforge.latest", "cf_export_latest", Kind::Bool),
            ],
        )
    }

    pub fn settings(root: &Path, index: usize, prefs: &Preferences) -> Result<Self> {
        match index {
            0 => Self::edit(
                root,
                "pack.toml",
                "pack_info",
                vec![
                    ("name", "name", Kind::Text),
                    ("author", "author", Kind::Text),
                    ("version", "version", Kind::Text),
                    ("versions.minecraft", "minecraft", Kind::Text),
                    ("versions.neoforge", "neoforge", Kind::Text),
                    ("versions.forge", "forge", Kind::Text),
                ],
            ),
            1 => Self::edit(
                root,
                ".pw/config.toml",
                "project_config",
                vec![
                    ("scan.use-gitignore", "use_gitignore", Kind::Bool),
                    ("scan.packwizignore", "packwizignore", Kind::Text),
                    ("install.jobs", "jobs", Kind::Number),
                    ("install.retries", "retries", Kind::Number),
                    ("install.retry-delay-seconds", "retry_delay", Kind::Number),
                    ("install.force", "force", Kind::Bool),
                    ("curseforge.api-key", "api_key", Kind::Secret),
                    ("curseforge.cdn-fallback", "cdn_fallback", Kind::Bool),
                    ("release.enabled", "release_enabled", Kind::Bool),
                    ("release.template-dir", "template_dir", Kind::Text),
                ],
            ),
            2 => Ok(Self {
                form: Form::new(
                    "preferences",
                    vec![
                        Field::new(
                            "language",
                            "language",
                            match prefs.language {
                                None => "auto",
                                Some(Language::ZhCn) => "zh-CN",
                                Some(Language::En) => "en",
                            },
                            Kind::Choice(vec!["auto".into(), "zh-CN".into(), "en".into()]),
                        ),
                        Field::new(
                            "editor",
                            "editor",
                            prefs.editor.clone().unwrap_or_default(),
                            Kind::Text,
                        ),
                    ],
                ),
                purpose: Purpose::Preferences,
            }),
            _ => {
                let mut recent: Vec<_> = prefs
                    .recent
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                let current = root.display().to_string();
                if !recent.contains(&current) {
                    recent.insert(0, current.clone());
                }
                Ok(Self {
                    form: Form::new(
                        "open_pack",
                        vec![
                            Field::new("path", "directory", current.clone(), Kind::Text),
                            Field::new("recent", "recent", current, Kind::Choice(recent)),
                        ],
                    ),
                    purpose: Purpose::Open,
                })
            }
        }
    }

    fn edit(
        root: &Path,
        relative: &str,
        title: &str,
        schema: Vec<(&str, &str, Kind)>,
    ) -> Result<Self> {
        let (document, expected) = edit::document(root, relative)?;
        let config = crate::config::ProjectConfig::load(root).map_err(Error::from)?;
        let fields = schema
            .into_iter()
            .map(|(key, label, kind)| {
                let value = get(&document, key)
                    .and_then(Item::as_value)
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .or_else(|| v.as_bool().map(|v| v.to_string()))
                            .or_else(|| v.as_integer().map(|v| v.to_string()))
                            .unwrap_or_else(|| v.to_string().trim().to_string())
                    })
                    .unwrap_or_else(|| {
                        if relative == ".pw/config.toml" {
                            match key {
                                "scan.use-gitignore" => {
                                    return config.scan.use_gitignore.to_string();
                                }
                                "scan.packwizignore" => {
                                    return config.scan.packwizignore.display().to_string();
                                }
                                "install.jobs" => return config.install.jobs.to_string(),
                                "install.retries" => return config.install.retries.to_string(),
                                "install.retry-delay-seconds" => {
                                    return config.install.retry_delay_seconds.to_string();
                                }
                                "install.force" => return config.install.force.to_string(),
                                "curseforge.cdn-fallback" => {
                                    return config.curseforge.cdn_fallback.to_string();
                                }
                                "release.enabled" => return config.release.enabled.to_string(),
                                _ => {}
                            }
                        }
                        if let Kind::Choice(options) = &kind {
                            options.first().cloned().unwrap_or_default()
                        } else if matches!(kind, Kind::Bool) {
                            "false".into()
                        } else {
                            String::new()
                        }
                    });
                Field::new(key, label, value, kind)
            })
            .collect();
        Ok(Self {
            form: Form::new(title, fields),
            purpose: Purpose::Edit {
                relative: relative.into(),
                document: Box::new(document),
                expected,
            },
        })
    }

    pub fn submit(&self, state: &Path, prefs: &Preferences) -> Result<Submission> {
        match &self.purpose {
            Purpose::GitHub(picker) => Ok(Submission::GitHub(picker.submit(&self.form)?)),
            Purpose::Source(editor) => {
                let (input, options) = editor.submit(&self.form)?;
                Ok(Submission::Source { input, options })
            }
            Purpose::CurseForge(selected) => Ok(Submission::CurseForge {
                selected: selected.clone(),
                side: crate::metadata::Side::parse(self.form.value("side")),
            }),
            Purpose::Prepared { request, label } => Ok(Submission::Prepared {
                request: request.clone(),
                label: label.clone(),
            }),
            Purpose::Resolve { index, keep } => Ok(Submission::Resolve {
                index: *index,
                keep: *keep,
            }),
            Purpose::Open => {
                let path = &self.form.fields[0];
                let recent = &self.form.fields[1];
                let value = if path.value == path.initial && recent.value != recent.initial {
                    &recent.value
                } else {
                    &path.value
                };
                if value.trim().is_empty() {
                    return Err(Error::new(ErrorCode::Invalid, "empty directory"));
                }
                Ok(Submission::Open(PathBuf::from(value)))
            }
            Purpose::Preferences => {
                let mut prefs = prefs.clone();
                prefs.language = match self.form.value("language") {
                    "zh-CN" => Some(Language::ZhCn),
                    "en" => Some(Language::En),
                    _ => None,
                };
                prefs.editor = (!self.form.value("editor").trim().is_empty())
                    .then(|| self.form.value("editor").to_string());
                Ok(Submission::Preferences(prefs))
            }
            Purpose::Edit {
                relative,
                document,
                expected,
            } => {
                let mut doc = (**document).clone();
                for field in &self.form.fields {
                    if field.value == field.initial {
                        continue;
                    }
                    let keys: Vec<_> = field.key.split('.').collect();
                    if field.value.is_empty() {
                        if field.key == "filename" {
                            return Err(Error::new(ErrorCode::Invalid, "filename is required"));
                        }
                        remove(&mut doc, &keys);
                        continue;
                    }
                    let value = match field.kind {
                        Kind::Bool => Value::from(field.value == "true"),
                        Kind::Number => {
                            let number: i64 = field
                                .value
                                .parse()
                                .map_err(|_| Error::new(ErrorCode::Invalid, &field.label))?;
                            if number < 0
                                || (number == 0
                                    && (field.key.ends_with("-id") || field.key.ends_with(".jobs")))
                            {
                                return Err(Error::new(ErrorCode::Invalid, &field.label));
                            }
                            Value::from(number)
                        }
                        _ => Value::from(field.value.clone()),
                    };
                    edit::set(&mut doc, &keys, value)?;
                }
                if let Some(filename) = doc.get("filename").and_then(Item::as_str) {
                    crate::pathutil::safe_slash_path(filename).map_err(Error::from)?;
                }
                Ok(Submission::Edit(edit::draft(
                    state,
                    relative,
                    expected.clone(),
                    &doc,
                )?))
            }
        }
    }
}

fn get<'a>(document: &'a DocumentMut, key: &str) -> Option<&'a Item> {
    let mut parts = key.split('.');
    let mut item = document.get(parts.next()?)?;
    for part in parts {
        item = item.get(part)?;
    }
    Some(item)
}
fn remove(document: &mut DocumentMut, keys: &[&str]) {
    let Some((last, parents)) = keys.split_last() else {
        return;
    };
    let mut table = document.as_table_mut();
    for key in parents {
        let Some(next) = table.get_mut(key).and_then(Item::as_table_mut) else {
            return;
        };
        table = next;
    }
    table.remove(last);
}
