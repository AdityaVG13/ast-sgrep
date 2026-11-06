use super::lock_clear_on_poison;
use super::types::{SearchHit, SearchResponse};
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn estimate_prevented_reads(root: &Path, hits: &[SearchHit]) -> (u64, u64, u64) {
    use std::path::Component;
    use std::sync::OnceLock;
    const META_CACHE_CAP: usize = 4_096;
    struct MetaCache {
        entries: Vec<(String, u64)>,
    }
    static META_CACHE: OnceLock<Mutex<MetaCache>> = OnceLock::new();
    let cache = META_CACHE.get_or_init(|| Mutex::new(MetaCache { entries: Vec::new() }));
    let mut files = HashSet::new();
    let mut read_bytes_estimate = 0u64;
    {
        let mut guard = lock_clear_on_poison(cache, |cache| cache.entries.clear());
        for h in hits {
            if !files.insert(h.file.as_str()) {
                continue;
            }
            // Sanitize: reject absolute escapes and parent-directory joins (89er).
            let hit_path = Path::new(&h.file);
            if hit_path.is_absolute()
                || hit_path.components().any(|c| {
                    matches!(
                        c,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                continue;
            }
            let key = root.join(hit_path).to_string_lossy().into_owned();
            let len = match guard
                .entries
                .binary_search_by(|(name, _)| name.as_str().cmp(&key))
            {
                Ok(i) => guard.entries[i].1,
                Err(_) => {
                    let n = std::fs::metadata(&key).ok().map(|m| m.len()).unwrap_or(0);
                    if guard.entries.len() >= META_CACHE_CAP {
                        guard.entries.remove(0);
                    }
                    match guard
                        .entries
                        .binary_search_by(|(name, _)| name.as_str().cmp(&key))
                    {
                        Ok(_) => {}
                        Err(i) => guard.entries.insert(i, (key, n)),
                    }
                    n
                }
            };
            read_bytes_estimate += len;
        }
    }
    let returned_excerpt_bytes = hits.iter().map(|h| h.excerpt.len() as u64).sum();
    (
        read_bytes_estimate,
        returned_excerpt_bytes,
        read_bytes_estimate.saturating_sub(returned_excerpt_bytes),
    )
}
pub(crate) fn record_ledger(ledger_path: Option<&Path>, response: &SearchResponse) {
    let Some(path) = ledger_path else {
        return;
    };
    let path = Path::new(&path);
    // Constrain ledger writes: absolute path required; no `..`; must stay under cwd (5xf2).
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        eprintln!("[asgrep] ignoring ASGREP_LEDGER_PATH: must be an absolute path without '..'");
        return;
    }
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let Ok(cwd) = crate::paths::canonicalize_path(&cwd, true) else {
        return;
    };
    let parent = path.parent().unwrap_or(path);
    let Ok(parent_canon) = crate::paths::canonicalize_path(parent, true) else {
        // Parent may not exist yet; require it under cwd by prefix check on the raw absolute path.
        if !path.starts_with(&cwd) {
            eprintln!("[asgrep] ignoring ASGREP_LEDGER_PATH: outside process cwd");
            return;
        }
        try_append_ledger(path, response);
        return;
    };
    if !parent_canon.starts_with(&cwd) {
        eprintln!("[asgrep] ignoring ASGREP_LEDGER_PATH: outside process cwd");
        return;
    }
    try_append_ledger(path, response);
}
/// Best-effort ledger append: search must not fail, but write errors are visible.
fn try_append_ledger(path: &Path, response: &SearchResponse) {
    if let Err(e) = append_ledger_entry(path, response) {
        eprintln!(
            "[asgrep] warning: failed to write ASGREP_LEDGER_PATH {}: {e}",
            path.display()
        );
    }
}
fn append_ledger_entry(path: &Path, response: &SearchResponse) -> std::io::Result<()> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut line = serde_json::to_vec(&serde_json::json!({
        "ts": ts, "query": response.query, "hits": response.hits.len(), "bytes": {
            "read_bytes_estimate": response.read_bytes_estimate, "returned_excerpt_bytes": response.returned_excerpt_bytes,
            "prevented_read_bytes": response.prevented_read_bytes, },
    })).map_err(std::io::Error::other)?;
    line.push(b'\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(&line)
}
