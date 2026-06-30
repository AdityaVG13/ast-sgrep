use std::fs;
use std::path::{Component, Path};

#[derive(Debug, Clone)]
struct Rule {
    base: String,
    pattern: String,
    negate: bool,
    dir_only: bool,
}

pub fn is_ignored(root: &Path, rel: &Path) -> bool {
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let rules = collect_rules(root, rel);

    if parent_dir_excluded(&rel_str, &rules) {
        return true;
    }

    let mut ignored = false;
    for rule in &rules {
        if matches_file(rule, &rel_str) {
            ignored = !rule.negate;
        }
    }
    ignored
}

fn collect_rules(root: &Path, rel: &Path) -> Vec<Rule> {
    let mut rules = default_rules();
    load_dir_rules(root, "", &mut rules);

    if let Some(parent) = rel.parent().filter(|p| !p.as_os_str().is_empty()) {
        let mut prefix = String::new();
        for comp in parent.components() {
            let Component::Normal(part) = comp else {
                continue;
            };
            prefix = if prefix.is_empty() {
                part.to_string_lossy().into_owned()
            } else {
                format!("{prefix}/{}", part.to_string_lossy())
            };
            load_dir_rules(&root.join(&prefix), &format!("{prefix}/"), &mut rules);
        }
    }

    rules
}

fn default_rules() -> Vec<Rule> {
    crate::skip::DEFAULT_SKIP_DIR_NAMES
        .iter()
        .map(|p| parse_rule("", &format!("{p}/")))
        .collect()
}

fn load_dir_rules(dir: &Path, base: &str, rules: &mut Vec<Rule>) {
    for name in [".gitignore", ".asgrepignore"] {
        let path = dir.join(name);
        if let Ok(content) = fs::read_to_string(path) {
            for line in content.lines() {
                if let Some(rule) = parse_rule_line(base, line) {
                    rules.push(rule);
                }
            }
        }
    }
}

fn parse_rule_line(base: &str, line: &str) -> Option<Rule> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    Some(parse_rule(base, line))
}

fn parse_rule(base: &str, line: &str) -> Rule {
    let negate = line.starts_with('!');
    let pat = if negate {
        line[1..].trim()
    } else {
        line.trim()
    };
    let dir_only = pat.ends_with('/');
    Rule {
        base: base.to_string(),
        pattern: pat.to_string(),
        negate,
        dir_only,
    }
}

fn rel_under_base<'a>(rule: &Rule, rel_str: &'a str) -> Option<&'a str> {
    if rule.base.is_empty() {
        return Some(rel_str);
    }
    rel_str
        .strip_prefix(&rule.base)
        .map(|rest| rest.strip_prefix('/').unwrap_or(rest))
}

fn matches_file(rule: &Rule, rel_str: &str) -> bool {
    if rule.dir_only {
        return false;
    }
    let Some(path) = rel_under_base(rule, rel_str) else {
        return false;
    };
    pattern_matches(&rule.pattern, path, false)
}

fn matches_dir(rule: &Rule, dir_path: &str) -> bool {
    let Some(path) = rel_under_base(rule, dir_path) else {
        return false;
    };
    pattern_matches(&rule.pattern, path, rule.dir_only)
}

fn pattern_matches(pattern: &str, path: &str, dir_only: bool) -> bool {
    let mut pat = pattern.trim_end_matches('/');
    let anchored = pat.starts_with('/');
    if anchored {
        pat = &pat[1..];
    }

    let matched = if pat.contains('/') || anchored {
        glob_matches(pat, path)
    } else {
        path.split('/').any(|seg| glob_matches(pat, seg)) || glob_matches(pat, path)
    };

    matched || (dir_only && (path == pat || path.starts_with(&format!("{pat}/"))))
}

fn glob_matches(pattern: &str, text: &str) -> bool {
    let pat = pattern.trim_end_matches('/');
    if pat.contains("**/") {
        if let Some(rest) = pat.split("**/").nth(1) {
            return glob_matches(rest, text)
                || text.split('/').any(|seg| glob_matches(rest, seg));
        }
    }
    if let Some(suffix) = pat.strip_prefix('*') {
        return text.ends_with(suffix) || text.split('/').any(|seg| seg.ends_with(suffix));
    }
    if let Some(prefix) = pat.strip_suffix('*') {
        return text.starts_with(prefix)
            || text.split('/').any(|seg| seg.starts_with(prefix));
    }
    text == pat || text.starts_with(&format!("{pat}/"))
}

fn parent_dir_excluded(rel_str: &str, rules: &[Rule]) -> bool {
    let Some((parent, _)) = rel_str.rsplit_once('/') else {
        return false;
    };
    let mut prefix = String::new();
    for part in parent.split('/') {
        prefix = if prefix.is_empty() {
            part.to_string()
        } else {
            format!("{prefix}/{part}")
        };
        if dir_ignored(&prefix, rules) {
            return true;
        }
    }
    false
}

fn dir_ignored(dir_path: &str, rules: &[Rule]) -> bool {
    let mut ignored = false;
    for rule in rules {
        if matches_dir(rule, dir_path) {
            ignored = !rule.negate;
        }
    }
    ignored
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn star_suffix_matches_extension() {
        assert!(glob_matches("*.pyc", "foo/bar.pyc"));
        assert!(!glob_matches("*.pyc", "foo/bar.py"));
    }

    #[test]
    fn double_star_prefix() {
        assert!(glob_matches("**/*.log", "deep/nested/app.log"));
    }

    #[test]
    fn negation_unignores_matching_files() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::write(root.join(".gitignore"), "*.log\n!important.log\n").unwrap();
        assert!(is_ignored(root, Path::new("app.log")));
        assert!(!is_ignored(root, Path::new("important.log")));
    }

    #[test]
    fn nested_gitignore_scopes_to_subdirectory() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("pkg")).unwrap();
        fs::write(root.join("pkg/.gitignore"), "*.txt\n").unwrap();
        fs::write(root.join("keep.txt"), "ok\n").unwrap();
        fs::write(root.join("pkg/skip.txt"), "no\n").unwrap();
        assert!(!is_ignored(root, Path::new("keep.txt")));
        assert!(is_ignored(root, Path::new("pkg/skip.txt")));
    }

    #[test]
    fn parent_directory_exclusion_blocks_negation() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::write(root.join(".gitignore"), "build/\n!build/keep.txt\n").unwrap();
        assert!(is_ignored(root, Path::new("build/keep.txt")));
    }
}
