use super::super::sql::{
    emb_vec, lang_and_clause, optional_row, query_cached_map, query_map_rows, read_legacy_emb,
    read_sem_row,
};
use super::{IndexStore, SemanticChunkStats};
use crate::Result;
use rusqlite::params;

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

fn field_blob_sql(on: bool, column: &'static str) -> &'static str {
    if on {
        column
    } else {
        "NULL"
    }
}

fn field_vectors_by_ids_sql(
    mask: crate::semantic_chunk::FieldVectorMask,
    placeholders: &str,
) -> String {
    format!(
        "SELECT id, {}, {}, {}, {}, {} FROM semantic_chunks WHERE id IN ({placeholders})",
        field_blob_sql(mask.name, "vector_name"),
        field_blob_sql(mask.docs, "vector_docs"),
        field_blob_sql(mask.body, "vector_body"),
        field_blob_sql(mask.graph, "vector_graph"),
        field_blob_sql(mask.tests_examples, "vector_tests_examples"),
    )
}

impl IndexStore {
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
        // Do not `MAX(length(vector))` over the table: that scans every blob
        // (~5 ms at 54k). IVF and search require uniform dim, so one row is
        // enough. COUNT/MAX(id) stay on the integer PK.
        let (count, max_id, dim): (usize, i64, usize) = if let Some(l) = lang {
            self.conn.query_row(
                "SELECT COUNT(*), COALESCE(MAX(sc.id),0),                  COALESCE(length((SELECT sc2.vector FROM semantic_chunks sc2                     JOIN files f2 ON f2.id=sc2.file_id WHERE f2.language=?1 LIMIT 1))/4, 0)                  FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE f.language=?1",
                params![l],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?
        } else {
            self.conn.query_row(
                "SELECT COUNT(*), COALESCE(MAX(id),0),                  COALESCE(length((SELECT vector FROM semantic_chunks LIMIT 1))/4, 0)                  FROM semantic_chunks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?
        };
        Ok(SemanticChunkStats { count, max_id, dim })
    }

    pub fn semantic_primary_dim(&self) -> Result<usize> {
        Ok(optional_row(
            &self.conn,
            "SELECT length(vector)/4 FROM semantic_chunks LIMIT 1",
            &[],
            |row| row.get::<_, i64>(0),
        )?
        .unwrap_or(0) as usize)
    }
    pub fn semantic_chunk_ids(&self, lang: Option<&str>) -> Result<Vec<i64>> {
        let (sql, l) = if lang.is_some() {
            ("SELECT sc.id FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE f.language=?1 ORDER BY sc.id", lang)
        } else {
            ("SELECT id FROM semantic_chunks ORDER BY id", None)
        };
        query_map_rows(&self.conn, sql, l, |r| r.get(0))
    }
    /// Count and max id of the id column [`Self::semantic_chunk_ids`] returns
    /// — exactly the fingerprint inputs `compute_ann_fingerprint` needs, at
    /// index-seek cost instead of a full id-array load.
    pub fn semantic_chunk_count_and_max_id(&self) -> Result<(usize, i64)> {
        let (count, max_id) = query_map_rows(
            &self.conn,
            "SELECT COUNT(*), (SELECT id FROM semantic_chunks ORDER BY id DESC LIMIT 1) FROM semantic_chunks",
            None,
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
        )?
        .into_iter()
        .next()
        .unwrap_or((0, 0));
        Ok((count as usize, max_id))
    }
    /// Same ORDER BY id as `semantic_chunk_ids(None)`, with the chunk's file path.
    /// IVF mmap row i is ids[i]; hybrid file-restrict uses paths[i].
    pub fn semantic_chunk_ids_and_paths(&self) -> Result<Vec<(i64, String)>> {
        query_map_rows(
            &self.conn,
            "SELECT sc.id, f.path FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id ORDER BY sc.id",
            None,
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }
    pub fn semantic_chunk_hits_by_ids(
        &self,
        ids: &[i64],
    ) -> Result<Vec<(i64, ast_sgrep_embed::SemanticChunkRow)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(ids.len());
        for batch in ids.chunks(500) {
            let ph = std::iter::repeat_n("?", batch.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT sc.id, f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text                  FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE sc.id IN ({ph})"
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(batch.iter()), |r| {
                let id: i64 = r.get(0)?;
                let row = ast_sgrep_embed::SemanticChunkRow {
                    path: r.get(1)?,
                    line_start: r.get(2)?,
                    line_end: r.get(3)?,
                    symbol: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    text: r.get(5)?,
                    embedding: Vec::new(),
                };
                Ok((id, row))
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    /// One IN-list round trip for IVF survivors: hit metadata plus the
    /// intent-masked field blobs. Same rows as hits_by_ids + field_vectors_by_ids.
    pub fn semantic_hits_and_fields_by_ids(
        &self,
        ids: &[i64],
        mask: crate::semantic_chunk::FieldVectorMask,
    ) -> Result<
        Vec<(
            i64,
            ast_sgrep_embed::SemanticChunkRow,
            crate::semantic_chunk::SemanticFieldVectors,
        )>,
    > {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(ids.len());
        for batch in ids.chunks(500) {
            let ph = std::iter::repeat_n("?", batch.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT sc.id, f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text, {}, {}, {}, {}, {} \
                 FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE sc.id IN ({ph})",
                field_blob_sql(mask.name, "sc.vector_name"),
                field_blob_sql(mask.docs, "sc.vector_docs"),
                field_blob_sql(mask.body, "sc.vector_body"),
                field_blob_sql(mask.graph, "sc.vector_graph"),
                field_blob_sql(mask.tests_examples, "sc.vector_tests_examples"),
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(batch.iter()), |r| {
                let id: i64 = r.get(0)?;
                let row = ast_sgrep_embed::SemanticChunkRow {
                    path: r.get(1)?,
                    line_start: r.get(2)?,
                    line_end: r.get(3)?,
                    symbol: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    text: r.get(5)?,
                    embedding: Vec::new(),
                };
                let fields = crate::semantic_chunk::SemanticFieldVectors {
                    name: r.get(6)?,
                    docs: r.get(7)?,
                    body: r.get(8)?,
                    graph: r.get(9)?,
                    tests_examples: r.get(10)?,
                };
                Ok((id, row, fields))
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
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
                let row = ast_sgrep_embed::SemanticChunkRow {
                    path: r.get(1)?,
                    line_start: r.get(2)?,
                    line_end: r.get(3)?,
                    symbol: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    text: r.get(5)?,
                    embedding: emb_vec(r, 6)?,
                };
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
        mask: crate::semantic_chunk::FieldVectorMask,
    ) -> Result<Vec<(i64, crate::semantic_chunk::SemanticFieldVectors)>> {
        if ids.is_empty() || !mask.any() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(ids.len());
        for batch in ids.chunks(500) {
            let placeholders = std::iter::repeat_n("?", batch.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = field_vectors_by_ids_sql(mask, &placeholders);
            // I5a: same statement-cache rationale as semantic_chunks_by_ids.
            // Mask cardinality is tiny (intent × bucket), so prepare_cached still hits.
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params_from_iter(batch.iter()),
                read_field_vector_row,
            )?;
            for row in rows {
                out.push(row?);
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
    /// `semantic_chunks_for_files`. The
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
        if let Some(language) = lang {
            bind.push(language)
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
    ) -> Result<Vec<(i64, ast_sgrep_embed::SemanticChunkRow)>> {
        self.semantic_rows_batched(
            files,
            lang,
            "sc.id, f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text, sc.vector",
            |r| {
                let id: i64 = r.get(0)?;
                let row = ast_sgrep_embed::SemanticChunkRow {
                    path: r.get(1)?,
                    line_start: r.get(2)?,
                    line_end: r.get(3)?,
                    symbol: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    text: r.get(5)?,
                    embedding: emb_vec(r, 6)?,
                };
                Ok((id, row))
            },
        )
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
}
