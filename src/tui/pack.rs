use super::form::{Field, Form, Kind as FieldKind};
use crate::operation::{
    Error, ErrorCode, Result,
    command::{Kind, Output, Request},
};

pub const ACTIONS: &[Kind] = &[
    Kind::Inspect,
    Kind::List,
    Kind::Scan,
    Kind::Check,
    Kind::Refresh,
    Kind::DownloadFiles,
    Kind::Sync,
    Kind::InstallLocal,
    Kind::PreparePack,
    Kind::PrepareServer,
    Kind::Modlist,
    Kind::ExportClient,
    Kind::ExportServer,
    Kind::ExportServerInstaller,
    Kind::ExportCurseForge,
    Kind::Hash,
    Kind::Init,
];

pub fn form(kind: Kind, lang: super::i18n::Language) -> Form {
    let mut fields = Vec::new();
    if kind.output() != Output::None {
        fields.push(Field::new(
            "output",
            "command_output",
            kind.default_output().unwrap_or(""),
            FieldKind::Text,
        ));
    }
    if matches!(
        kind,
        Kind::Sync | Kind::InstallLocal | Kind::ExportCurseForge
    ) {
        fields.push(Field::new(
            "side",
            "declared_side",
            "both",
            FieldKind::Choice(vec!["both".into(), "client".into(), "server".into()]),
        ));
    }
    if kind == Kind::ExportClient {
        fields.push(Field::new("root_dir", "archive_root", "", FieldKind::Text));
    }
    if kind.installation() {
        for (key, label) in [
            ("jobs", "jobs"),
            ("retries", "retries"),
            ("delay", "retry_delay"),
        ] {
            fields.push(Field::new(key, label, "", FieldKind::Number));
        }
        fields.push(Field::new("force", "force", "false", FieldKind::Bool));
        if kind == Kind::InstallLocal {
            fields.push(Field::new(
                "cleanup",
                "command_cleanup",
                "default",
                FieldKind::Choice(vec!["default".into(), "true".into(), "false".into()]),
            ));
        }
    }
    if kind == Kind::Hash {
        fields.push(Field::new("input", "source_path", "", FieldKind::Text));
        fields.push(Field::new(
            "algorithm",
            "hash_format",
            "sha256",
            FieldKind::Choice(vec![
                "sha256".into(),
                "sha1".into(),
                "sha512".into(),
                "murmur2".into(),
            ]),
        ));
    }
    let mut form = Form::new(kind.command(), fields);
    form.preview = Some(
        lang.text(if kind.readonly() {
            "command_readonly"
        } else {
            "command_transaction"
        })
        .into(),
    );
    form
}

pub fn submit(kind: Kind, form: &Form) -> Result<Request> {
    let mut request = Request::new(kind);
    if kind.output() != Output::None && !form.value("output").trim().is_empty() {
        request.output = Some(form.value("output").into());
    }
    if matches!(
        kind,
        Kind::Sync | Kind::InstallLocal | Kind::ExportCurseForge
    ) {
        request.side = form.value("side").into();
    }
    request.root_dir = form.value("root_dir").into();
    if kind.installation() {
        request.jobs = number(form, "jobs")?;
        request.retries = number(form, "retries")?;
        request.retry_delay_seconds = number(form, "delay")?;
        request.force = form.value("force") == "true";
        request.cleanup = match form.value("cleanup") {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
    }
    if kind == Kind::Hash {
        if form.value("input").trim().is_empty() {
            return Err(Error::new(ErrorCode::Invalid, "hash_input_required"));
        }
        request.input = Some(form.value("input").into());
        request.algorithm = form.value("algorithm").into();
    }
    request.validate()?;
    if kind == Kind::InstallLocal && request.output.is_none() {
        return Err(Error::new(ErrorCode::Invalid, "output_required"));
    }
    Ok(request)
}

fn number<T: std::str::FromStr>(form: &Form, key: &str) -> Result<Option<T>> {
    let value = form.value(key).trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse()
        .map(Some)
        .map_err(|_| Error::new(ErrorCode::Invalid, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installation_form_preserves_defaults_and_rejects_invalid_jobs() {
        let mut form = form(Kind::InstallLocal, super::super::i18n::Language::ZhCn);
        assert!(submit(Kind::InstallLocal, &form).is_err());
        form.fields
            .iter_mut()
            .find(|f| f.key == "output")
            .unwrap()
            .value = "../installed".into();
        let request = submit(Kind::InstallLocal, &form).unwrap();
        assert_eq!(request.jobs, None);
        assert_eq!(request.cleanup, None);
        form.fields
            .iter_mut()
            .find(|f| f.key == "jobs")
            .unwrap()
            .value = "0".into();
        assert!(submit(Kind::InstallLocal, &form).is_err());
    }
}
