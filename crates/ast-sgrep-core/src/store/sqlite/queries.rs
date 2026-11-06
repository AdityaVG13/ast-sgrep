use super::super::embed_support::{embed_cache_cap, read_sym_loc};
use super::super::sql::{
    append_lang_filter, at_least_rows, calls_matching, calls_matching_limited, count_star, emb_vec,
    lang_and_clause, like_terms_filter, optional_row, query_cached_map, query_cached_map_capped,
    query_limit_map, query_map_rows, read_legacy_emb, read_sem_row, where_clause,
};
use super::{
    sql_usize_from_byte_offset, CallEvidenceRow, CallRow, ImportQueryRow, ImportRow, IndexStore,
    IndexedLineRow, PatternNodeRow, SemanticChunkStats, SymbolLocationRow, SymbolRow,
    IMPORT_SELECT, SYM_LOC,
};
use crate::{IndexStatus, Result};
use rusqlite::types::{Type, ValueRef};
use rusqlite::{params, ToSql};
use std::sync::Arc;

const FILE_STAT_SEED: u64 = 0xcbf2_9ce4_8422_2325;
const FILE_STAT_PRIME: u64 = 0x0000_0100_0000_01b3;

pub fn fold_file_stat_identity(fp: &mut u64, path: &str, mtime_secs: i64, mtime_nanos: u32) {
    let mut hash = FILE_STAT_SEED;
    for &byte in path.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FILE_STAT_PRIME);
    }
    *fp ^= hash ^ (mtime_secs as u64) ^ (u64::from(mtime_nanos).rotate_left(17));
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

    pub(crate) fn file_identities(&self) -> Result<Vec<(String, super::FileIdentity)>> {
        let mut rows = query_cached_map(
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
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(rows)
    }

    /// Order-independent fingerprint of indexed path + mtime identities.
    /// Matches the Code Mode ambient stat probe so an unchanged tree can skip
    /// the writer without a session-side memo.
    pub fn file_stat_fingerprint(&self) -> Result<u64> {
        let mut fp = 0u64;
        let mut stmt = self
            .conn
            .prepare_cached("SELECT path, mtime_secs, mtime_nanos FROM files")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            fold_file_stat_identity(&mut fp, &row.get::<_, String>(0)?, row.get(1)?, row.get(2)?);
        }
        Ok(fp)
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
    /// Fast non-emptiness probe for the open gate: LIMIT-1 instead of the six
    /// COUNT(*)s in `status()`. The gate only ever tested `file_count == 0`.
    pub fn has_indexed_files(&self) -> Result<bool> {
        at_least_rows(&self.conn, "files", 1)
    }
    /// True when indexed lines ≥ threshold. Bundle header counts avoid the
    /// COUNT/LIMIT probe; the SQL fallback (legacy/gate control) queries once
    /// and does not memoize the answer.
    pub fn indexed_line_count_at_least(&self, threshold: usize) -> Result<bool> {
        if let Some(bundle) = self.read_bundle(None)? {
            return Ok(bundle.line_count() >= threshold);
        }
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
        if crate::store::read_bundle::bundle_enabled() {
            if let Ok(Some(corpus)) = self.line_corpus() {
                if let Some(excerpt) = corpus.excerpt(path, line_start, line_end) {
                    return Ok(excerpt);
                }
            }
        }
        let mut stmt = self.conn.prepare_cached(
            // COALESCE both columns: SQLite substr() over an empty BLOB
            // (a blank line) yields NULL, which must read as an empty
            // excerpt line, not an InvalidColumnType error.
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
    pub(crate) fn incoming_calls_capped(&self, callee: &str, limit: usize) -> Result<Vec<CallRow>> {
        calls_matching_limited(&self.conn, "callee", callee, Some(limit))
    }
    pub(crate) fn outgoing_calls_capped(&self, caller: &str, limit: usize) -> Result<Vec<CallRow>> {
        calls_matching_limited(&self.conn, "caller", caller, Some(limit))
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
        self.imports_from_file_limited(path, None)
    }
    pub(crate) fn imports_from_file_capped(
        &self,
        path: &str,
        limit: usize,
    ) -> Result<Vec<ImportRow>> {
        self.imports_from_file_limited(path, Some(limit))
    }
    fn imports_from_file_limited(
        &self,
        path: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ImportRow>> {
        query_cached_map_capped(
            &self.conn,
            "SELECT i.module_path, i.line_no FROM imports i JOIN files f ON f.id=i.file_id \
             WHERE f.path=?1 ORDER BY i.line_no, i.module_path",
            params![path],
            limit,
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
        if let Ok(Some(bundle)) = self.read_bundle(None) {
            if let Some(n) = crate::store::bundle_graph::pattern_row_count(&bundle) {
                return Ok(n);
            }
        }
        count_star(&self.conn, "pattern_nodes")
    }
    /// Does any indexed symbol carry this exact name? Ground-truth
    /// check for single-term hybrid queries that `classify` reads as
    /// Conceptual but that name a real identifier in this repository.
    pub fn has_symbol_named(&self, name: &str) -> Result<bool> {
        if let Some(bundle) = self.read_bundle(None)? {
            if crate::store::bundle_graph::symbols_section_present(&bundle) {
                return Ok(!crate::store::bundle_graph::defs_exact(&bundle, name, None, 1)
                    .is_empty());
            }
        }
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM symbols WHERE name = ?1 LIMIT 1",
                params![name],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        Ok(found.is_some())
    }
    /// Does the index hold any file whose extraction walk hit the depth
    /// budget (optionally scoped to one language)? Such files' index
    /// rows are incomplete, so the cached pattern lane must not serve them as
    /// authoritative and falls back to the native walk instead.
    pub fn has_depth_truncated_files(&self, lang: Option<&str>) -> Result<bool> {
        let sql = "SELECT 1 FROM files WHERE depth_truncated = 1 \
                   AND (?1 IS NULL OR language = ?1) \
                   LIMIT 1";
        let found: Option<i64> = self
            .conn
            .query_row(sql, params![lang], |r| r.get(0))
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        Ok(found.is_some())
    }
    pub(crate) fn fill_pattern_excerpt(&self, row: &super::PatternNodeRow) -> Result<String> {
        if row.excerpt.is_empty() {
            self.indexed_excerpt_in_range(&row.path, row.line_start, row.line_end)
        } else {
            Ok(row.excerpt.clone())
        }
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
        if let Ok(Some(bundle)) = self.read_bundle(None) {
            return Ok(
                crate::store::bundle_graph::patterns_exact(&bundle, signature, lang, limit)
                    .into_iter()
                    .map(|(path, language, line_start, line_end)| PatternNodeRow {
                        path,
                        language,
                        line_start,
                        line_end,
                        excerpt: String::new(),
                    })
                    .collect(),
            );
        }
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
    /// Hybrid structural stage: only matching signatures in the cascade files.
    ///
    /// Unbounded `pattern_nodes_matching` walks every row for a signature.
    /// `INDEXED BY idx_pattern_nodes_file` walked every node in those files
    /// (~1k–25k/file). Join `files` to `pattern_nodes` and let SQLite seek
    /// `idx_pattern_nodes_file_sig` `(file_id, signature)`. No `ORDER BY`:
    /// finish sorts the keep-set. Placeholders quantized to a power of two
    /// ≥ 8, padded with `''` (no indexed path/signature is empty).
    pub(crate) fn pattern_nodes_matching_for_files(
        &self,
        signatures: &[String],
        lang: Option<&str>,
        files: &std::collections::HashSet<String>,
        limit: usize,
    ) -> Result<Vec<(PatternNodeRow, String)>> {
        if files.is_empty() || signatures.is_empty() {
            return Ok(Vec::new());
        }
        let mut paths: Vec<String> = files.iter().cloned().collect();
        paths.sort_unstable();
        let path_bucket = paths.len().next_power_of_two().max(8);
        if path_bucket > paths.len() {
            paths.resize(path_bucket, String::new());
        }
        let mut sigs: Vec<String> = signatures.to_vec();
        sigs.sort_unstable();
        sigs.dedup();
        let sig_n = sigs.len();
        let sig_bucket = sig_n.next_power_of_two().max(8);
        if sig_bucket > sig_n {
            sigs.resize(sig_bucket, String::new());
        }
        let path_ph = (1..=path_bucket)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sig_start = path_bucket + 1;
        let sig_end = path_bucket + sig_bucket;
        let sig_ph = (sig_start..=sig_end)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        // Cap the row budget exactly like the symbol/caller/anchor passes, and
        // order by the index the planner already drives (signature, then rowid)
        // so the kept prefix is the same one today's scan yields first. A common
        // term matches hundreds of nodes inside the 100-file cascade while
        // fusion keeps `limit`; the tail paid row materialization for hits that
        // could never survive (measured: 374 rows / 1.17 ms for `hits`).
        let mut bind: Vec<String> = paths.clone();
        bind.extend(sigs.iter().cloned());
        if let Some(language) = lang {
            bind.push(language.to_string());
        }
        let lang_ph = if lang.is_some() {
            format!(" AND f.language = ?{}", bind.len())
        } else {
            String::new()
        };
        let sql = format!(
            "SELECT f.path, f.language, n.line_start, n.line_end, n.excerpt, n.signature \
             FROM files f JOIN pattern_nodes n ON n.file_id = f.id \
             WHERE f.path IN ({path_ph}) AND n.signature IN ({sig_ph}){lang_ph} \
             ORDER BY n.signature, n.id LIMIT ?{}",
            bind.len() + 1
        );
        let map = |r: &rusqlite::Row<'_>| {
            Ok((
                PatternNodeRow {
                    path: r.get(0)?,
                    language: r.get(1)?,
                    line_start: r.get(2)?,
                    line_end: r.get(3)?,
                    excerpt: r.get(4)?,
                },
                r.get::<_, String>(5)?,
            ))
        };
        query_limit_map(&self.conn, &sql, bind, limit, map)
    }
}
