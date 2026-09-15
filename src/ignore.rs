use std::fs;
use std::path::Path;

use crate::operation::{Error, ErrorCode};
use crate::pathutil::normalize_slash;

#[derive(Debug, Clone)]
pub struct IgnoreSet {
    rules: Vec<IgnoreRule>,
}

#[derive(Debug, Clone)]
struct IgnoreRule {
    pattern: String,
    negated: bool,
    directory_only: bool,
    anchored: bool,
}

impl IgnoreSet {
    pub fn empty() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn load_operation(path: &Path) -> Result<Self, Error> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::empty()),
            Err(err) => Err(Error::named(
                ErrorCode::Failed,
                "read_file_failed",
                format!("failed to read {}: {err}", path.display()),
            )
            .context(format!("{}: {err}", path.display()))),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn parse(text: &str) -> Self {
        let mut rules = Vec::new();
        for raw in text.lines() {
            let mut line = raw.trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with("\\#") {
                line = &line[1..];
            } else if line.starts_with('#') {
                continue;
            }

            let negated = if line.starts_with("\\!") {
                line = &line[1..];
                false
            } else {
                line.starts_with('!')
            };
            if negated {
                line = line[1..].trim();
            }
            if line.is_empty() {
                continue;
            }

            let anchored = line.starts_with('/');
            let directory_only = line.ends_with('/');
            if directory_only {
                line = &line[..line.len() - 1];
            }

            rules.push(IgnoreRule {
                pattern: normalize_slash(line.trim_start_matches('/')),
                negated,
                directory_only,
                anchored,
            });
        }
        Self { rules }
    }

    pub fn is_ignored(&self, rel_path: &str, is_dir: bool) -> bool {
        let rel_path = normalize_slash(rel_path);
        let mut ignored = false;
        for rule in &self.rules {
            let matches = if rule.directory_only && !is_dir {
                rule.matches_parent_dir(&rel_path)
            } else {
                rule.matches(&rel_path)
            };
            if matches {
                ignored = !rule.negated;
            }
        }
        ignored
    }

    pub fn is_explicitly_included(&self, rel_path: &str, is_dir: bool) -> bool {
        let rel_path = normalize_slash(rel_path);
        for rule in &self.rules {
            let matches = if rule.directory_only && !is_dir {
                rule.matches_parent_dir(&rel_path)
            } else {
                rule.matches(&rel_path)
            };
            if matches && rule.negated {
                return true;
            }
        }
        false
    }
}

impl IgnoreRule {
    fn matches(&self, rel_path: &str) -> bool {
        if self.pattern.is_empty() {
            return false;
        }

        if self.pattern.contains('/') {
            return path_segment_wildcard_match(&self.pattern, rel_path);
        }

        if self.anchored {
            return !rel_path.contains('/') && wildcard_match(&self.pattern, rel_path);
        }

        rel_path
            .split('/')
            .any(|part| wildcard_match(&self.pattern, part))
    }

    fn matches_parent_dir(&self, rel_path: &str) -> bool {
        if self.pattern.contains('/') {
            let Some(parent) = rel_path.rsplit_once('/').map(|(parent, _)| parent) else {
                return false;
            };
            return parent
                .split('/')
                .scan(String::new(), |prefix, part| {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(part);
                    Some(prefix.clone())
                })
                .any(|candidate| path_segment_wildcard_match(&self.pattern, &candidate));
        }
        if self.anchored {
            return rel_path
                .split('/')
                .next()
                .is_some_and(|part| wildcard_match(&self.pattern, part));
        }
        rel_path
            .split('/')
            .any(|part| wildcard_match(&self.pattern, part))
    }
}

fn path_segment_wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern_parts = pattern.split('/').collect::<Vec<_>>();
    let text_parts = text.split('/').collect::<Vec<_>>();
    path_segment_wildcard_match_parts(&pattern_parts, &text_parts)
}

fn path_segment_wildcard_match_parts(pattern_parts: &[&str], text_parts: &[&str]) -> bool {
    if pattern_parts.is_empty() {
        return text_parts.is_empty();
    }

    if pattern_parts[0] == "**" {
        return path_segment_wildcard_match_parts(&pattern_parts[1..], text_parts)
            || (!text_parts.is_empty()
                && path_segment_wildcard_match_parts(pattern_parts, &text_parts[1..]));
    }

    !text_parts.is_empty()
        && wildcard_match(pattern_parts[0], text_parts[0])
        && path_segment_wildcard_match_parts(&pattern_parts[1..], &text_parts[1..])
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let (mut p, mut t) = (0, 0);
    let mut star = None;
    let mut match_after_star = 0;

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            match_after_star = t;
            p += 1;
        } else if let Some(star_pos) = star {
            p = star_pos + 1;
            match_after_star += 1;
            t = match_after_star;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_basic_gitignore_shapes() {
        let set = IgnoreSet::parse("target/\n*.tmp\n!important.tmp\n");

        assert!(set.is_ignored("target", true));
        assert!(set.is_ignored("target/a.jar", false));
        assert!(set.is_ignored("notes.tmp", false));
        assert!(!set.is_ignored("important.tmp", false));
        assert!(set.is_explicitly_included("important.tmp", false));
        assert!(!set.is_explicitly_included("notes.tmp", false));
    }

    #[test]
    fn path_wildcards_do_not_cross_directories() {
        let set = IgnoreSet::parse("foo/*.jar\n");

        assert!(set.is_ignored("foo/a.jar", false));
        assert!(!set.is_ignored("foo/bar/a.jar", false));
    }

    #[test]
    fn double_star_crosses_directories() {
        let set = IgnoreSet::parse("foo/**\n");

        assert!(set.is_ignored("foo/a.jar", false));
        assert!(set.is_ignored("foo/bar/a.jar", false));
    }

    #[test]
    fn leading_slash_anchors_pattern_to_root() {
        let set = IgnoreSet::parse("/*.zip\n");

        assert!(set.is_ignored("pack.zip", false));
        assert!(!set.is_ignored("tacz/pack.zip", false));
    }

    #[test]
    fn escaped_comment_and_negation_prefixes_are_literals() {
        let set = IgnoreSet::parse("\\#idea\n\\!important.tmp\n*.tmp\n");

        assert!(set.is_ignored("#idea", false));
        assert!(set.is_ignored("!important.tmp", false));
        assert!(!set.is_explicitly_included("!important.tmp", false));
    }
}
