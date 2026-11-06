use super::super::embed_support::embed_cache_cap;
use super::super::sql::{at_least_rows, count_star, optional_row, query_cached_map};
use super::{IndexStore, IndexedLineRow};
use crate::{IndexStatus, Result};
use rusqlite::types::{Type, ValueRef};
use rusqlite::params;
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
}
