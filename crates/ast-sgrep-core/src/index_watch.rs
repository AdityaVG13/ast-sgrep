//! Watch-path normalize / canonicalize / skip helpers for indexing.
//! Extracted from `index.rs` (EXP-008 / F-004 watch-path cluster). Leaf helpers
//! only; `Indexer::update_paths` stays in `index`. FORCE_SIDECAR stays in `index`
//! (F-003 escalate — do not extract).

use crate::gitignore::{should_skip_dir, should_skip_file};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

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

/// Resolve the nearest existing ancestor without following the final path
/// component. This confines intermediate symlinks while preserving the indexed
/// key for a newly created or deleted file.
pub fn canonicalize_affected_path(path: &Path) -> std::io::Result<PathBuf> {
    let Some(name) = path.file_name() else {
        return path.canonicalize();
    };
    let mut existing = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut suffix = vec![name.to_os_string()];
    loop {
        match existing.canonicalize() {
            Ok(mut canonical) => {
                for component in suffix.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
            {
                let Some(name) = existing.file_name().map(ToOwned::to_owned) else {
                    return Err(error);
                };
                suffix.push(name);
                if !existing.pop() {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
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
