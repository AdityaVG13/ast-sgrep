//! Compact search-id expansion (`impl McpServer`).
//! Extracted from `lib.rs`. `tool_code_read` accepts compact
//! `<path_id>:<start>-<end>` ids as well as `path#Lstart-Lend`; the
//! expansion helpers live here so the dispatch and tool handlers stay in
//! `lib`.

use super::McpServer;
use ast_sgrep_core::IndexStore;
use ast_sgrep_plugins::compact_path_id;
use std::collections::HashMap;
use std::path::Path;

impl McpServer {
    /// Expand a compact `<path_id>:<start>-<end>` id into the node id form that
    /// `read_node` understands. Non-compact ids pass through untouched.
    pub(crate) fn resolve_compact_id(&self, root: &Path, id: &str) -> String {
        if id.contains("#L") {
            return id.to_owned();
        }
        let Some((path_id, range)) = id.rsplit_once(':') else {
            return id.to_owned();
        };
        let Some((start, end)) = range.split_once('-') else {
            return id.to_owned();
        };
        if start.is_empty() || !start.bytes().all(|b| b.is_ascii_digit()) {
            return id.to_owned();
        }
        if end.is_empty() || !end.bytes().all(|b| b.is_ascii_digit()) {
            return id.to_owned();
        }
        let needle = path_id.split('~').next().unwrap_or(path_id);
        match self.lookup_compact_path(root, needle) {
            Some(path) => format!("{path}#L{start}-L{end}"),
            None => id.to_owned(),
        }
    }

    pub(crate) fn lookup_compact_path(&self, root: &Path, needle: &str) -> Option<String> {
        let gen = ast_sgrep_core::read_writer_generation(root, self.index_path.as_deref());
        {
            let guard = Self::lock_or_recover(&self.compact_paths, |intern| *intern = None);
            if let Some((cached_gen, cached_root, map)) = guard.as_ref() {
                if *cached_gen == gen && cached_root == root {
                    return map.get(needle).cloned();
                }
            }
        }
        let paths = self.indexed_paths_for(root)?;
        let mut map = HashMap::with_capacity(paths.len());
        for path in paths {
            map.entry(compact_path_id(&path)).or_insert(path);
        }
        let found = map.get(needle).cloned();
        *Self::lock_or_recover(&self.compact_paths, |intern| *intern = None) =
            Some((gen, root.to_path_buf(), map));
        found
    }

    pub(crate) fn indexed_paths_for(&self, root: &Path) -> Option<Vec<String>> {
        // A cached Searcher pins a DEFERRED snapshot: it is only authoritative
        // when its recorded writer generation is still the stamp on disk. An
        // external `asgrep index` bumps the stamp; trusting the stale snapshot
        // would launder old paths under the new generation (see
        // lookup_compact_path). The miss falls through to a fresh readonly
        // open; the cache itself is rebuilt on the next search.
        let generation = ast_sgrep_core::read_writer_generation(root, self.index_path.as_deref());
        if let Ok(guard) = self.searcher_cache.lock() {
            if let Some(cached) = guard.as_ref() {
                let fresh = cached.root == root && cached.writer_generation == generation;
                if fresh {
                    return cached.searcher.store().all_file_paths().ok();
                }
            }
        }
        IndexStore::open_readonly(root, self.index_path.as_deref())
            .ok()
            .and_then(|store| store.all_file_paths().ok())
    }
}
