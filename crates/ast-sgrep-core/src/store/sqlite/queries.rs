use super::super::embed_support::{embed_cache_cap, read_sym_loc};
use super::super::sql::{
    append_lang_filter, calls_matching, count_star, emb_vec, lang_and_clause, like_terms_filter,
    optional_row, query_cached_map, query_limit_map, query_map_rows, read_legacy_emb, read_sem_row,
    where_clause,
};
use super::{
    sql_usize_from_byte_offset, CallEvidenceRow, CallRow, ImportQueryRow, ImportRow, IndexStore,
    IndexedLineRow, PatternNodeRow, SemanticChunkStats, SymbolLocationRow, SymbolRow,
    IMPORT_SELECT, SYM_LOC,
};
use crate::{IndexStatus, Result};
use rusqlite::types::{Type, ValueRef};
use rusqlite::{params, ToSql};
use std::collections::HashMap;
use std::sync::Arc;

fn read_field_vector_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(i64, crate::semantic_chunk::SemanticFieldVectors)> {
    Ok((
        row.get(0)?,
        crate::semantic_chunk::SemanticFieldVectors {
            name: row.get(1)?,
            docs: row.get(2)?,
            body: row.get(3)?,
            graph: row.get(4)?,
            tests_examples: row.get(5)?,
        },
    ))
}

impl IndexStore {
    pub fn file_hash(&self, rel_path: &str) -> Result<Option<String>> {
        optional_row(
            &self.conn,
            "SELECT content_hash FROM files WHERE path = ?1",
            &[&rel_path],
            |r| r.get(0),
        )
    }

