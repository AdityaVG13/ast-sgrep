use std::path::Path;
pub const DEFAULT_SKIP_DIR_NAMES: &[&str] =
    &[".git", ".asgrep", "target", "node_modules", "dist", "build", ".cargo"];
pub const INDEXABLE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "pyi", "go", "java", "cs", "rb", "toml",
    "md", "txt", "json", "yaml", "yml",
];
pub fn should_skip_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| DEFAULT_SKIP_DIR_NAMES.contains(&name))
}
pub fn should_skip_file(path: &Path) -> bool {
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.starts_with('.'))
    { return true; }
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| !INDEXABLE_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(true)
}
