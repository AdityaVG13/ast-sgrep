use super::super::embed_support::{
    embed_cache_cap, embed_chunks, evict_embed_cache, insert_embed_cache_entries,
    requested_model_identity, structure_fingerprint, touch_embed_cache_entries, EmbeddedChunk,
    EmbeddedChunks,
};
use super::super::sql::{delete_file_children, delete_file_lines, query_cached_map};
use super::{CallerRow, ImportRow, IndexStore, RefreshLinesInput, SymbolRow, UpsertFileInput};
use crate::Result;
use ast_sgrep_lang::PatternNode;
use rusqlite::params;
use std::collections::HashMap;

impl IndexStore {
    pub fn upsert_file(&self, input: UpsertFileInput<'_>) -> Result<i64> {
        let requested_identity = input
            .embed_semantic
            .then(|| requested_model_identity(input.embed_backend));
        let struct_fp = structure_fingerprint(
            input.symbols,
            input.callers,
            input.imports,
            input.pattern_nodes,
            input.semantic_chunks,
            requested_identity.as_deref(),
        );
        let struct_key = format!("struct:{}", input.rel_path);
        let embed_identity_matches = !input.embed_semantic
            || self.get_meta("embed_model")?.as_deref() == requested_identity.as_deref();
        if let Some(file_id) = self.file_id(input.rel_path)? {
            if embed_identity_matches
                && self.get_meta(&struct_key)?.as_deref() == Some(struct_fp.as_str())
            {
                return self.with_file_tx(|| {
                    self.refresh_lines_only(RefreshLinesInput {
                        file_id,
                        language: input.language,
                        mtime_secs: input.mtime_secs,
                        mtime_nanos: input.mtime_nanos,
                        content_hash: input.content_hash,
                        lines: input.lines,
                        eol: input.eol,
                        rel_path: input.rel_path,
                    })
                });
            }
        }
        let emb = embed_chunks(
            &self.conn,
            input.semantic_chunks,
            input.embed_semantic,
            input.embed_backend,
        )?;
        let (cache_hits, cache_misses) = (
            emb.cache_hits.len(),
            emb.chunks.len().saturating_sub(emb.cache_hits.len()),
        );
        self.with_file_tx(|| {
            let id = self.upsert_file_inner(input, &emb.chunks, &struct_key, &struct_fp)?;
            self.persist_embed_cache_side_effects(&emb, cache_hits, cache_misses)?;
            Ok(id)
        })
    }
    fn persist_embed_cache_side_effects(
        &self,
        emb: &EmbeddedChunks,
        cache_hits: usize,
        cache_misses: usize,
    ) -> Result<()> {
        if !emb.cache_entries.is_empty() {
            if let Err(e) =
                insert_embed_cache_entries(&self.conn, &self.cache_seq, &emb.cache_entries)
            {
                eprintln!("[asgrep] warning: failed to write embedding cache: {e}");
            }
            if let Err(e) = evict_embed_cache(&self.conn, embed_cache_cap()) {
                eprintln!("[asgrep] warning: failed to evict embedding cache: {e}");
            }
        } else if !emb.cache_hits.is_empty() {
            let hits: Vec<_> = emb
                .cache_hits
                .iter()
                .map(|h| (h.chunk_hash.clone(), h.model_id.clone()))
                .collect();
            if let Err(e) = touch_embed_cache_entries(&self.conn, &self.cache_seq, &hits) {
                eprintln!("[asgrep] warning: failed to touch embedding cache: {e}");
            }
        }
        self.bump_meta_u64("embed_cache_hits", cache_hits)?;
        self.bump_meta_u64("embed_cache_misses", cache_misses)?;
        Ok(())
    }
    /// Lines/FTS only when structure fingerprint matches (append / truncate / full rewrite).
    pub fn refresh_lines_only(&self, input: RefreshLinesInput<'_>) -> Result<i64> {
        let RefreshLinesInput {
            file_id,
            language: lang,
            mtime_secs,
            mtime_nanos,
            content_hash: hash,
            lines,
            eol,
            rel_path,
        } = input;
        let existing: Vec<(u32, String)> = {
            let mut stmt = self.conn.prepare_cached(
                "SELECT line_no, content FROM lines WHERE file_id = ?1 ORDER BY line_no",
            )?;
            let rows = stmt.query_map(params![file_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let common = existing
            .iter()
            .zip(lines.iter())
            .take_while(|(a, b)| a.1 == b.1)
            .count();
        if common == existing.len() && lines.len() >= existing.len() {
            // Append trailing lines — must keep lines_fts AND lines_trigram in sync
            // (literal BMH path uses trigram when indexed lines ≥ 1000).
            let extra = &lines[common..];
            if !extra.is_empty() {
                self.insert_lines(file_id, extra)?;
            }
        } else if common == lines.len() && existing.len() > lines.len() {
            // Truncate trailing lines: drop FTS + content=trigram rowids before lines.
            delete_file_lines(&self.conn, file_id, Some(lines.len() as u32 + 1))?;
        } else {
            delete_file_lines(&self.conn, file_id, None)?;
            self.insert_lines(file_id, lines)?;
        }
        self.conn
            .prepare_cached(
                "UPDATE files SET language=?1, mtime_secs=?2, mtime_nanos=?3, content_hash=?4 WHERE id=?5",
            )?
            .execute(params![lang, mtime_secs, mtime_nanos, hash, file_id])?;
        self.set_meta(&format!("eol:{rel_path}"), eol)?;
        self.bump_index_data_version()?;
        self.invalidate_lexicon()?;
        Ok(file_id)
    }
    fn upsert_file_inner(
        &self,
        input: UpsertFileInput<'_>,
        emb: &[EmbeddedChunk],
        struct_key: &str,
        struct_fp: &str,
    ) -> Result<i64> {
        let file_id = self.upsert_file_row(
            input.rel_path,
            input.language,
            input.mtime_secs,
            input.mtime_nanos,
            input.content_hash,
        )?;
        self.insert_lines(file_id, input.lines)?;
        self.set_meta(&format!("eol:{}", input.rel_path), input.eol)?;
        let symbol_ids = self.insert_symbols(file_id, input.symbols)?;
        self.insert_semantic_chunks(
            file_id,
            input.symbols,
            &symbol_ids,
            input.semantic_chunks,
            emb,
            input.embed_backend,
        )?;
        self.insert_callers(file_id, input.callers)?;
        self.insert_imports(file_id, input.imports)?;
        self.insert_pattern_nodes(file_id, input.pattern_nodes)?;
        self.set_meta(struct_key, struct_fp)?;
        crate::semantic_ann::mark_semantic_ivf_stale(self)?;
        self.bump_index_data_version()?;
        self.invalidate_lexicon()?;
        Ok(file_id)
    }
    fn upsert_file_row(
        &self,
        path: &str,
        lang: Option<&str>,
        mtime_secs: i64,
        mtime_nanos: u32,
        hash: &str,
    ) -> Result<i64> {
        if let Some(id) = self.file_id(path)? {
            delete_file_children(&self.conn, id)?;
            self.conn.prepare_cached(
                "UPDATE files SET language=?1, mtime_secs=?2, mtime_nanos=?3, content_hash=?4 WHERE id=?5",
            )?.execute(params![lang, mtime_secs, mtime_nanos, hash, id])?;
            return Ok(id);
        }
        self.conn.prepare_cached( "INSERT INTO files(path, language, mtime_secs, mtime_nanos, content_hash) VALUES(?1,?2,?3,?4,?5)",
        )?.execute(params![path, lang, mtime_secs, mtime_nanos, hash])?;
        Ok(self.conn.last_insert_rowid())
    }
    /// Prepare a cached INSERT and bind each row. SQL text must remain byte-identical.
    fn insert_each<T>(
        &self,
        sql: &'static str,
        rows: &[T],
        mut bind: impl FnMut(&mut rusqlite::CachedStatement<'_>, &T) -> rusqlite::Result<()>,
    ) -> Result<()> {
        let mut st = self.conn.prepare_cached(sql)?;
        for row in rows {
            bind(&mut st, row)?;
        }
        Ok(())
    }

    fn insert_lines(&self, file_id: i64, lines: &[(u32, String)]) -> Result<()> {
        let mut ls = self
            .conn
            .prepare_cached("INSERT INTO lines(file_id, line_no, content) VALUES(?1,?2,?3)")?;
        let mut fts = self.conn.prepare_cached(
            "INSERT INTO lines_fts(rowid, content, file_id, line_no) VALUES(?1,?2,?3,?4)",
        )?;
        // vvpk: the same line also lands in the unstemmed code field.
        let mut code_fts = self.conn.prepare_cached(
            "INSERT INTO lines_code_fts(rowid, content, file_id, line_no) VALUES(?1,?2,?3,?4)",
        )?;
        let mut tri = self
            .conn
            .prepare_cached("INSERT INTO lines_trigram(rowid, content) VALUES(?1,?2)")?;
        for (no, content) in lines {
            ls.execute(params![file_id, no, content])?;
            let rid = self.conn.last_insert_rowid();
            fts.execute(params![rid, content, file_id, no])?;
            code_fts.execute(params![rid, content, file_id, no])?;
            tri.execute(params![rid, content])?;
        }
        Ok(())
    }

    fn insert_symbols(&self, file_id: i64, symbols: &[SymbolRow]) -> Result<Vec<i64>> {
        let mut ids = Vec::with_capacity(symbols.len());
        let mut st = self.conn.prepare_cached(
            "INSERT INTO symbols(file_id, name, kind, line_start, line_end, byte_start, byte_end) VALUES(?1,?2,?3,?4,?5,?6,?7)", )?;
        for s in symbols {
            st.execute(params![
                file_id,
                s.name,
                s.kind,
                s.line_start,
                s.line_end,
                s.byte_start as i64,
                s.byte_end as i64
            ])?;
            ids.push(self.conn.last_insert_rowid());
        }
        Ok(ids)
    }
    fn insert_semantic_chunks(
        &self,
        file_id: i64,
        symbols: &[SymbolRow],
        symbol_ids: &[i64],
        chunks: &[crate::semantic_chunk::SemanticChunkInput],
        emb: &[EmbeddedChunk],
        preference: ast_sgrep_embed::EmbedPreference,
    ) -> Result<()> {
        if emb.is_empty() {
            // A re-upsert of an EXISTING file reaches here AFTER upsert_file_row's
            // delete_file_children already removed its old semantic_chunks. Bump so
            // SemanticCache + IVF fingerprint detect the mutation (bead ast-sgrep-44a4).
            // Benign over-bump when the file never had chunks (new file, no chunks):
            // an extra cache miss, never a stale hit.
            self.bump_semantic_data_version()?;
            return Ok(());
        }
        if emb.len() != chunks.len() {
            return Err(crate::StoreError::Other(format!(
                "embedding result count {} does not match semantic child count {}",
                emb.len(),
                chunks.len()
            )));
        }
        let first = &emb[0];
        if emb
            .iter()
            .any(|entry| entry.backend != first.backend || entry.dim != first.dim)
        {
            return Err(crate::StoreError::Other(
                "embedding provider returned mixed backend or dimension identities".into(),
            ));
        }
        let model = ast_sgrep_embed::configured_backend_model_id(first.backend, first.dim)
            .ok_or_else(|| {
                crate::StoreError::Other(format!(
                    "resolved {:?} embedding backend has no configured model identity",
                    first.backend
                ))
            })?;
        let (siblings, stored_backend, stored_model, stored_dim): (
            bool,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = self.conn.query_row(
            "SELECT
               EXISTS(SELECT 1 FROM semantic_chunks WHERE file_id != ?1),
               (SELECT value FROM meta WHERE key = 'embed_backend'),
               (SELECT value FROM meta WHERE key = 'embed_model'),
               (SELECT value FROM meta WHERE key = 'embed_dim')",
            params![file_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        if siblings {
            let backend_matches = stored_backend.as_deref().is_some_and(|stored| {
                ast_sgrep_embed::EmbedBackendKind::parse(stored) == Some(first.backend)
            });
            if !backend_matches
                || stored_dim.as_deref().and_then(|dim| dim.parse().ok()) != Some(first.dim)
                || stored_model.as_deref() != Some(model.as_str())
            {
                return Err(crate::StoreError::Other(format!(
                    "resolved embedding identity {:?}/{model}/{} does not match existing repository vectors; run `asgrep reindex`",
                    first.backend, first.dim
                )));
            }
        }
        let name_to_id: HashMap<String, i64> = symbols
            .iter()
            .zip(symbol_ids)
            .map(|(s, id)| (format!("{}:{}", s.name, s.line_start), *id))
            .collect();
        let mut st = self.conn.prepare_cached(
            "INSERT INTO semantic_chunks(file_id, symbol_id, chunk_kind, line_start, line_end, symbol_name, text, vector) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", )?;
        for (c, e) in chunks.iter().zip(emb.iter()) {
            let sid = name_to_id
                .get(&format!("{}:{}", c.symbol_name, c.line_start))
                .copied();
            st.execute(params![
                file_id,
                sid,
                if c.kind == "file" { "file" } else { "symbol" },
                c.line_start,
                c.line_end,
                c.symbol_name,
                c.excerpt,
                e.vector_bytes
            ])?;
        }
        self.persist_embed_metadata(Some(first.dim), Some(first.backend), preference)
    }
    fn persist_embed_metadata(
        &self,
        dim: Option<usize>,
        kind: Option<ast_sgrep_embed::EmbedBackendKind>,
        preference: ast_sgrep_embed::EmbedPreference,
    ) -> Result<()> {
        if let Some(k) = kind {
            // embed_backend always stores the RESOLVED kind (search loads
            // vectors by it). The build-time preference lives separately so
            // identity checks can distinguish an Auto build (reopen no-ops
            // under Auto) from an explicit one (28vo exactness) — parity.
            // e2hc.13: a partial (single-file) update must not promote a
            // legacy v1 store (meta == "semantic") to v2 while sibling chunks
            // remain v1 — only a full index_all rewrite may.
            if self.get_meta("embed_backend")?.as_deref() != Some("semantic") {
                self.set_meta("embed_backend", k.as_meta_str())?;
                if preference == ast_sgrep_embed::EmbedPreference::Auto {
                    self.set_meta("embed_backend_pref", "auto")?;
                } else {
                    self.delete_meta("embed_backend_pref")?;
                }
            }
            let model = dim.and_then(|dim| ast_sgrep_embed::configured_backend_model_id(k, dim));
            if let Some(model) = model {
                self.set_meta("embed_model", &model)?;
            } else {
                self.delete_meta("embed_model")?;
            }
        }
        if let Some(d) = dim {
            self.set_meta("embed_dim", &d.to_string())?;
        }
        // Bump semantic_data_version on every chunk insertion so SemanticCache and
        // the IVF fingerprint detect delete+re-add collisions (bead ast-sgrep-44a4).
        self.bump_semantic_data_version()?;
        Ok(())
    }
    fn insert_callers(&self, file_id: i64, callers: &[CallerRow]) -> Result<()> {
        self.insert_each(
            "INSERT INTO callers(file_id, caller, callee, line_no, byte_start, byte_end) VALUES(?1,?2,?3,?4,?5,?6)",
            callers,
            |st, c| {
                st.execute(params![
                    file_id,
                    c.caller,
                    c.callee,
                    c.line_no,
                    c.byte_start as i64,
                    c.byte_end as i64
                ])?;
                Ok(())
            },
        )
    }
    fn insert_pattern_nodes(&self, file_id: i64, nodes: &[PatternNode]) -> Result<()> {
        self.insert_each(
            "INSERT INTO pattern_nodes(file_id, signature, line_start, line_end, excerpt) VALUES(?1,?2,?3,?4,?5)",
            nodes,
            |st, n| {
                st.execute(params![
                    file_id,
                    n.signature,
                    n.line_start,
                    n.line_end,
                    n.excerpt
                ])?;
                Ok(())
            },
        )
    }
    fn insert_imports(&self, file_id: i64, imports: &[ImportRow]) -> Result<()> {
        self.insert_each(
            "INSERT INTO imports(file_id, module_path, line_no) VALUES(?1,?2,?3)",
            imports,
            |st, i| {
                st.execute(params![file_id, i.module_path, i.line_no])?;
                Ok(())
            },
        )
    }
    pub fn remove_file(&self, rel_path: &str) -> Result<()> {
        self.with_file_tx(|| {
            let Some(id) = self.file_id(rel_path)? else {
                return Ok(());
            };
            delete_file_children(&self.conn, id)?;
            self.conn
                .execute("DELETE FROM files WHERE id = ?1", params![id])?;
            self.delete_meta(&format!("eol:{rel_path}"))?;
            self.delete_meta(&format!("body:{rel_path}"))?;
            self.delete_meta(&format!("struct:{rel_path}"))?;
            crate::semantic_ann::mark_semantic_ivf_stale(self)?;
            self.bump_index_data_version()?;
            self.bump_semantic_data_version()?;
            self.invalidate_lexicon()?;
            Ok(())
        })
    }
    pub(crate) fn remove_files_with_prefix(&self, prefix: &str) -> Result<usize> {
        let pattern = format!("{}*", super::super::sql::escape_glob_literal(prefix));
        let paths: Vec<String> = query_cached_map(
            &self.conn,
            "SELECT path FROM files WHERE path GLOB ?1 ORDER BY path",
            params![pattern],
            |row| row.get(0),
        )?;
        if paths.is_empty() {
            return Ok(0);
        }
        self.with_file_tx(|| {
            for path in &paths {
                self.remove_file(path)?;
            }
            Ok(paths.len())
        })
    }
}
