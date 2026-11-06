use anyhow::{anyhow, Context};
use ast_sgrep_core::{expand_incremental_path_list, Language, MAX_INCREMENTAL_PATHS};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::jail::{JailKind, JailResolve, jail_join};

pub(super) fn incremental_paths(args: &Value, root: &Path) -> anyhow::Result<Option<Vec<PathBuf>>> {
    let Some(raw_paths) = args.get("paths") else {
        return Ok(None);
    };
    let raw_paths = raw_paths
        .as_array()
        .context("index_repo paths must be an array")?;
    if raw_paths.is_empty() {
        return Err(anyhow!("index_repo paths must be non-empty"));
    }
    if raw_paths.len() > MAX_INCREMENTAL_PATHS {
        return Err(anyhow!(
            "index_repo paths exceeds max {MAX_INCREMENTAL_PATHS}"
        ));
    }

    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve index root: {}", root.display()))?;
    let mut seen = HashSet::with_capacity(raw_paths.len());
    let mut paths = Vec::with_capacity(raw_paths.len());
    for raw in raw_paths {
        let raw = raw
            .as_str()
            .context("index_repo paths entries must be strings")?;
        if raw.is_empty() {
            return Err(anyhow!("index_repo paths entries must be non-empty"));
        }
        // Targeted reindex tolerates a missing leaf (new/deleted files keep
        // working); `..` and containment still reject with this tool's
        // legacy messages.
        let canonical = match jail_join(&root, raw, JailResolve::LenientLeaf) {
            Ok(path) => path,
            Err(err) => {
                return Err(match err.kind {
                    JailKind::Traversal => {
                        anyhow!("index_repo path traversal rejected: {raw}")
                    }
                    JailKind::UnresolvableRoot => {
                        anyhow!("cannot resolve index root: {}", root.display())
                    }
                    JailKind::Unresolvable => {
                        let candidate = err.candidate.display().to_string();
                        err.into_anyhow(format!("cannot resolve index path: {candidate}"))
                    }
                    JailKind::Outside => anyhow!(
                        "index_repo path is outside project root: {}",
                        err.candidate.display()
                    ),
                    // No legacy arm: the old code never checked UTF-8 (fail closed).
                    JailKind::NonUtf8 => {
                        anyhow!("index_repo path is not valid UTF-8: {raw}")
                    }
                });
            }
        };
        if seen.insert(canonical.clone()) {
            paths.push(canonical);
        }
    }
    Ok(Some(expand_incremental_path_list(
        paths,
        MAX_INCREMENTAL_PATHS,
    )))
}

pub(super) fn scoped_search_query(args: &Value, query: &str) -> String {
    let scope = ["in", "file_filter", "fileFilter"]
        .into_iter()
        .find_map(|key| args.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty() && !path.split(['/', '\\']).any(|segment| segment == ".."));
    match scope {
        Some(path)
            if !query
                .split_whitespace()
                .any(|token| token.starts_with("in:")) =>
        {
            format!("in:{path} {query}")
        }
        _ => query.to_string(),
    }
}

pub(super) fn optional_lang(args: &Value) -> anyhow::Result<Option<String>> {
    let Some(raw) = args
        .get("lang")
        .or_else(|| args.get("lang_filter"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let Some(language) = Language::parse(raw) else {
        return Err(anyhow!(
            "unknown lang '{raw}' (expected a stored id or extension such as rs, ts, py)"
        ));
    };
    Ok(Some(language.as_str().to_string()))
}