    pub(crate) fn file_identities(&self) -> Result<HashMap<String, super::FileIdentity>> {
        let rows = query_cached_map(
            &self.conn,
            "SELECT path, mtime_secs, mtime_nanos, content_hash FROM files",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    super::FileIdentity {
                        mtime_secs: row.get(1)?,
                        mtime_nanos: row.get(2)?,
                        content_hash: row.get(3)?,
                    },
                ))
            },
        )?;
        Ok(rows.into_iter().collect())
    }
    pub fn all_file_paths(&self) -> Result<Vec<String>> {
        query_cached_map(
            &self.conn,
            "SELECT path FROM files ORDER BY path",
            [],
            |r| r.get(0),
        )
    }
    pub(crate) fn has_file_with_prefix(&self, prefix: &str) -> Result<bool> {
        let pattern = format!("{}*", super::super::sql::escape_glob_literal(prefix));
        Ok(self
            .conn
            .prepare_cached("SELECT 1 FROM files WHERE path GLOB ?1 LIMIT 1")?
            .exists(params![pattern])?)
    }
    pub fn status(&self) -> Result<IndexStatus> {
        let (fc, lc, sc, cc, ic, sec): (usize, usize, usize, usize, usize, usize) = self.conn.query_row(
            "SELECT (SELECT COUNT(*) FROM files),(SELECT COUNT(*) FROM lines),(SELECT COUNT(*) FROM symbols),\
             (SELECT COUNT(*) FROM callers),(SELECT COUNT(*) FROM imports),(SELECT COUNT(*) FROM semantic_chunks)",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)), )?;
        Ok(IndexStatus {
            root: self.root.display().to_string(),
            index_path: self.db_path.display().to_string(),
            file_count: fc,
            line_count: lc,
            symbol_count: sc,
            caller_count: cc,
            import_count: ic,
            semantic_chunk_count: sec,
            embed_backend: self.get_meta("embed_backend")?,
            embed_dim: self.get_meta("embed_dim")?.and_then(|d| d.parse().ok()),
            embed_cache_entries: count_star(&self.conn, "embed_cache")?,
            embed_cache_capacity: embed_cache_cap(),
            embed_cache_hits: self.meta_u64("embed_cache_hits")?,
            embed_cache_misses: self.meta_u64("embed_cache_misses")?,
            semantic_ivf_present: crate::semantic_ivf::semantic_ivf_path(&self.db_path).exists(),
            durability: self.durability.as_str().to_owned(),
            writer_generation: crate::store::read_writer_generation(
                &self.root,
                Some(&self.db_path),
            ),
        })
    }
    pub fn indexed_line_count(&self) -> Result<usize> {
        count_star(&self.conn, "lines")
    }
    /// True when indexed lines ≥ threshold (LIMIT probe; avoids full COUNT).
    pub fn indexed_line_count_at_least(&self, threshold: usize) -> Result<bool> {
        super::super::sql::at_least_rows(&self.conn, "lines", threshold)
    }
    pub fn all_indexed_lines(&self) -> Result<Vec<IndexedLineRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.path, l.line_no, l.content, f.language FROM lines l JOIN files f ON f.id = l.file_id ORDER BY f.path, l.line_no")?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        let mut last_path: Option<Arc<str>> = None;
        let mut last_lang: Option<Arc<str>> = None;
        while let Some(row) = rows.next()? {
            let path = row.get_ref(0)?.as_str().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(e))
            })?;
            if last_path.as_deref() != Some(path) {
                last_path = Some(Arc::from(path));
                last_lang = match row.get_ref(3)? {
                    ValueRef::Null => None,
                    v => Some(Arc::from(v.as_str().map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(e))
                    })?)),
                };
            }
            out.push((
                Arc::clone(last_path.as_ref().expect("path")),
                row.get(1)?,
                row.get(2)?,
                last_lang.clone(),
            ));
        }
        Ok(out)
    }
    pub(crate) fn indexed_excerpt_in_range(
        &self,
        path: &str,
        line_start: u32,
        line_end: u32,
    ) -> Result<String> {
        let mut stmt = self.conn.prepare_cached(
            // COALESCE both columns: SQLite substr() over an empty BLOB
            // (a blank line) yields NULL, which must read as an empty
            // excerpt line, not an InvalidColumnType error (ast-sgrep-5vur).
            "SELECT COALESCE(CAST(substr(CAST(l.content AS BLOB), 1, ?4) AS BLOB), x''),
                    COALESCE(length(CAST(l.content AS BLOB)), 0)
             FROM lines l JOIN files f ON f.id = l.file_id
             WHERE f.path = ?1 AND l.line_no >= ?2 AND l.line_no <= ?3
             ORDER BY l.line_no",
        )?;
        let max = crate::limits::MAX_SEARCH_HIT_EXCERPT_BYTES;
        let prefix_limit = max.saturating_add(1);
        let mut rows = stmt.query(params![path, line_start, line_end, prefix_limit])?;
        let mut excerpt = String::new();
        while let Some(row) = rows.next()? {
            let prefix = row.get::<_, Vec<u8>>(0)?;
            let line_bytes = row.get::<_, i64>(1)?;
            let line_truncated = usize::try_from(line_bytes)
                .map(|len| len > prefix.len())
                .unwrap_or(true);
            let valid_len = match std::str::from_utf8(&prefix) {
                Ok(_) => prefix.len(),
                Err(error) if line_truncated && error.error_len().is_none() => error.valid_up_to(),
                Err(error) => {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        0,
                        Type::Text,
                        Box::new(error),
                    )
                    .into())
                }
            };
            let line = std::str::from_utf8(&prefix[..valid_len]).expect("validated prefix");
            let separator = usize::from(!excerpt.is_empty());
            if excerpt
                .len()
                .saturating_add(separator)
                .saturating_add(line.len())
                <= max
                && !line_truncated
            {
                if separator == 1 {
                    excerpt.push('\n');
                }
                excerpt.push_str(line);
                continue;
            }
            const MARKER: &str = "…";
            let content_limit = max.saturating_sub(MARKER.len());
            if excerpt.len() > content_limit {
                let mut end = content_limit;
                while end > 0 && !excerpt.is_char_boundary(end) {
                    end -= 1;
                }
                excerpt.truncate(end);
            }
            let mut allowance = content_limit.saturating_sub(excerpt.len());
            if separator == 1 && allowance > 0 {
                excerpt.push('\n');
                allowance -= 1;
            }
            let mut end = allowance.min(line.len());
            while end > 0 && !line.is_char_boundary(end) {
                end -= 1;
            }
            excerpt.push_str(&line[..end]);
            excerpt.push_str(MARKER);
            break;
        }
        Ok(excerpt)
    }
    pub fn semantic_chunk_max_id(&self) -> Result<Option<i64>> {
        optional_row(
            &self.conn,
            "SELECT MAX(id) FROM semantic_chunks",
            &[],
            |r| r.get::<_, Option<i64>>(0),
        )
        .map(Option::flatten)
    }
    /// gauntlet-r4 (E1): true when BOTH persistent semantic sources are
    /// globally empty. Each EXISTS short-circuits on the first row, so the
    /// probe costs microseconds on non-empty stores and answers instantly on
    /// empty ones. Callers use it to skip per-file query loops that provably
    /// return nothing.
    pub fn semantic_sources_empty(&self) -> Result<bool> {
        let empty: i64 = self.conn.query_row(
            "SELECT CASE WHEN EXISTS(SELECT 1 FROM semantic_chunks LIMIT 1) \
             OR EXISTS(SELECT 1 FROM embeddings LIMIT 1) THEN 0 ELSE 1 END",
            [],
            |r| r.get(0),
        )?;
        Ok(empty != 0)
    }
    pub fn semantic_chunk_stats(&self, lang: Option<&str>) -> Result<SemanticChunkStats> {
        let max_id = self.semantic_chunk_max_id()?.unwrap_or(0);
        let (count, dim): (usize, usize) = if let Some(l) = lang {
            self.conn.query_row(
                "SELECT COUNT(*), COALESCE(MAX(length(sc.vector)/4),0) FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE f.language=?1",
                params![l], |r| Ok((r.get(0)?, r.get(1)?)), )?
        } else {
            self.conn.query_row(
                "SELECT COUNT(*), COALESCE(MAX(length(vector)/4),0) FROM semantic_chunks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
        };
        Ok(SemanticChunkStats { count, max_id, dim })
    }
    pub fn semantic_chunk_ids(&self, lang: Option<&str>) -> Result<Vec<i64>> {
        let (sql, l) = if lang.is_some() {
            ("SELECT sc.id FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE f.language=?1 ORDER BY sc.id", lang)
        } else {
            ("SELECT id FROM semantic_chunks ORDER BY id", None)
        };
        query_map_rows(&self.conn, sql, l, |r| r.get(0))
    }
    pub fn semantic_chunks_by_ids(
        &self,
        ids: &[i64],
    ) -> Result<Vec<(i64, ast_sgrep_embed::SemanticChunkRow)>> {
        let mut out = Vec::with_capacity(ids.len());
        for batch in ids.chunks(500) {
            let ph = std::iter::repeat_n("?", batch.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT sc.id, f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text, sc.vector \
                 FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE sc.id IN ({ph})"
            );
            // gauntlet-r6 (I5a): prepare_cached — the 500-id bucket text is
            // stable across calls, so the statement parses once per process
            // instead of once per query per batch (the IVF candidate path
            // runs this loop on every cache-miss query).
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(batch.iter()), |r| {
                let id: i64 = r.get(0)?;
                // Fail closed on corrupt blobs (parity with read_sem_row / emb_vec).
                // unwrap_or_default() previously dropped corrupt IVF candidates as
                // empty vectors, silently skewing ANN re-rank results (pass3).
                let row = (
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    r.get(5)?,
                    emb_vec(r, 6)?,
                );
                Ok((id, row))
            })?;
            out.extend(rows.collect::<std::result::Result<Vec<_>, _>>()?);
        }
        Ok(out)
    }
    pub fn all_semantic_chunks(
        &self,
        lang: Option<&str>,
    ) -> Result<Vec<ast_sgrep_embed::SemanticChunkRow>> {
        let sql = format!(
            "SELECT f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text, sc.vector \
             FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE 1=1{} ORDER BY sc.id",
            lang_and_clause(lang)
        );
        query_map_rows(&self.conn, &sql, lang, read_sem_row)
    }

    /// Per-field embedding blobs for 7d5x.2.2 persist / 7d5x.3 weighting.
    pub fn semantic_chunk_field_vectors(
        &self,
    ) -> Result<Vec<(i64, crate::semantic_chunk::SemanticFieldVectors)>> {
        self.semantic_field_vectors_filtered(None)
    }

    pub fn semantic_field_vectors_filtered(
        &self,
        lang: Option<&str>,
    ) -> Result<Vec<(i64, crate::semantic_chunk::SemanticFieldVectors)>> {
        let sql = format!(
            "SELECT sc.id, sc.vector_name, sc.vector_docs, sc.vector_body, sc.vector_graph, sc.vector_tests_examples \
             FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE 1=1{} ORDER BY sc.id",
            lang_and_clause(lang)
        );
        query_map_rows(&self.conn, &sql, lang, read_field_vector_row)
    }

    pub fn semantic_field_vectors_by_ids(
        &self,
        ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, crate::semantic_chunk::SemanticFieldVectors>> {
        let mut out = std::collections::HashMap::with_capacity(ids.len());
        for batch in ids.chunks(500) {
            let placeholders = std::iter::repeat_n("?", batch.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT id, vector_name, vector_docs, vector_body, vector_graph, vector_tests_examples \
                 FROM semantic_chunks WHERE id IN ({placeholders})"
            );
            // I5a: same statement-cache rationale as semantic_chunks_by_ids.
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params_from_iter(batch.iter()),
                read_field_vector_row,
            )?;
            for row in rows {
                let (id, fields) = row?;
                out.insert(id, fields);
            }
        }
        Ok(out)
    }

    /// Walk sorted file paths and extend with per-path query results (stable path order).
    fn map_sorted_files<T>(
        files: &std::collections::HashSet<String>,
        mut query: impl FnMut(&str) -> Result<Vec<T>>,
    ) -> Result<Vec<T>> {
        let mut paths = files.iter().collect::<Vec<_>>();
        paths.sort_unstable();
        let mut out = Vec::new();
        for path in paths {
            out.extend(query(path)?);
        }
        Ok(out)
    }
    /// gauntlet-r6 (B1): shared batched replacement for the per-path loops in
    /// `semantic_chunks_for_files` / `semantic_field_vectors_for_files`. The
    /// loops emit, for each byte-sorted path, that path's rows in ascending
    /// `sc.id`; one `WHERE f.path IN (…) ORDER BY f.path, sc.id` produces the
    /// identical sequence (Rust String sort == SQLite BINARY collation on
    /// UTF-8). Placeholder count is quantized to a power of two ≥ 8 so
    /// `prepare_cached` sees stable statement text; padding uses an IMPOSSIBLE
    /// value ('' — no indexed path is empty) rather than repeating a real
    /// path, because duplicates would duplicate rows here. Empty requested
    /// sets return empty without touching SQL.
    fn semantic_rows_batched<T>(
        &self,
        files: &std::collections::HashSet<String>,
        lang: Option<&str>,
        select_cols: &str,
        map: fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let mut paths: Vec<String> = files.iter().cloned().collect();
        paths.sort_unstable();
        let n = paths.len();
        if n == 0 {
            return Ok(Vec::new());
        }
        // Round UP to the next power of two (>= 8). The previous
        // `(n - 1).next_power_of_two()` formula SHRANK the bucket when n was
        // already a power of two plus one (n=9 -> bucket 8), truncating the
        // placeholder list while all n paths were still bound — a guaranteed
        // "Wrong number of parameters" for exactly those file counts.
        let bucket = n.next_power_of_two().max(8);
        if bucket > n {
            paths.resize(bucket, String::new());
        }
        let placeholders = std::iter::repeat_n("?", bucket)
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT {select_cols} FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id \
             WHERE f.path IN ({placeholders}){} ORDER BY f.path, sc.id",
            if lang.is_some() {
                " AND f.language = ?"
            } else {
                ""
            }
        );
        let mut bind: Vec<&str> = paths.iter().map(String::as_str).collect();
        match lang {
            Some(language) => bind.push(language),
            None => {}
        }
        query_cached_map(
            &self.conn,
            &sql,
            rusqlite::params_from_iter(bind.iter()),
            map,
        )
    }
    pub(crate) fn semantic_chunks_for_files(
        &self,
        files: &std::collections::HashSet<String>,
        lang: Option<&str>,
    ) -> Result<Vec<ast_sgrep_embed::SemanticChunkRow>> {
        self.semantic_rows_batched(
            files,
            lang,
            "f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text, sc.vector",
            read_sem_row,
        )
    }
    pub(crate) fn semantic_field_vectors_for_files(
        &self,
        files: &std::collections::HashSet<String>,
        lang: Option<&str>,
    ) -> Result<Vec<crate::semantic_chunk::SemanticFieldVectors>> {
        let rows = self.semantic_rows_batched(
            files,
            lang,
            "sc.id, sc.vector_name, sc.vector_docs, sc.vector_body, sc.vector_graph, sc.vector_tests_examples",
            read_field_vector_row,
        )?;
        Ok(rows.into_iter().map(|(_, fields)| fields).collect())
    }
    pub(crate) fn legacy_embeddings_for_files(
        &self,
        files: &std::collections::HashSet<String>,
        lang: Option<&str>,
    ) -> Result<Vec<ast_sgrep_embed::SemanticChunkRow>> {
        Self::map_sorted_files(files, |path| match lang {
            Some(language) => query_cached_map(
                &self.conn,
                "SELECT f.path, l.line_no, l.content, sc.symbol_name, e.vector \
                 FROM embeddings e JOIN lines l ON l.file_id=e.file_id AND l.line_no=e.line_no \
                 JOIN files f ON f.id=e.file_id \
                 LEFT JOIN semantic_chunks sc ON sc.file_id=f.id AND sc.line_start=l.line_no \
                 WHERE f.path=?1 AND f.language=?2 ORDER BY l.line_no LIMIT 5000",
                params![path, language],
                read_legacy_emb,
            ),
            None => query_cached_map(
                &self.conn,
                "SELECT f.path, l.line_no, l.content, sc.symbol_name, e.vector \
                 FROM embeddings e JOIN lines l ON l.file_id=e.file_id AND l.line_no=e.line_no \
                 JOIN files f ON f.id=e.file_id \
                 LEFT JOIN semantic_chunks sc ON sc.file_id=f.id AND sc.line_start=l.line_no \
                 WHERE f.path=?1 ORDER BY l.line_no LIMIT 5000",
                params![path],
                read_legacy_emb,
            ),
        })
    }
    pub fn symbols_in_file(&self, rel_path: &str) -> Result<Vec<SymbolRow>> {
        query_cached_map(
            &self.conn,
            "SELECT s.name, s.kind, s.line_start, s.line_end, s.byte_start, s.byte_end \
             FROM symbols s JOIN files f ON f.id=s.file_id WHERE f.path=?1 ORDER BY s.line_start",
            params![rel_path],
            |r| {
                Ok(SymbolRow {
                    name: r.get(0)?,
                    kind: r.get(1)?,
                    line_start: r.get(2)?,
                    line_end: r.get(3)?,
                    byte_start: sql_usize_from_byte_offset(r, 4)?,
                    byte_end: sql_usize_from_byte_offset(r, 5)?,
                })
            },
        )
    }
    pub fn incoming_calls(&self, callee: &str) -> Result<Vec<CallRow>> {
        calls_matching(&self.conn, "callee", callee)
    }
    pub fn outgoing_calls(&self, caller: &str) -> Result<Vec<CallRow>> {
        calls_matching(&self.conn, "caller", caller)
    }
    pub(crate) fn outgoing_calls_with_scip(
        &self,
        caller: &str,
        limit: usize,
    ) -> Result<Vec<CallEvidenceRow>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        query_cached_map(
            &self.conn,
            "SELECT f.path, c.line_no, c.caller, c.callee,
                    EXISTS(SELECT 1 FROM scip_facts sf
                           WHERE sf.file_id=c.file_id AND sf.line_no=c.line_no
                             AND sf.name=c.callee AND sf.is_def=0) AS scip_exact
             FROM callers c JOIN files f ON f.id=c.file_id
             WHERE lower(c.caller)=lower(?1)
             ORDER BY scip_exact DESC, f.path, c.line_no, c.callee LIMIT ?2",
            params![caller, limit],
            |row| {
                Ok(CallEvidenceRow {
                    file: row.get(0)?,
                    line: row.get(1)?,
                    caller: row.get(2)?,
                    callee: row.get(3)?,
                    scip_exact: row.get(4)?,
                })
            },
        )
    }
    pub fn symbol_at_line(&self, path: &str, line: u32) -> Result<Option<SymbolLocationRow>> {
        optional_row(
            &self.conn, &format!("{SYM_LOC} WHERE f.path=?1 AND s.line_start<=?2 AND s.line_end>=?2 ORDER BY (s.line_end-s.line_start), s.line_start DESC, s.name LIMIT 1"),
            &[&path as &dyn ToSql, &line as &dyn ToSql], read_sym_loc,
        )
    }
    pub fn first_symbol_in_file(&self, path: &str) -> Result<Option<SymbolLocationRow>> {
        optional_row(
            &self.conn,
            &format!("{SYM_LOC} WHERE f.path=?1 ORDER BY s.line_start, s.line_end, s.name LIMIT 1"),
            &[&path],
            read_sym_loc,
        )
    }
    pub fn symbols_named(&self, name: &str, limit: usize) -> Result<Vec<SymbolLocationRow>> {
        query_cached_map(
            &self.conn,
            &format!(
                "{SYM_LOC} WHERE lower(s.name)=lower(?1) ORDER BY f.path, s.line_start, s.line_end LIMIT ?2"
            ),
            params![name, limit as i64],
            read_sym_loc,
        )
    }
    pub fn imports_from_file(&self, path: &str) -> Result<Vec<ImportRow>> {
        query_cached_map(
            &self.conn,
            "SELECT i.module_path, i.line_no FROM imports i JOIN files f ON f.id=i.file_id \
             WHERE f.path=?1 ORDER BY i.line_no, i.module_path",
            params![path],
            |r| {
                Ok(ImportRow {
                    module_path: r.get(0)?,
                    line_no: r.get(1)?,
                })
            },
        )
    }
    pub fn resolve_module_path(&self, from_file: &str, module: &str) -> Result<Vec<String>> {
        let lang = self.file_language(from_file)?;
        let cands = super::super::module_resolve::collect_module_candidates(
            from_file,
            module,
            lang.as_deref(),
        );
        let mut out = Vec::new();
        for c in cands {
            if self.file_exists(&c)? {
                out.push(c);
            }
        }
        Ok(out)
    }
    pub(crate) fn file_language(&self, path: &str) -> Result<Option<String>> {
        optional_row(
            &self.conn,
            "SELECT language FROM files WHERE path=?1",
            &[&path],
            |row| row.get::<_, Option<String>>(0),
        )
        .map(Option::flatten)
    }
    pub fn pattern_node_count(&self) -> Result<usize> {
        count_star(&self.conn, "pattern_nodes")
    }
    pub fn pattern_nodes_matching(
        &self,
        signature: &str,
        lang: Option<&str>,
    ) -> Result<Vec<PatternNodeRow>> {
        self.pattern_nodes_matching_inner(signature, lang, None)
    }
    /// Distinct paths holding at least one node with any of `signatures`.
    ///
    /// Narrows the native tree-sitter pass to files that can possibly contain
    /// a match (every native match is a node of the pattern's kind, hence
    /// indexed under one of these signatures). The native matcher still decides
    /// every hit, so over-broad candidates never change results.
    pub fn pattern_node_candidate_paths(
        &self,
        signatures: &[String],
        lang: Option<&str>,
    ) -> Result<std::collections::HashSet<String>> {
        let mut paths = std::collections::HashSet::new();
        for signature in signatures {
            for row in self.pattern_nodes_matching(signature, lang)? {
                paths.insert(row.path);
            }
        }
        Ok(paths)
    }
    pub(crate) fn pattern_nodes_matching_limited(
        &self,
        signature: &str,
        lang: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PatternNodeRow>> {
        self.pattern_nodes_matching_inner(signature, lang, Some(limit))
    }
    fn pattern_nodes_matching_inner(
        &self,
        signature: &str,
        lang: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<PatternNodeRow>> {
        let mut sql = String::from(
            "SELECT f.path, f.language, n.line_start, n.line_end, n.excerpt FROM pattern_nodes n JOIN files f ON f.id=n.file_id WHERE n.signature=?1",
        );
        if lang.is_some() {
            sql.push_str(" AND f.language=?2");
        }
        sql.push_str(" ORDER BY f.path, n.line_start");
        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        let map = |r: &rusqlite::Row<'_>| {
            Ok(PatternNodeRow {
                path: r.get(0)?,
                language: r.get(1)?,
                line_start: r.get(2)?,
                line_end: r.get(3)?,
                excerpt: r.get(4)?,
            })
        };
        match lang {
            Some(l) => query_cached_map(&self.conn, &sql, params![signature, l], map),
            None => query_cached_map(&self.conn, &sql, params![signature], map),
        }
    }
    pub fn file_text(&self, path: &str) -> Result<Option<String>> {
        let lines = self.file_lines(path)?;
        if lines.is_empty() {
            return Ok(None);
        }
        let sep = match self.get_meta(&format!("eol:{path}"))? {
            Some(v) if v == "crlf" => "\r\n",
            _ => "\n",
        };
        Ok(Some(
            lines
                .iter()
                .map(|(_, c)| c.as_str())
                .collect::<Vec<_>>()
                .join(sep),
        ))
    }
    pub fn file_lines(&self, path: &str) -> Result<Vec<(u32, String)>> {
        query_cached_map( &self.conn, "SELECT l.line_no, l.content FROM lines l JOIN files f ON f.id=l.file_id WHERE f.path=?1 ORDER BY l.line_no",
            params![path], |r| Ok((r.get(0)?, r.get(1)?)), )
    }
    pub fn line_content(&self, path: &str, line: u32) -> Result<Option<String>> {
        optional_row(
            &self.conn, "SELECT l.content FROM lines l JOIN files f ON f.id=l.file_id WHERE f.path=?1 AND l.line_no=?2",
            &[&path as &dyn ToSql, &line as &dyn ToSql], |r| r.get(0),
        )
    }
    pub fn query_imports(
        &self,
        module: Option<&str>,
        lang: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ImportQueryRow>> {
        let map = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?));
        let Some(m) = module.filter(|m| !m.is_empty()) else {
            let mut parts = Vec::new();
            let mut bind = Vec::new();
            append_lang_filter(&mut parts, &mut bind, lang);
            let w = where_clause(&parts);
            return query_limit_map(
                &self.conn,
                &format!("{IMPORT_SELECT}{w} LIMIT ?{}", bind.len() + 1),
                bind,
                limit,
                map,
            );
        };
        let (w, bind) = like_terms_filter("i.module_path", &[m.to_string()], lang);
        query_limit_map(
            &self.conn,
            &format!("{IMPORT_SELECT}{w} LIMIT ?{}", bind.len() + 1),
            bind,
            limit,
            map,
        )
    }
    pub fn all_legacy_embeddings(
        &self,
        lang: Option<&str>,
    ) -> Result<Vec<ast_sgrep_embed::SemanticChunkRow>> {
        let sql = format!(
            "SELECT f.path, l.line_no, l.content, sc.symbol_name, e.vector FROM embeddings e \
             JOIN lines l ON l.file_id=e.file_id AND l.line_no=e.line_no JOIN files f ON f.id=e.file_id \
             LEFT JOIN semantic_chunks sc ON sc.file_id=f.id AND sc.line_start=l.line_no WHERE 1=1{} LIMIT 5000",
            lang_and_clause(lang)
        );
        query_map_rows(&self.conn, &sql, lang, read_legacy_emb)
    }
    pub fn file_exists(&self, path: &str) -> Result<bool> {
        Ok(self
            .conn
            .prepare_cached("SELECT 1 FROM files WHERE path=?1")?
            .exists(params![path])?)
    }
}
