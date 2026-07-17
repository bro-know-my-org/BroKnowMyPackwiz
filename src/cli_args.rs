/// Repairs only the pack-root operand used by legacy Windows devtool commands.
/// Some launchers preserve the outer quotes and split the quoted path on spaces
/// before spawning `bkmpw`.
pub fn repair_legacy_windows_pack_root(mut args: Vec<String>) -> Vec<String> {
    if !matches!(
        args.first().map(String::as_str),
        Some("update" | "install-files" | "install-files-headless" | "install-files-retry")
    ) {
        return args;
    }

    let Some(first) = args.get(1).and_then(|arg| arg.strip_prefix('"')) else {
        return args;
    };

    if let Some(root) = first.strip_suffix('"') {
        args[1] = root.to_string();
        return args;
    }

    let Some(end) = args[2..]
        .iter()
        .position(|arg| arg.ends_with('"'))
        .map(|offset| offset + 2)
    else {
        return args;
    };

    let mut root = first.to_string();
    for part in &args[2..=end] {
        root.push(' ');
        root.push_str(part);
    }
    root.pop();
    args.splice(1..=end, [root]);
    args
}

#[cfg(test)]
mod tests {
    use super::repair_legacy_windows_pack_root;

    #[test]
    fn joins_a_quoted_argument_split_on_spaces() {
        let args = vec![
            "update".to_string(),
            "\"C:\\UPrograms\\PCL".to_string(),
            "正式版".to_string(),
            "2.8.13\\.minecraft\\versions\\CDPR0703\"".to_string(),
            "create-delight-core".to_string(),
        ];

        assert_eq!(
            repair_legacy_windows_pack_root(args),
            vec![
                "update",
                "C:\\UPrograms\\PCL 正式版 2.8.13\\.minecraft\\versions\\CDPR0703",
                "create-delight-core",
            ]
        );
    }

    #[test]
    fn removes_literal_outer_quotes_from_one_argument() {
        let args = vec![
            "install-files-headless".to_string(),
            "\"C:\\packs\\demo\"".to_string(),
        ];

        assert_eq!(
            repair_legacy_windows_pack_root(args),
            vec!["install-files-headless", "C:\\packs\\demo"]
        );
    }

    #[test]
    fn leaves_unmatched_pack_root_unchanged() {
        let args = vec![
            "update".to_string(),
            "\"unfinished".to_string(),
            "create-delight-core".to_string(),
        ];

        assert_eq!(repair_legacy_windows_pack_root(args.clone()), args);
    }

    #[test]
    fn never_rewrites_arguments_of_other_commands() {
        let args = vec![
            "add-github".to_string(),
            "pack".to_string(),
            "both".to_string(),
            "owner/repo".to_string(),
            "--asset".to_string(),
            "\"quoted asset\"".to_string(),
        ];

        assert_eq!(repair_legacy_windows_pack_root(args.clone()), args);
    }
}
