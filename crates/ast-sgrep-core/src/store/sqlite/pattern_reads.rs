use super::{IndexStore, PatternNodeRow};
use super::super::sql::query_cached_map;
use crate::Result;
use rusqlite::params;
use rusqlite::types::Type;

impl IndexStore {
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
    /// (~1k–25k/file). Drive `idx_pattern_nodes_signature` and admit only the
    /// cascade's file set through an id-keyed CTE. Placeholders quantized to a
    /// power of two ≥ 8, padded with `''` / `0` (no indexed path or signature
    /// is empty, and no `files` rowid is 0).
    ///
    /// The file set is resolved to ids before the scan. Filtering the join
    /// with `f.path IN (<100 paths>)` made SQLite compare every signature
    /// match against 100 path strings, and a common signature matches
    /// thousands of nodes: the structural pass cost 240–350ms per query on a
    /// 4.5k-file corpus. The id-keyed CTE yields the identical row set and
    /// order at a tenth of the cost (measured `config` 347ms → 29ms, `error`
    /// 241ms → 23ms, byte-identical rows).
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
        let path_ph = (1..=path_bucket)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut ids: Vec<i64> = {
            let mut stmt = self
                .conn
                .prepare_cached(&format!("SELECT id FROM files WHERE path IN ({path_ph})"))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(paths.iter()), |r| r.get(0))?;
            rows.collect::<std::result::Result<Vec<i64>, _>>()?
        };
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        ids.sort_unstable();
        let id_bucket = ids.len().next_power_of_two().max(8);
        ids.resize(id_bucket, 0);
        let mut sigs: Vec<String> = signatures.to_vec();
        sigs.sort_unstable();
        sigs.dedup();
        let sig_n = sigs.len();
        let sig_bucket = sig_n.next_power_of_two().max(8);
        if sig_bucket > sig_n {
            sigs.resize(sig_bucket, String::new());
        }
        let id_values = (1..=id_bucket)
            .map(|i| format!("(?{i})"))
            .collect::<Vec<_>>()
            .join(",");
        let sig_start = id_bucket + 1;
        let sig_end = id_bucket + sig_bucket;
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
        let mut bind: Vec<rusqlite::types::Value> = ids
            .iter()
            .map(|&id| rusqlite::types::Value::Integer(id))
            .collect();
        bind.extend(sigs.iter().cloned().map(rusqlite::types::Value::Text));
        if let Some(language) = lang {
            bind.push(rusqlite::types::Value::Text(language.to_string()));
        }
        let lang_ph = if lang.is_some() {
            format!(" AND f.language = ?{}", bind.len())
        } else {
            String::new()
        };
        let sql = format!(
            "WITH allowed(id) AS (VALUES {id_values}) \
             SELECT f.path, f.language, n.line_start, n.line_end, n.excerpt, n.signature \
             FROM pattern_nodes n JOIN allowed a ON a.id = n.file_id \
             JOIN files f ON f.id = n.file_id \
             WHERE n.signature IN ({sig_ph}){lang_ph} \
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
        bind.push(rusqlite::types::Value::Integer(limit as i64));
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(bind.iter()), map)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}
