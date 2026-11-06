//! Watch-path normalize / canonicalize / skip helpers for indexing.
//! Extracted from `index.rs`. Leaf helpers only; `Indexer::update_paths`
//! stays in `index`.

use crate::gitignore::{should_skip_dir, should_skip_file};
use std::path::{Path, PathBuf};

pub use crate::paths::canonicalize_affected_path;

/// Normalize a watcher path against a canonicalized index root.
pub(crate) fn normalize_watch_path(root: &Path, input_path: &Path) -> Option<PathBuf> {
    let candidate = if input_path.is_absolute() {
        input_path.to_path_buf()
    } else {
        root.join(input_path)
    };
    canonicalize_affected_path(&candidate)
        .ok()
        .filter(|canonical| canonical.starts_with(root))
}

/// Guard predicate for watch updates: skip-dir components, skip-file policy, gitignore.
/// Callers still handle empty-rel / directory continues separately (those do not bump files_skipped).
pub(crate) fn should_skip_watch_path(
    abs: &Path,
    rel: &Path,
    respect_gitignore: bool,
    ignore: &crate::gitignore::IgnoreMatcher,
) -> bool {
    // Same short-circuit order as the former inline condition in `update_paths`.
    rel.components()
        .any(|c| should_skip_dir(Path::new(c.as_os_str())))
        || should_skip_file(abs)
        || (respect_gitignore && ignore.is_ignored(rel))
}
