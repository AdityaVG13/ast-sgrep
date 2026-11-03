use ast_sgrep_lang::Language;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
pub const DEFAULT_SKIP_DIR_NAMES: &[&str] = &[".git", ".asgrep"];

/// Document/data rows indexed alongside source: they carry no tree-sitter
/// Language but their lines feed lexical search.
pub const DOCUMENT_EXTENSIONS: &[&str] = &["toml", "md", "txt", "json", "yaml", "yml"];

/// True when the index walk should admit ext. Source extensions resolve
/// through Language::from_extension so this set can never drift from
/// SOURCE_EXTENSIONS in ast-sgrep-lang.
pub fn is_indexable_extension(ext: &str) -> bool {
    Language::from_extension(ext).is_some()
        || DOCUMENT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}
pub fn should_skip_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| DEFAULT_SKIP_DIR_NAMES.contains(&name))
}
pub fn should_skip_file(path: &Path) -> bool {
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
    {
        return true;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| !is_indexable_extension(ext))
        .unwrap_or(true)
}

/// Check one path using the repository ignore rules.
pub fn is_ignored(root: &Path, rel: &Path) -> bool {
    IgnoreMatcher::new(root).is_ignored(rel)
}

#[derive(Debug, Clone)]
struct Rule {
    base: String,
    pattern: String,
    negate: bool,
    dir_only: bool,
}

/// One matcher holds every ignore rule as a flat prefixed vector, parsed once
/// at construction. Query-time matching has no per-prefix chain map.
#[derive(Debug, Clone)]
pub struct IgnoreMatcher {
    root: PathBuf,
    rules: Vec<Rule>,
}
impl IgnoreMatcher {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            rules: load_all_rules(root),
        }
    }
    pub fn clear(&mut self) {
        let root = self.root.clone();
        *self = Self::new(&root);
    }
    pub fn is_ignored(&self, rel: &Path) -> bool {
        let rel_str = rel
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        if parent_dir_excluded(&rel_str, &self.rules) {
            return true;
        }
        let mut ignored = false;
        for rule in &self.rules {
            if matches_file(rule, &rel_str) {
                ignored = !rule.negate;
            }
        }
        ignored
    }
    pub fn is_dir_ignored(&self, rel_dir: &Path) -> bool {
        dir_ignored(
            &rel_dir
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/"),
            &self.rules,
        )
    }
}

fn load_all_rules(root: &Path) -> Vec<Rule> {
    let rules = std::cell::RefCell::new(default_rules());
    load_dir_rules(root, "", &mut rules.borrow_mut());
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if !entry.file_type().is_dir() || should_skip_dir(entry.path()) {
                return false;
            }
            let Ok(rel) = entry.path().strip_prefix(root) else {
                return false;
            };
            if rel.as_os_str().is_empty() {
                return true;
            }
            let rel_str = rel
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            let mut rules = rules.borrow_mut();
            if dir_ignored(&rel_str, &rules) {
                return false;
            }
            load_dir_rules(entry.path(), &format!("{rel_str}/"), &mut rules);
            true
        })
    {
        let _ = entry;
    }
    rules.into_inner()
}
fn default_rules() -> Vec<Rule> {
    DEFAULT_SKIP_DIR_NAMES
        .iter()
        .map(|p| parse_rule("", &format!("{p}/")))
        .collect()
}
fn load_dir_rules(dir: &Path, base: &str, rules: &mut Vec<Rule>) {
    for name in [".gitignore", ".asgrepignore"] {
        if let Ok(content) = fs::read_to_string(dir.join(name)) {
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
        None
    } else {
        Some(parse_rule(base, line))
    }
}
fn parse_rule(base: &str, line: &str) -> Rule {
    let negate = line.starts_with('!');
    let pat = if negate {
        line[1..].trim()
    } else {
        line.trim()
    };
    Rule {
        base: base.to_string(),
        pattern: pat.to_string(),
        negate,
        dir_only: pat.ends_with('/'),
    }
}
fn rel_under_base<'a>(rule: &Rule, rel_str: &'a str) -> Option<&'a str> {
    if rule.base.is_empty() {
        return Some(rel_str);
    }
    rel_str
        .strip_prefix(rule.base.as_str())
        .map(|rest| rest.strip_prefix('/').unwrap_or(rest))
}
fn matches_file(rule: &Rule, rel_str: &str) -> bool {
    !rule.dir_only
        && rel_under_base(rule, rel_str).is_some_and(|p| pattern_matches(&rule.pattern, p, false))
}
fn matches_dir(rule: &Rule, dir_path: &str) -> bool {
    rel_under_base(rule, dir_path).is_some_and(|p| pattern_matches(&rule.pattern, p, rule.dir_only))
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
            return glob_matches(rest, text) || text.split('/').any(|seg| glob_matches(rest, seg));
        }
    }
    // A single '*' follows gitignore segment semantics and never crosses '/'.
    // Root rule /* therefore ignores root entries but not descendants of an
    // explicitly re-included directory such as !/crates/.
    if !pat.contains('/') && text.contains('/') {
        return false;
    }
    if let Some(suffix) = pat.strip_prefix('*') {
        return text.ends_with(suffix) || text.split('/').any(|seg| seg.ends_with(suffix));
    }
    if let Some(prefix) = pat.strip_suffix('*') {
        return text.starts_with(prefix) || text.split('/').any(|seg| seg.starts_with(prefix));
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
            part.into()
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
