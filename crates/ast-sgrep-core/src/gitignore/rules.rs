use std::fs;
use std::path::{Component, Path};

use super::glob::pattern_matches;

#[derive(Debug, Clone)]
pub(crate) struct Rule {
    pub base: String,
    pub pattern: String,
    pub negate: bool,
    pub dir_only: bool,
}

pub(crate) fn collect_rules(root: &Path, rel: &Path) -> Vec<Rule> {
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

pub fn rel_under_base<'a>(rule: &Rule, rel_str: &'a str) -> Option<&'a str> {
    if rule.base.is_empty() {
        return Some(rel_str);
    }
    rel_str
        .strip_prefix(&rule.base)
        .map(|rest| rest.strip_prefix('/').unwrap_or(rest))
}

pub fn matches_file(rule: &Rule, rel_str: &str) -> bool {
    if rule.dir_only {
        return false;
    }
    let Some(path) = rel_under_base(rule, rel_str) else {
        return false;
    };
    pattern_matches(&rule.pattern, path, false)
}

pub fn matches_dir(rule: &Rule, dir_path: &str) -> bool {
    let Some(path) = rel_under_base(rule, dir_path) else {
        return false;
    };
    pattern_matches(&rule.pattern, path, rule.dir_only)
}

pub fn parent_dir_excluded(rel_str: &str, rules: &[Rule]) -> bool {
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

pub fn dir_ignored(dir_path: &str, rules: &[Rule]) -> bool {
    let mut ignored = false;
    for rule in rules {
        if matches_dir(rule, dir_path) {
            ignored = !rule.negate;
        }
    }
    ignored
}

pub fn is_path_ignored(rel_str: &str, rules: &[Rule]) -> bool {
    if parent_dir_excluded(rel_str, rules) {
        return true;
    }

    let mut ignored = false;
    for rule in rules {
        if matches_file(rule, rel_str) {
            ignored = !rule.negate;
        }
    }
    ignored
}
