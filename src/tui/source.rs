use super::form::{Field, Form, Kind};
use crate::{
    catalog::source::{Input, Options},
    metadata::Side,
    operation::{Error, ErrorCode, Result},
};

pub struct Editor {
    input: Input,
}
impl Editor {
    pub fn open(input: Input) -> (Self, Form) {
        let mut fields = Vec::new();
        let filename = match &input {
            Input::Url { url, sha256 } => {
                fields.push(Field::new("url", "url", url.clone(), Kind::Text));
                fields.push(Field::new(
                    "hash",
                    "source_hash",
                    sha256.clone(),
                    Kind::Text,
                ));
                String::new()
            }
            Input::Local(path) => {
                fields.push(Field::new(
                    "path",
                    "source_path",
                    path.to_string_lossy(),
                    Kind::Text,
                ));
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }
            Input::GitHub {
                asset,
                update_tag,
                update_filter,
                ..
            } => {
                fields.push(Field::new(
                    "tag",
                    "github_update_tag",
                    update_tag.clone(),
                    Kind::Text,
                ));
                fields.push(Field::new(
                    "asset",
                    "github_update_filter",
                    update_filter.clone(),
                    Kind::Text,
                ));
                asset.name.clone()
            }
        };
        fields.extend([
            Field::new("name", "name", String::new(), Kind::Text),
            Field::new("filename", "filename", filename, Kind::Text),
            Field::new(
                "kind",
                "cf_type",
                "mods",
                Kind::Choice(vec![
                    "mods".into(),
                    "resourcepacks".into(),
                    "shaderpacks".into(),
                ]),
            ),
            Field::new(
                "side",
                "declared_side",
                "both",
                Kind::Choice(vec!["both".into(), "client".into(), "server".into()]),
            ),
            Field::new("download", "source_download", "false", Kind::Bool),
        ]);
        (Self { input }, Form::new("source_add", fields))
    }
    pub fn submit(&self, form: &Form) -> Result<(Input, Options)> {
        let input = match &self.input {
            Input::Url { .. } => Input::Url {
                url: form.value("url").trim().into(),
                sha256: form.value("hash").trim().into(),
            },
            Input::Local(_) => {
                if form.value("path").trim().is_empty() {
                    return Err(Error::new(ErrorCode::Invalid, "source_path_required"));
                }
                Input::Local(form.value("path").into())
            }
            Input::GitHub {
                repository, asset, ..
            } => Input::GitHub {
                repository: repository.clone(),
                asset: asset.clone(),
                update_tag: form.value("tag").into(),
                update_filter: form.value("asset").into(),
            },
        };
        let filename = if form.value("filename").is_empty() {
            match &input {
                Input::Local(path) => path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                _ => String::new(),
            }
        } else {
            form.value("filename").into()
        };
        let options = Options {
            name: form.value("name").into(),
            filename,
            kind: form.value("kind").into(),
            side: Side::parse(form.value("side")),
            download: form.value("download") == "true",
        };
        if options.name.trim().is_empty() {
            return Err(Error::new(ErrorCode::Invalid, "name_required"));
        }
        crate::pathutil::safe_filename(&options.filename).map_err(Error::from)?;
        Ok((input, options))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_form_defaults_to_metadata_and_derives_the_source_filename() {
        let (editor, mut form) = Editor::open(Input::Local("/tmp/中文.jar".into()));
        assert_eq!(form.value("download"), "false");
        assert_eq!(form.value("filename"), "中文.jar");
        assert!(editor.submit(&form).is_err());
        form.fields
            .iter_mut()
            .find(|f| f.key == "name")
            .unwrap()
            .value = "Test".into();
        let (input, options) = editor.submit(&form).unwrap();
        assert!(matches!(input, Input::Local(_)));
        assert_eq!(options.filename, "中文.jar");
        assert!(!options.download);
    }
}
