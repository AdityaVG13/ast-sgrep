//! Gitignore-style path matching with directory-scoped rules and negation.

use std::path::Path;

mod glob;
mod rules;

/// Returns whether `rel` (relative to `root`) is ignored by gitignore rules.
pub fn is_ignored(root: &Path, rel: &Path) -> bool {
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    rules::is_path_ignored(&rel_str, &rules::collect_rules(root, rel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn star_suffix_matches_extension() {
        assert!(super::glob::glob_matches("*.pyc", "foo/bar.pyc"));
        assert!(!super::glob::glob_matches("*.pyc", "foo/bar.py"));
    }

    #[test]
    fn double_star_prefix() {
        assert!(super::glob::glob_matches("**/*.log", "deep/nested/app.log"));
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
