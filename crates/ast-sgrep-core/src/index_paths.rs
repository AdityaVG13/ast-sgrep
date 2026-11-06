//! Incremental path-list expansion helpers for indexing.
//! Extracted from `index.rs`. Leaf-ward of `Indexer::update_paths`, which
//! keeps enforcing `MAX_INCREMENTAL_PATHS`; names are re-exported through
//! `index` so external callers keep compiling unchanged.

use crate::gitignore::{should_skip_dir, should_skip_file};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Maximum exact paths accepted by one incremental update request.
pub const MAX_INCREMENTAL_PATHS: usize = 1_024;

/// Expand directory targets into contained indexable files for explicit
/// `index_repo` / `--path` requests. Watch events must keep calling
/// `update_paths` directly so a directory chmod does not re-walk the tree.
pub fn expand_incremental_path_list(paths: Vec<PathBuf>, max: usize) -> Vec<PathBuf> {
    let max = max.min(MAX_INCREMENTAL_PATHS);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        if out.len() >= max {
            break;
        }
        if path.is_dir() {
            for file in walk_indexable_files(&path, max.saturating_sub(out.len())) {
                if seen.insert(file.clone()) {
                    out.push(file);
                    if out.len() >= max {
                        break;
                    }
                }
            }
        } else if seen.insert(path.clone()) {
            out.push(path);
        }
    }
    out
}

fn walk_indexable_files(dir: &Path, max: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if max == 0 {
        return files;
    }
    for entry in WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.file_type().is_dir() {
                !should_skip_dir(entry.path())
            } else {
                true
            }
        })
    {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if should_skip_file(path) {
            continue;
        }
        files.push(path.to_path_buf());
        if files.len() >= max {
            break;
        }
    }
    files
}
