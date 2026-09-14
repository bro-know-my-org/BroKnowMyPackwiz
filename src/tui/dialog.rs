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
    Edit {
        relative: String,
        document: Box<DocumentMut>,
        expected: Option<String>,
    },
    Open,
    Preferences,
}
pub struct Dialog {
    pub form: Form,
    purpose: Purpose,
}
pub enum Submission {
    Edit(Draft),
    Open(PathBuf),
    Preferences(Preferences),
}

impl Dialog {
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
