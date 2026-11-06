//! Central path-identity policy (F6).
//!
//! Every canonicalization in core routes through
//! [`canonicalize_path`], which takes an explicit follow-symlinks yes/no
//! parameter so scope-admission sites cannot silently diverge on
//! symlink/UNC/error behavior:
//!
//! - `follow_symlinks = true` is exactly `std::fs::canonicalize`
//!   (root pinning, db/source fingerprinting, ledger confinement).
//! - `follow_symlinks = false` resolves only existing ancestors and
//!   preserves the final component, so a not-yet-existing (or deleted)
//!   indexed key keeps its identity instead of erroring.
//!
//! [`indexed_rel_path`] also lives here: DB key validation is the same
//! path-identity family. `index` and `index_watch` re-export both names so
//! existing `crate::index::…` / `crate::…` paths keep working.

use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

/// Canonicalize `path`, explicitly choosing symlink behavior.
///
/// `true` follows symlinks (identical to [`Path::canonicalize`]);
/// `false` walks up to the nearest existing ancestor, canonicalizes that,
/// and re-appends the missing suffix unresolved.
pub fn canonicalize_path(path: &Path, follow_symlinks: bool) -> std::io::Result<PathBuf> {
    if follow_symlinks {
        return path.canonicalize();
    }
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

/// Resolve the nearest existing ancestor without following the final path
/// component. This confines intermediate symlinks while preserving the indexed
/// key for a newly created or deleted file.
pub fn canonicalize_affected_path(path: &Path) -> std::io::Result<PathBuf> {
    canonicalize_path(path, false)
}

/// Indexed relative paths must be valid UTF-8. Lossy conversion is forbidden:
/// two distinct non-UTF8 `OsStr` paths must not collide into one DB key.
///
/// Also rejects absolute paths and `..` / root / prefix components so an
/// untrusted relative key cannot escape the project root when later joined
/// (defense-in-depth alongside MCP `parse_node_id` and search 89er).
pub fn indexed_rel_path(rel: &Path) -> crate::Result<String> {
    if rel.as_os_str().is_empty() {
        return Err(crate::StoreError::Other(
            "empty relative path rejected (asgrep-kqhp)".into(),
        ));
    }
    if rel.is_absolute()
        || rel.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(crate::StoreError::Other(format!(
            "path traversal rejected (asgrep-kqhp): {}",
            rel.display()
        )));
    }
    let raw = rel.to_str().ok_or_else(|| {
        crate::StoreError::Other(format!(
            "non-UTF8 path rejected (asgrep-kqhp): {}",
            rel.display()
        ))
    })?;
    if raw.contains('\0') {
        return Err(crate::StoreError::Other(format!(
            "NUL in path rejected (asgrep-kqhp): {}",
            rel.display()
        )));
    }
    #[cfg(windows)]
    {
        Ok(raw.replace('\\', "/"))
    }
    #[cfg(not(windows))]
    {
        // A backslash is an ordinary filename byte on Unix. Rewriting it after
        // component validation can turn `..\\escape.rs` into a traversal path
        // and can collide with the distinct `dir/file.rs` name.
        Ok(raw.to_owned())
    }
}
