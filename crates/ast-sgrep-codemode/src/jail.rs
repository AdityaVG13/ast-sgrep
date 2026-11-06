//! Single root-jail primitive for per-call paths.
//!
//! `root_arg`, `jail_rel_path`, and `incremental_paths` used to re-do the
//! same traversal-reject + canonicalize + containment check with the `..`
//! detector living in two places (decoded `Component::ParentDir` here vs a
//! raw string-segment split in the query-scope filter). Every per-call path
//! that must stay under the session workspace resolves through [`jail_join`],
//! which keeps the strictest semantics of the copies:
//!
//! * `..` rejection via decoded components — platform-correct: on Windows a
//!   backslash IS a separator so `..\` rejects, while on Unix a backslash is
//!   an ordinary filename byte and `literal\..\name.ts` stays a literal
//!   name (pinned by `read_refs_preserve_literal_backslashes`). The
//!   scope-filter's string-segment split is a different layer (silent drop
//!   of query scopes, not path resolution) and stays where it is;
//! * the workspace root itself is canonicalized, so a symlinked root cannot
//!   widen containment;
//! * the resolved path must be valid UTF-8 and contained in the root.
//!
//! Existence policy differs by caller contract and stays explicit:
//! [`JailResolve::Strict`] (root overrides, reads) requires the target to
//! exist; [`JailResolve::LenientLeaf`] (targeted reindex) preserves a
//! missing leaf's identity via `canonicalize_affected_path` so new/deleted
//! files keep working. Error KIND is typed ([`JailKind`]) so each caller
//! maps to its exact legacy message; [`JailError`] also carries the I/O
//! cause so error-API cause chains survive.

use std::path::{Component, Path, PathBuf};

/// Existence policy for the final path component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JailResolve {
    /// `Path::canonicalize`: the target must exist.
    Strict,
    /// `canonicalize_affected_path`: a missing leaf keeps its identity so
    /// targeted reindex can address new or deleted files.
    LenientLeaf,
}

/// Why the jail refused a path. Callers map each kind to their pinned
/// legacy message; no message text lives here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JailKind {
    /// `..` detected by either detector, before touching disk.
    Traversal,
    /// The workspace root itself would not canonicalize.
    UnresolvableRoot,
    /// The candidate would not resolve under the chosen policy.
    Unresolvable,
    /// The resolved path escapes the workspace root.
    Outside,
    /// The resolved path is not valid UTF-8.
    NonUtf8,
}

/// A refused jail resolution: the kind plus the inputs that produced it.
/// Canonicalization failures keep the underlying `std::io::Error` so
/// `anyhow` context chains (pinned by the error-API suites) survive.
#[derive(Debug)]
pub struct JailError {
    pub kind: JailKind,
    pub raw: String,
    pub candidate: PathBuf,
    pub source: Option<std::io::Error>,
}

impl std::fmt::Display for JailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            JailKind::Traversal => write!(f, "path traversal rejected: {}", self.raw),
            JailKind::UnresolvableRoot => {
                write!(f, "cannot resolve session root: {}", self.candidate.display())
            }
            JailKind::Unresolvable => {
                write!(f, "cannot resolve path: {}", self.candidate.display())
            }
            JailKind::Outside => write!(
                f,
                "path is outside session root: {}",
                self.candidate.display()
            ),
            JailKind::NonUtf8 => write!(f, "path is not valid UTF-8: {}", self.raw),
        }
    }
}

impl std::error::Error for JailError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|cause| cause as &(dyn std::error::Error + 'static))
    }
}

impl JailError {
    /// Wrap into `anyhow` with caller-supplied context, preserving any
    /// underlying I/O failure as the cause chain.
    pub fn into_anyhow(self, context: String) -> anyhow::Error {
        match self.source {
            Some(cause) => anyhow::Error::new(cause).context(context),
            None => anyhow::Error::msg(context),
        }
    }
}

fn refuse(
    kind: JailKind,
    raw: &str,
    candidate: PathBuf,
    source: Option<std::io::Error>,
) -> JailError {
    JailError {
        kind,
        raw: raw.to_string(),
        candidate,
        source,
    }
}

/// Resolve `raw` under workspace `root`, or refuse with the reason why.
///
/// `raw` may be relative (joined under `root`) or absolute (still jailed).
/// On success returns the canonical absolute path inside `root`.
pub fn jail_join(
    root: &Path,
    raw: &str,
    resolve: JailResolve,
) -> Result<PathBuf, JailError> {
    // `..` rejection on decoded components: platform-correct on every OS.
    // (The query-scope filter's raw string-segment split is a different
    // layer — silent scope drop, not path resolution — and stays there.)
    let requested = Path::new(raw);
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(refuse(JailKind::Traversal, raw, root.join(raw), None));
    }
    let base = root
        .canonicalize()
        .map_err(|cause| refuse(JailKind::UnresolvableRoot, raw, root.to_path_buf(), Some(cause)))?;
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        base.join(requested)
    };
    let resolved = match resolve {
        JailResolve::Strict => candidate.canonicalize(),
        JailResolve::LenientLeaf => {
            ast_sgrep_core::canonicalize_affected_path(&candidate)
        }
    }
    .map_err(|cause| refuse(JailKind::Unresolvable, raw, candidate.clone(), Some(cause)))?;
    if resolved.to_str().is_none() {
        return Err(refuse(JailKind::NonUtf8, raw, resolved, None));
    }
    if !resolved.starts_with(&base) {
        return Err(refuse(JailKind::Outside, raw, resolved, None));
    }
    Ok(resolved)
}
