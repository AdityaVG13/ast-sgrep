use super::embed_support::{
    embed_cache_cap, embed_chunks, evict_embed_cache, init_cache_seq, insert_embed_cache_entries,
    normalize_rel, read_sym_loc, structure_fingerprint, touch_embed_cache_entries, EmbeddedChunk,
    EmbeddedChunks,
};
use super::index_db_path;
use super::sql::configure_connection;
use super::sql::{
    append_lang_filter, calls_matching, count_star, delete_file_children, delete_file_lines,
    lang_and_clause, like_terms_filter, optional_row, query_cached_map, query_limit_map,
    query_map_rows, read_legacy_emb, read_sem_row, where_clause, CLEAR_ALL_SQL, SCHEMA_DDL,
};
use crate::{IndexStatus, Result};
use ast_sgrep_lang::PatternNode;
use rusqlite::types::{Type, ValueRef};
use rusqlite::{params, Connection, ToSql};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;
const SCHEMA_VERSION: i64 = 5;
const IMPORT_SELECT: &str =
    "SELECT f.path, f.language, i.module_path, i.line_no FROM imports i JOIN files f ON f.id = i.file_id";
const SYM_LOC: &str = "SELECT f.path, s.name, f.language, s.line_start, s.line_end FROM symbols s JOIN files f ON f.id = s.file_id";
pub type IndexedLineRow = (Arc<str>, u32, String, Option<Arc<str>>);
pub type ImportQueryRow = (String, Option<String>, String, u32);
pub type CallRow = (String, u32, String, String);
pub struct PatternNodeRow {
    pub path: String,
    pub language: Option<String>,
    pub line_start: u32,
    pub line_end: u32,
    pub excerpt: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticChunkStats {
    pub count: usize,
    pub max_id: i64,
    pub dim: usize,
}
#[derive(Debug, Clone)]
pub struct SymbolRow {
    pub name: String,
    pub kind: String,
    pub line_start: u32,
    pub line_end: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}
#[derive(Debug, Clone)]
pub struct CallerRow {
    pub caller: String,
    pub callee: String,
    pub line_no: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}
#[derive(Debug, Clone)]
pub struct ImportRow {
    pub module_path: String,
    pub line_no: u32,
}
#[derive(Debug, Clone)]
pub struct SymbolLocationRow {
    pub path: String,
    pub name: String,
    pub language: Option<String>,
    pub line_start: u32,
    pub line_end: u32,
}
pub struct UpsertFileInput<'a> {
    pub rel_path: &'a str,
    pub language: Option<&'a str>,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub content_hash: &'a str,
    pub lines: &'a [(u32, String)],
    pub eol: &'a str,
    pub symbols: &'a [SymbolRow],
    pub callers: &'a [CallerRow],
    pub imports: &'a [ImportRow],
    pub pattern_nodes: &'a [PatternNode],
    pub semantic_chunks: &'a [crate::semantic_chunk::SemanticChunkInput],
    pub embed_semantic: bool,
    pub embed_backend: ast_sgrep_embed::EmbedPreference,
}
pub struct IndexStore {
    conn: Connection,
    root: std::path::PathBuf,
    db_path: std::path::PathBuf,
    file_tx_active: std::cell::Cell<bool>,
    cache_seq: std::cell::Cell<i64>,
}
impl IndexStore {
    pub fn open(root: &Path, index_path: Option<&Path>) -> Result<Self> {
        let db_path = index_db_path(root, index_path);
        if let Some(p) = db_path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let conn = Connection::open(&db_path)?;
        configure_connection(&conn)?;
        let store = Self {
            conn,
            root: root.to_path_buf(),
            db_path,
            file_tx_active: std::cell::Cell::new(false),
            cache_seq: std::cell::Cell::new(0),
        };
        store.init_schema()?;
        init_cache_seq(&store.conn, &store.cache_seq)?;
        Ok(store)
    }
    fn init_schema(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }
        self.conn.execute_batch(SCHEMA_DDL)?;
        if version < 3 {
            self.conn.execute_batch(
                "INSERT INTO lines_trigram(rowid, content) SELECT rowid, content FROM lines;",
            )?;
        }
        self.conn
            .execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        Ok(())
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.prepare_cached( "INSERT INTO meta(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )?.execute(params![key, value])?;
        Ok(())
    }
    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        optional_row(
            &self.conn,
            "SELECT value FROM meta WHERE key = ?1",
            &[&key],
            |r| r.get(0),
        )
    }
    pub fn delete_meta(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM meta WHERE key = ?1", params![key])?;
        Ok(())
    }
    pub fn file_id(&self, rel_path: &str) -> Result<Option<i64>> {
        optional_row(
            &self.conn,
            "SELECT id FROM files WHERE path = ?1",
            &[&rel_path],
            |r| r.get(0),
        )
    }
    /// File-tx stays OFF until bulk commit (no re-NORMAL after each file).
    pub fn begin_file_tx(&self) -> Result<()> {
        if !self.conn.is_autocommit() {
            return Ok(());
        }
        self.conn
            .execute_batch("PRAGMA synchronous = OFF; BEGIN IMMEDIATE")?;
        self.file_tx_active.set(true);
        Ok(())
    }
    pub fn commit_file_tx(&self) -> Result<()> {
        self.end_file_tx(true)
    }
    pub fn rollback_file_tx(&self) -> Result<()> {
        self.end_file_tx(false)
    }
    fn end_file_tx(&self, commit: bool) -> Result<()> {
        if !self.file_tx_active.replace(false) {
            return Ok(());
        }
        if commit {
            self.conn.execute_batch("COMMIT")?;
        } else {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        Ok(())
    }
    fn with_file_tx<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.begin_file_tx()?;
        match f() {
            Ok(v) => {
                self.commit_file_tx()?;
                Ok(v)
            }
            Err(e) => {
                self.rollback_file_tx()?;
                Err(e)
            }
        }
    }
    fn meta_u64(&self, key: &str) -> Result<u64> {
        Ok(self
            .get_meta(key)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }
    fn bump_meta_u64(&self, key: &str, delta: usize) -> Result<()> {
        if delta == 0 {
            return Ok(());
        }
        let total = self.meta_u64(key)?.saturating_add(delta as u64);
        self.set_meta(key, &total.to_string())
    }
    pub fn begin_bulk_tx(&self) -> Result<()> {
        if !self.conn.is_autocommit() {
            return Ok(());
        }
        self.conn.execute_batch(
            "PRAGMA temp_store = MEMORY; PRAGMA cache_size = -131072; PRAGMA mmap_size = 536870912; \
             PRAGMA synchronous = OFF; BEGIN IMMEDIATE",
        )?;
        Ok(())
    }
    pub fn commit_bulk_tx(&self) -> Result<()> {
        self.end_bulk_tx(true)
    }
    pub fn rollback_bulk_tx(&self) -> Result<()> {
        self.end_bulk_tx(false)
    }
    fn end_bulk_tx(&self, commit: bool) -> Result<()> {
        if self.conn.is_autocommit() {
            return Ok(());
        }
        self.file_tx_active.set(false);
        if commit {
            self.conn.execute_batch("COMMIT")?;
            let _ = self
                .conn
                .execute_batch("PRAGMA synchronous = NORMAL; PRAGMA cache_size = -16384");
        } else {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        Ok(())
    }
    pub fn clear_all_data(&self) -> Result<()> {
        self.conn.execute_batch(CLEAR_ALL_SQL)?;
        let _ = self.conn.execute_batch("VACUUM");
        Ok(())
    }
    pub fn upsert_file(&self, input: UpsertFileInput<'_>) -> Result<i64> {
        let struct_fp = structure_fingerprint(
            input.symbols,
            input.callers,
            input.imports,
            input.pattern_nodes,
            input.semantic_chunks,
        );
        let struct_key = format!("struct:{}", input.rel_path);
        if let Some(file_id) = self.file_id(input.rel_path)? {
            if self.get_meta(&struct_key)?.as_deref() == Some(struct_fp.as_str()) {
                return self.with_file_tx(|| {
                    self.refresh_lines_only(
                        file_id,
                        input.language,
                        input.mtime_secs,
                        input.mtime_nanos,
                        input.content_hash,
                        input.lines,
                        input.eol,
                        input.rel_path,
                    )
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
    pub fn refresh_lines_only(
        &self,
        file_id: i64,
        lang: Option<&str>,
        mtime_secs: i64,
        mtime_nanos: u32,
        hash: &str,
        lines: &[(u32, String)],
        eol: &str,
        rel_path: &str,
    ) -> Result<i64> {
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
        )?;
        self.insert_callers(file_id, input.callers)?;
        self.insert_imports(file_id, input.imports)?;
        self.insert_pattern_nodes(file_id, input.pattern_nodes)?;
        self.set_meta(struct_key, struct_fp)?;
        crate::semantic_ann::mark_semantic_ivf_stale(self);
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
    fn insert_lines(&self, file_id: i64, lines: &[(u32, String)]) -> Result<()> {
        let mut ls = self
            .conn
            .prepare_cached("INSERT INTO lines(file_id, line_no, content) VALUES(?1,?2,?3)")?;
        let mut fts = self.conn.prepare_cached(
            "INSERT INTO lines_fts(rowid, content, file_id, line_no) VALUES(?1,?2,?3,?4)",
        )?;
        let mut tri = self
            .conn
            .prepare_cached("INSERT INTO lines_trigram(rowid, content) VALUES(?1,?2)")?;
        for (no, content) in lines {
            ls.execute(params![file_id, no, content])?;
            let rid = self.conn.last_insert_rowid();
            fts.execute(params![rid, content, file_id, no])?;
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
    ) -> Result<()> {
        if emb.is_empty() {
            return Ok(());
        }
        if emb.len() < chunks.len() && emb[0].backend == ast_sgrep_embed::EmbedBackendKind::Neural {
            let (first, last) = (&chunks[0], &chunks[chunks.len() - 1]);
            for e in emb {
                self.conn.execute(
                    "INSERT INTO semantic_chunks(file_id, symbol_id, chunk_kind, line_start, line_end, symbol_name, text, vector) VALUES(?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![file_id, "file", first.line_start, last.line_end, "", &e.text, &e.vector_bytes], )?;
            }
            let last = &emb[emb.len() - 1];
            return self.persist_embed_metadata(Some(last.dim), Some(last.backend));
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
                "symbol",
                c.line_start,
                c.line_end,
                c.symbol_name,
                e.text,
                e.vector_bytes
            ])?;
        }
        let last = emb.last();
        self.persist_embed_metadata(last.map(|e| e.dim), last.map(|e| e.backend))
    }
    fn persist_embed_metadata(
        &self,
        dim: Option<usize>,
        kind: Option<ast_sgrep_embed::EmbedBackendKind>,
    ) -> Result<()> {
        if let Some(k) = kind {
            self.set_meta("embed_backend", k.as_meta_str())?;
            if k == ast_sgrep_embed::EmbedBackendKind::Neural {
                self.set_meta("embed_model", ast_sgrep_embed::neural_configured_model_id())?;
            } else {
                self.delete_meta("embed_model")?;
            }
        }
        if let Some(d) = dim {
            self.set_meta("embed_dim", &d.to_string())?;
        }
        Ok(())
    }
    fn insert_callers(&self, file_id: i64, callers: &[CallerRow]) -> Result<()> {
        let mut st = self.conn.prepare_cached( "INSERT INTO callers(file_id, caller, callee, line_no, byte_start, byte_end) VALUES(?1,?2,?3,?4,?5,?6)",
        )?;
        for c in callers {
            st.execute(params![
                file_id,
                c.caller,
                c.callee,
                c.line_no,
                c.byte_start as i64,
                c.byte_end as i64
            ])?;
        }
        Ok(())
    }
    fn insert_pattern_nodes(&self, file_id: i64, nodes: &[PatternNode]) -> Result<()> {
        let mut st = self.conn.prepare_cached( "INSERT INTO pattern_nodes(file_id, signature, line_start, line_end, excerpt) VALUES(?1,?2,?3,?4,?5)",
        )?;
        for n in nodes {
            st.execute(params![
                file_id,
                n.signature,
                n.line_start,
                n.line_end,
                n.excerpt
            ])?;
        }
        Ok(())
    }
    fn insert_imports(&self, file_id: i64, imports: &[ImportRow]) -> Result<()> {
        let mut st = self.conn.prepare_cached(
            "INSERT INTO imports(file_id, module_path, line_no) VALUES(?1,?2,?3)",
        )?;
        for i in imports {
            st.execute(params![file_id, i.module_path, i.line_no])?;
        }
        Ok(())
    }
    pub fn remove_file(&self, rel_path: &str) -> Result<()> {
        if let Some(id) = self.file_id(rel_path)? {
            delete_file_children(&self.conn, id)?;
            self.conn
                .execute("DELETE FROM files WHERE id = ?1", params![id])?;
            self.delete_meta(&format!("eol:{rel_path}"))?;
            crate::semantic_ann::mark_semantic_ivf_stale(self);
        }
        Ok(())
    }
    pub fn file_hash(&self, rel_path: &str) -> Result<Option<String>> {
        optional_row(
            &self.conn,
            "SELECT content_hash FROM files WHERE path = ?1",
            &[&rel_path],
            |r| r.get(0),
        )
    }
    pub fn all_file_paths(&self) -> Result<Vec<String>> {
        query_cached_map(
            &self.conn,
            "SELECT path FROM files ORDER BY path",
            [],
            |r| r.get(0),
        )
    }
    pub fn status(&self) -> Result<IndexStatus> {
        let (fc, lc, sc, cc, ic, sec): (i64, i64, i64, i64, i64, i64) = self.conn.query_row(
            "SELECT (SELECT COUNT(*) FROM files),(SELECT COUNT(*) FROM lines),(SELECT COUNT(*) FROM symbols),\
             (SELECT COUNT(*) FROM callers),(SELECT COUNT(*) FROM imports),(SELECT COUNT(*) FROM semantic_chunks)",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)), )?;
        Ok(IndexStatus {
            root: self.root.display().to_string(),
            index_path: self.db_path.display().to_string(),
            file_count: fc as usize,
            line_count: lc as usize,
            symbol_count: sc as usize,
            caller_count: cc as usize,
            import_count: ic as usize,
            semantic_chunk_count: sec as usize,
            embed_backend: self.get_meta("embed_backend")?,
            embed_dim: self.get_meta("embed_dim")?.and_then(|d| d.parse().ok()),
            embed_cache_entries: count_star(&self.conn, "embed_cache")?,
            embed_cache_capacity: embed_cache_cap(),
            embed_cache_hits: self.meta_u64("embed_cache_hits")?,
            embed_cache_misses: self.meta_u64("embed_cache_misses")?,
            semantic_ivf_present: crate::semantic_ivf::semantic_ivf_path(&self.db_path).exists(),
        })
    }
    pub fn indexed_line_count(&self) -> Result<usize> {
        count_star(&self.conn, "lines")
    }
    /// True when indexed lines ≥ threshold (LIMIT probe; avoids full COUNT).
    pub fn indexed_line_count_at_least(&self, threshold: usize) -> Result<bool> {
        super::sql::at_least_rows(&self.conn, "lines", threshold)
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
    pub fn semantic_chunk_max_id(&self) -> Result<Option<i64>> {
        optional_row(
            &self.conn,
            "SELECT MAX(id) FROM semantic_chunks",
            &[],
            |r| r.get::<_, Option<i64>>(0),
        )
        .map(Option::flatten)
    }
    pub fn semantic_chunk_stats(&self, lang: Option<&str>) -> Result<SemanticChunkStats> {
        let max_id = self.semantic_chunk_max_id()?.unwrap_or(0);
        let (count, dim): (i64, i64) = if let Some(l) = lang {
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
        Ok(SemanticChunkStats {
            count: count as usize,
            max_id,
            dim: dim as usize,
        })
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
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(batch.iter()), |r| {
                let id: i64 = r.get(0)?;
                let row = (
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    r.get(5)?,
                    {
                        let v: Vec<u8> = r.get(6)?;
                        ast_sgrep_embed::embed_from_bytes(&v).unwrap_or_default()
                    },
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
                    byte_start: r.get::<_, i64>(4)? as usize,
                    byte_end: r.get::<_, i64>(5)? as usize,
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
                "{SYM_LOC} WHERE s.name=?1 ORDER BY f.path, s.line_start, s.line_end LIMIT ?2"
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
        let module = module.trim().trim_matches(['"', '\'']);
        if module.is_empty() {
            return Ok(Vec::new());
        }
        let parent = Path::new(from_file)
            .parent()
            .unwrap_or_else(|| Path::new(""));
        let crate_src = from_file
            .find("/src/")
            .map(|i| Path::new(&from_file[..i + 4]));
        let slash = module.replace("::", "/");
        let mut bases = Vec::new();
        if let Some(rest) = slash.strip_prefix("crate/") {
            if let Some(src) = crate_src {
                bases.push(src.join(rest));
            }
        } else if slash == "crate" {
            if let Some(src) = crate_src {
                bases.push(src.to_path_buf());
            }
        } else if slash.starts_with("super/") || slash.starts_with("self/") {
            let mut base = parent.to_path_buf();
            let mut rest = slash.as_str();
            while let Some(n) = rest.strip_prefix("super/") {
                base.pop();
                rest = n;
            }
            rest = rest.strip_prefix("self/").unwrap_or(rest);
            bases.push(base.join(rest));
        } else if module.starts_with('.') {
            bases.push(parent.join(module));
        } else {
            bases.push(parent.join(&slash));
            if let Some(src) = crate_src {
                bases.push(src.join(&slash));
            }
        }
        const EXTS: &[&str] = &[
            "rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "cs", "rb",
        ];
        let mut cands = BTreeSet::new();
        for base in bases {
            let n = normalize_rel(&base);
            cands.insert(n.clone());
            if base.extension().is_none() {
                for e in EXTS {
                    cands.insert(format!("{n}.{e}"));
                }
                cands.insert(format!("{n}/mod.rs"));
                for e in ["ts", "tsx", "js", "jsx"] {
                    cands.insert(format!("{n}/index.{e}"));
                }
            }
        }
        let mut out = Vec::new();
        for c in cands {
            if self.file_exists(&c)? {
                out.push(c);
            }
        }
        Ok(out)
    }
    pub fn pattern_node_count(&self) -> Result<usize> {
        count_star(&self.conn, "pattern_nodes")
    }
    pub fn pattern_nodes_matching(
        &self,
        signature: &str,
        lang: Option<&str>,
    ) -> Result<Vec<PatternNodeRow>> {
        let mut sql = String::from(
            "SELECT f.path, f.language, n.line_start, n.line_end, n.excerpt FROM pattern_nodes n JOIN files f ON f.id=n.file_id WHERE n.signature=?1",
        );
        if lang.is_some() {
            sql.push_str(" AND f.language=?2");
        }
        sql.push_str(" ORDER BY f.path, n.line_start");
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
        if module.is_none_or(|m| m.is_empty()) {
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
        }
        let m = module.unwrap().to_string();
        let (w, bind) = like_terms_filter("i.module_path", &[m], lang);
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
