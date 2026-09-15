use super::{dialog::Dialog, preferences::Preferences};
use crate::operation::{Error, ErrorCode, Result, durable};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub struct External {
    relative: String,
    expected: Option<String>,
    original: String,
    source: PathBuf,
    command: Vec<String>,
}

impl External {
    pub fn prepare(root: &Path, relative: &str, state: &Path, prefs: &Preferences) -> Result<Self> {
        crate::operation::paths::relative(relative)?;
        let path = durable::absolute(&durable::canonical(root)?.join(relative))?;
        let expected = durable::fingerprint(&path)?;
        let original = if expected.is_some() {
            fs::read_to_string(&path)?
        } else {
            String::new()
        };
        if durable::fingerprint(&path)? != expected {
            return Err(Error::new(ErrorCode::Conflict, relative));
        }
        let dir = state.join("drafts");
        fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
        let source = dir.join(format!("editor-{}.toml", durable::unique_id()));
        durable::write(&source, original.as_bytes())?;
        let command = prefs
            .editor
            .clone()
            .or_else(|| std::env::var("VISUAL").ok())
            .or_else(|| std::env::var("EDITOR").ok())
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "notepad.exe".into()
                } else {
                    "vi".into()
                }
            });
        let command = if Path::new(&command).is_file() {
            vec![command]
        } else {
            shell_words::split(&command)
                .map_err(|e| Error::new(ErrorCode::Invalid, e.to_string()))?
        };
        if command.is_empty() {
            return Err(Error::named(
                ErrorCode::Invalid,
                "empty_editor_command",
                "empty editor command",
            ));
        }
        Ok(Self {
            relative: relative.into(),
            expected,
            original,
            source,
            command,
        })
    }

    pub fn run(self) -> Result<Dialog> {
        let status = Command::new(&self.command[0])
            .args(&self.command[1..])
            .arg(&self.source)
            .status()?;
        if !status.success() {
            return Err(Error::named(
                ErrorCode::Failed,
                "editor_failed",
                format!(
                    "editor exited with {status}; draft: {}",
                    self.source.display()
                ),
            )
            .context(format!("{status}; {}", self.source.display())));
        }
        let text = fs::read_to_string(&self.source)?;
        let document = text.parse::<toml_edit::DocumentMut>().map_err(|e| {
            Error::named(
                ErrorCode::Invalid,
                "editor_invalid_document",
                format!("{e}; draft: {}", self.source.display()),
            )
            .context(format!("{e}; {}", self.source.display()))
        })?;
        let preview = preview(&self.original, &text);
        Ok(Dialog::review(
            self.relative,
            self.expected,
            document,
            preview,
        ))
    }
}

fn preview(before: &str, after: &str) -> String {
    let (before, before_secrets) = redact(before);
    let (after, after_secrets) = redact(after);
    let before: Vec<_> = before.lines().collect();
    let after: Vec<_> = after.lines().collect();
    let prefix = before
        .iter()
        .zip(&after)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut output = before[prefix..before.len() - suffix]
        .iter()
        .map(|line| format!("- {line}"))
        .chain(
            after[prefix..after.len() - suffix]
                .iter()
                .map(|line| format!("+ {line}")),
        )
        .collect::<Vec<_>>()
        .join("\n");
    for key in before_secrets
        .keys()
        .chain(after_secrets.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        if before_secrets.get(key) != after_secrets.get(key) {
            output.push_str(&format!("\n- {key} = •••\n+ {key} = •••"));
        }
    }
    output
}

fn redact(text: &str) -> (String, std::collections::BTreeMap<String, String>) {
    let mut secrets = std::collections::BTreeMap::new();
    let Ok(mut document) = text.parse::<toml_edit::DocumentMut>() else {
        return ("[…]".into(), secrets);
    };
    redact_table(document.as_table_mut(), "", &mut secrets);
    (document.to_string(), secrets)
}

fn redact_table(
    table: &mut dyn toml_edit::TableLike,
    prefix: &str,
    secrets: &mut std::collections::BTreeMap<String, String>,
) {
    for (key, item) in table.iter_mut() {
        let path = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        if ["api-key", "token", "password", "secret"]
            .iter()
            .any(|word| key.to_ascii_lowercase().contains(word))
        {
            secrets.insert(path, item.to_string());
            *item = toml_edit::value("•••");
        } else if let Some(table) = item.as_table_like_mut() {
            redact_table(table, &path, secrets);
        } else if let Some(tables) = item.as_array_of_tables_mut() {
            for (i, table) in tables.iter_mut().enumerate() {
                redact_table(table, &format!("{path}.{i}"), secrets);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_does_not_disclose_credentials() {
        let text = preview(
            "name = 'Old'\napi-key = 'secret1'",
            "name = 'New'\napi-key = 'secret2'",
        );
        assert!(text.contains("- name = 'Old'"));
        assert!(text.contains("+ name = 'New'"));
        assert!(!text.contains("secret1") && !text.contains("secret2"));
        let text = preview(
            "api-key = '''hidden\nvalue1'''",
            "api-key = '''hidden\nvalue2'''",
        );
        assert!(text.contains("api-key = •••"));
        assert!(!text.contains("hidden") && !text.contains("value1") && !text.contains("value2"));
    }
}
