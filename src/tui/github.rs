use super::{
    form::{Field, Form, Kind},
    i18n::Language,
};
use crate::{
    catalog::{
        github::{Asset, Page, Release},
        source::Input,
    },
    operation::{Error, ErrorCode, Result},
};

pub enum Action {
    Releases {
        repository: String,
        page: usize,
    },
    Assets {
        repository: String,
        release: Release,
        page: usize,
    },
    Add(Input),
}
enum Step {
    Repository,
    Releases {
        repository: String,
        page: usize,
        items: Page<Release>,
    },
    Assets {
        repository: String,
        release: Release,
        page: usize,
        items: Page<Asset>,
    },
}
pub struct Picker {
    step: Step,
}
impl Picker {
    pub fn repository() -> (Self, Form) {
        (
            Self {
                step: Step::Repository,
            },
            Form::new(
                "github_repository",
                vec![Field::new(
                    "repository",
                    "github_repository",
                    String::new(),
                    Kind::Text,
                )],
            ),
        )
    }
    pub fn releases(repository: String, page: usize, items: Page<Release>) -> (Self, Form) {
        let choices: Vec<_> = items.items.iter().map(|r| r.tag.clone()).collect();
        let form = form("github_releases", choices, page);
        (
            Self {
                step: Step::Releases {
                    repository,
                    page,
                    items,
                },
            },
            form,
        )
    }
    pub fn assets(
        repository: String,
        release: Release,
        page: usize,
        items: Page<Asset>,
    ) -> (Self, Form) {
        let choices: Vec<_> = items.items.iter().map(|a| a.name.clone()).collect();
        let form = form("github_assets", choices, page);
        (
            Self {
                step: Step::Assets {
                    repository,
                    release,
                    page,
                    items,
                },
            },
            form,
        )
    }
    pub fn preview(&self, form: &mut Form, lang: Language) {
        let text = match &self.step {
            Step::Repository => return,
            Step::Releases { items, .. } => items
                .items
                .iter()
                .find(|r| r.tag == form.value("choice"))
                .map(|r| {
                    format!(
                        "{}\n{}\n{}: {}\n\n{}",
                        r.name,
                        r.published,
                        lang.text("github_prerelease"),
                        lang.text(if r.prerelease { "yes" } else { "no" }),
                        r.body
                    )
                }),
            Step::Assets { items, release, .. } => items
                .items
                .iter()
                .find(|a| a.name == form.value("choice"))
                .map(|a| {
                    format!(
                        "{} · {}\n{}\n{} bytes\nSHA-256: {}",
                        release.tag,
                        release.name,
                        a.name,
                        a.size,
                        a.sha256
                            .as_deref()
                            .unwrap_or(lang.text("github_hash_pending"))
                    )
                }),
        };
        form.preview = Some(text.unwrap_or_else(|| lang.text("github_empty").into()));
    }
    pub fn submit(&self, form: &Form) -> Result<Action> {
        if let Step::Repository = &self.step {
            return Ok(Action::Releases {
                repository: crate::github::normalize_project(form.value("repository"))
                    .map_err(Error::from)?,
                page: 0,
            });
        }
        let page = form
            .value("page")
            .parse::<usize>()
            .ok()
            .and_then(|v| v.checked_sub(1))
            .ok_or_else(|| invalid("github_page_required"))?;
        match &self.step {
            Step::Releases {
                repository,
                page: current,
                items,
            } => {
                if page != *current {
                    if page > *current && !items.has_more {
                        return Err(invalid("github_no_next_page"));
                    }
                    return Ok(Action::Releases {
                        repository: repository.clone(),
                        page,
                    });
                }
                let release = items
                    .items
                    .iter()
                    .find(|r| r.tag == form.value("choice"))
                    .ok_or_else(|| invalid("github_empty"))?;
                Ok(Action::Assets {
                    repository: repository.clone(),
                    release: release.clone(),
                    page: 0,
                })
            }
            Step::Assets {
                repository,
                release,
                page: current,
                items,
            } => {
                if page != *current {
                    if page > *current && !items.has_more {
                        return Err(invalid("github_no_next_page"));
                    }
                    return Ok(Action::Assets {
                        repository: repository.clone(),
                        release: release.clone(),
                        page,
                    });
                }
                let asset = items
                    .items
                    .iter()
                    .find(|a| a.name == form.value("choice"))
                    .ok_or_else(|| invalid("github_empty"))?;
                Ok(Action::Add(Input::GitHub {
                    repository: repository.clone(),
                    asset: asset.clone(),
                    update_tag: "latest".into(),
                    update_filter: asset.name.clone(),
                }))
            }
            Step::Repository => unreachable!(),
        }
    }
}
fn form(title: &str, choices: Vec<String>, page: usize) -> Form {
    Form::new(
        title,
        vec![
            Field::new(
                "choice",
                title,
                choices.first().cloned().unwrap_or_default(),
                Kind::Choice(choices),
            ),
            Field::new("page", "github_page", (page + 1).to_string(), Kind::Number),
        ],
    )
}
fn invalid(key: &str) -> Error {
    Error::new(ErrorCode::Invalid, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selecting_a_release_uses_its_identity_and_page_changes_reload() {
        let releases = Page {
            items: vec![Release {
                id: 123,
                name: "中文版本".into(),
                tag: "v1".into(),
                prerelease: false,
                published: String::new(),
                body: String::new(),
            }],
            has_more: true,
        };
        let (picker, mut form) = Picker::releases("owner/repo".into(), 0, releases);
        let Action::Assets { release, page, .. } = picker.submit(&form).unwrap() else {
            panic!()
        };
        assert_eq!(release.id, 123);
        assert_eq!(page, 0);
        form.fields
            .iter_mut()
            .find(|f| f.key == "page")
            .unwrap()
            .value = "2".into();
        assert!(matches!(
            picker.submit(&form).unwrap(),
            Action::Releases { page: 1, .. }
        ));
        picker.preview(&mut form, Language::ZhCn);
        assert!(form.preview.as_ref().unwrap().contains("中文版本"));
    }
}
