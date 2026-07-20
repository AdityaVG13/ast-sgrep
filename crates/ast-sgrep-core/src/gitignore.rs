use std::cell::RefCell; use std::collections::HashMap; use std::fs; use std::path::{Component, Path, PathBuf}; use std::rc::Rc;

pub const DEFAULT_SKIP_DIR_NAMES: &[&str] =
    &[".git", ".asgrep", "target", "node_modules", "dist", "build", ".cargo", "~"];
pub const INDEXABLE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "pyi", "go", "java", "cs", "rb", "toml", "md", "txt", "json", "yaml", "yml",
]; pub fn should_skip_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str()) .is_some_and(|name| DEFAULT_SKIP_DIR_NAMES.contains(&name))
} pub fn should_skip_file(path: &Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')) { return true; } path.extension()
        .and_then(|e| e.to_str()) .map(|ext| !INDEXABLE_EXTENSIONS.contains(&ext.to_lowercase().as_str())) .unwrap_or(true)
}

#[derive(Debug, Clone)] struct Rule {
    base: String, pattern: String, negate: bool, dir_only: bool,
} pub struct IgnoreMatcher {
    root: PathBuf, chains: RefCell<HashMap<String, Rc<Vec<Rule>>>>,
} impl IgnoreMatcher {
    pub fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf(), chains: RefCell::new(HashMap::new()) }
    } pub fn clear(&self) { self.chains.borrow_mut().clear(); } pub fn is_ignored(&self, rel: &Path) -> bool {
        let rel_str = rel.to_string_lossy().replace('\\', "/"); let rules = self.chain_for(&parent_prefix(rel)); if parent_dir_excluded(&rel_str, &rules) { return true; } let mut ignored = false; for rule in rules.iter() {
            if matches_file(rule, &rel_str) { ignored = !rule.negate; }
        } ignored
    } pub fn is_dir_ignored(&self, rel_dir: &Path) -> bool {
        dir_ignored(
            &rel_dir.to_string_lossy().replace('\\', "/"), &self.chain_for(&parent_prefix(rel_dir)),
        )
    } fn chain_for(&self, prefix: &str) -> Rc<Vec<Rule>> {
        if let Some(hit) = self.chains.borrow().get(prefix) { return Rc::clone(hit); } let rules = if prefix.is_empty() {
            let mut rules = default_rules(); load_dir_rules(&self.root, "", &mut rules); rules
        } else {
            let parent = prefix.rsplit_once('/').map(|(p, _)| p).unwrap_or(""); let mut rules = self.chain_for(parent).as_ref().clone(); load_dir_rules(&self.root.join(prefix), &format!("{prefix}/"), &mut rules); rules
        }; let rc = Rc::new(rules); self.chains.borrow_mut().insert(prefix.to_string(), Rc::clone(&rc)); rc
    }
} pub fn is_ignored(root: &Path, rel: &Path) -> bool { IgnoreMatcher::new(root).is_ignored(rel) } fn parent_prefix(rel: &Path) -> String {
    let mut prefix = String::new(); if let Some(parent) = rel.parent().filter(|p| !p.as_os_str().is_empty()) {
        for comp in parent.components() {
            let Component::Normal(part) = comp else { continue }; if !prefix.is_empty() { prefix.push('/'); } prefix.push_str(&part.to_string_lossy());
        }
    } prefix
} fn default_rules() -> Vec<Rule> {
    DEFAULT_SKIP_DIR_NAMES.iter().map(|p| parse_rule("", &format!("{p}/"))).collect()
} fn load_dir_rules(dir: &Path, base: &str, rules: &mut Vec<Rule>) {
    for name in [".gitignore", ".asgrepignore"] {
        if let Ok(content) = fs::read_to_string(dir.join(name)) {
            for line in content.lines() {
                if let Some(rule) = parse_rule_line(base, line) { rules.push(rule); }
            }
        }
    }
} fn parse_rule_line(base: &str, line: &str) -> Option<Rule> {
    let line = line.trim(); if line.is_empty() || line.starts_with('#') {
        None
    } else {
        Some(parse_rule(base, line))
    }
} fn parse_rule(base: &str, line: &str) -> Rule {
    let negate = line.starts_with('!'); let pat = if negate { line[1..].trim() } else { line.trim() }; Rule { base: base.to_string(), pattern: pat.to_string(), negate, dir_only: pat.ends_with('/') }
} fn rel_under_base<'a>(rule: &Rule, rel_str: &'a str) -> Option<&'a str> {
    if rule.base.is_empty() { return Some(rel_str); } rel_str.strip_prefix(&rule.base).map(|rest| rest.strip_prefix('/').unwrap_or(rest))
} fn matches_file(rule: &Rule, rel_str: &str) -> bool {
    !rule.dir_only && rel_under_base(rule, rel_str).is_some_and(|p| pattern_matches(&rule.pattern, p, false))
} fn matches_dir(rule: &Rule, dir_path: &str) -> bool {
    rel_under_base(rule, dir_path).is_some_and(|p| pattern_matches(&rule.pattern, p, rule.dir_only))
} fn pattern_matches(pattern: &str, path: &str, dir_only: bool) -> bool {
    let mut pat = pattern.trim_end_matches('/'); let anchored = pat.starts_with('/'); if anchored { pat = &pat[1..]; } let matched = if pat.contains('/') || anchored {
        glob_matches(pat, path)
    } else {
        path.split('/').any(|seg| glob_matches(pat, seg)) || glob_matches(pat, path)
    }; matched || (dir_only && (path == pat || path.starts_with(&format!("{pat}/"))))
} fn glob_matches(pattern: &str, text: &str) -> bool {
    let pat = pattern.trim_end_matches('/'); if pat.contains("**/") {
        if let Some(rest) = pat.split("**/").nth(1) {
            return glob_matches(rest, text) || text.split('/').any(|seg| glob_matches(rest, seg));
        }
    } if let Some(suffix) = pat.strip_prefix('*') { return text.ends_with(suffix) || text.split('/').any(|seg| seg.ends_with(suffix)); }
    if let Some(prefix) = pat.strip_suffix('*') { return text.starts_with(prefix) || text.split('/').any(|seg| seg.starts_with(prefix)); } text == pat || text.starts_with(&format!("{pat}/"))
} fn parent_dir_excluded(rel_str: &str, rules: &[Rule]) -> bool {
    let Some((parent, _)) = rel_str.rsplit_once('/') else { return false }; let mut prefix = String::new(); for part in parent.split('/') {
        prefix = if prefix.is_empty() { part.into() } else { format!("{prefix}/{part}") }; if dir_ignored(&prefix, rules) { return true; }
    } false
} fn dir_ignored(dir_path: &str, rules: &[Rule]) -> bool {
    let mut ignored = false; for rule in rules {
        if matches_dir(rule, dir_path) { ignored = !rule.negate; }
    } ignored
}

#[cfg(test)] mod tests {
    use super::should_skip_dir; use std::path::Path; #[test] fn skips_path_escape_noise_directory() {
        assert!(should_skip_dir(Path::new("~"))); assert!(!should_skip_dir(Path::new("src")));
    }
}
