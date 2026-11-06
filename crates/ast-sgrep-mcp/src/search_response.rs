//! Search-response trio (`impl McpServer`).
//! Extracted from `lib.rs` verbatim: `tool_agent_search`, `elide_seen_snippets`,
//! `diagnose_miss`. Dispatch (`dispatch_tool`), catalog, `from_env`, the
//! searcher pipeline, and the index trio stay in `lib`.

use super::envelope::{fnv1a64, render_compact_envelope};
use super::{
    AgentSearchArgs, AgentSearchMode, ELIDED_SNIPPET, MAX_EMITTED_SNIPPETS, McpServer, PreviewMode,
};
use ast_sgrep_core::IndexStore;
use ast_sgrep_plugins::{MissContext, to_compact_miss_json};
use serde_json::Value;
use std::path::Path;

impl McpServer {
    pub(crate) fn tool_agent_search(
        &self,
        args: AgentSearchArgs,
        mode: AgentSearchMode,
    ) -> anyhow::Result<String> {
        let AgentSearchArgs {
            query,
            root,
            limit,
            resend_seen,
            budget_tokens,
            file_filter,
            lang,
            preview,
        } = args;
        let (response, indexed_files) =
            match self.run_search(root.clone(), limit, file_filter, lang, mode, &query) {
                Ok(pair) => pair,
                Err(error) if Self::is_empty_index_error(&error) => {
                    let miss = to_compact_miss_json(
                        &query,
                        &self.diagnose_miss(&root, &query, mode, Some(0)),
                    );
                    return Ok(serde_json::to_string(&miss)?);
                }
                Err(error) => return Err(error),
            };
        // 6a3i: a miss is the cheapest response we can send, and the one where
        // a vague answer costs the most in speculative agent retries.
        if response.hits.is_empty() {
            let miss = to_compact_miss_json(
                &query,
                &self.diagnose_miss(&root, &query, mode, indexed_files),
            );
            return Ok(serde_json::to_string(&miss)?);
        }
        // kxmc: compact key-free envelope, minified.
        let mut envelope = render_compact_envelope(&response, preview, budget_tokens);
        if !resend_seen && preview != PreviewMode::None {
            self.elide_seen_snippets(&root, &mut envelope);
        }
        Ok(serde_json::to_string(&envelope)?)
    }

    /// Replace snippets this session already sent for the same id and unchanged
    /// content (v972).
    ///
    /// Iterative agent search re-runs overlapping queries constantly, and the
    /// model pays again for bytes it already has. Keying on a content hash (not
    /// just the id) means an edited file re-sends in full, and the map is
    /// cleared on `index_repo` so an elision never spans index generations.
    fn elide_seen_snippets(&self, root: &Path, envelope: &mut Value) {
        let Some(hits) = envelope.get_mut("h").and_then(Value::as_array_mut) else {
            return;
        };
        let mut seen = Self::lock_or_recover(&self.emitted_snippets, |seen| seen.clear());
        let mut elided = 0_usize;
        for hit in hits {
            let Some(row) = hit.as_array_mut() else {
                continue;
            };
            let (Some(id), Some(snippet)) = (
                row.first().and_then(Value::as_str).map(str::to_owned),
                row.get(4).and_then(Value::as_str).map(str::to_owned),
            ) else {
                continue;
            };
            if snippet.is_empty() || snippet == ELIDED_SNIPPET {
                continue;
            }
            let digest = fnv1a64(snippet.as_bytes());
            let key = (root.to_path_buf(), id);
            if seen.get(&key) == Some(&digest) {
                row[4] = Value::String(ELIDED_SNIPPET.to_owned());
                elided += 1;
                continue;
            }
            if seen.len() >= MAX_EMITTED_SNIPPETS && !seen.contains_key(&key) {
                continue;
            }
            seen.insert(key, digest);
        }
        if elided > 0 {
            // Volatile accounting, so it sorts with the `z*` tail (9q0l).
            envelope["ze"] = Value::from(elided);
        }
    }

    /// Classify a zero-hit search. `indexed_files` is the probe already taken
    /// on the search path (`None` = not known; `Some(0)` = empty index).
    fn diagnose_miss(
        &self,
        root: &Path,
        query: &str,
        mode: AgentSearchMode,
        indexed_files: Option<usize>,
    ) -> MissContext {
        let indexed_files = indexed_files.or_else(|| {
            IndexStore::open_readonly(root, self.index_path.as_deref())
                .ok()
                .and_then(|store| store.has_indexed_files().ok())
                .map(usize::from)
        });
        let channel = match mode {
            AgentSearchMode::Hybrid => "hybrid",
            AgentSearchMode::Keyword => "lexical",
            AgentSearchMode::Ast => "structural",
            AgentSearchMode::Semantic => "semantic",
        };
        // Semantic search over an index with no semantic chunks is a channel
        // that could not run, not an honest absence of matches.
        let unavailable = if matches!(mode, AgentSearchMode::Semantic) && !self.use_embed {
            vec!["semantic".to_owned()]
        } else {
            Vec::new()
        };
        let mut scope = Vec::new();
        if root != self.root {
            if let Some(relative) = root.strip_prefix(&self.root).ok().map(Path::to_path_buf) {
                if !relative.as_os_str().is_empty() {
                    scope.push(("root".to_owned(), relative.display().to_string()));
                }
            }
        }
        MissContext {
            tried: vec![channel.to_owned()],
            unavailable,
            scope,
            indexed_files,
            pattern_miss: ast_sgrep_plugins::is_bodyless_fn_args_pattern(query),
        }
    }
}
