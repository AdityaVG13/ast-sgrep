use super::super::embed_support::read_sym_loc;
use super::super::sql::{
    calls_matching, calls_matching_limited, optional_row, query_cached_map,
    query_cached_map_capped,
};
use super::{
    CallEvidenceRow, CallRow, ImportRow, IndexStore, SymbolLocationRow, SymbolRow,
    SYM_LOC, sql_usize_from_byte_offset,
};
use crate::Result;
use rusqlite::{ToSql, params};

impl IndexStore {
    /// Highest indexed file mtime, as a worktree revision proxy (d3l5).
    ///
    /// This is what the index believes about the worktree, so a response can
    /// state the source state it actually read rather than the current one.
    pub fn worktree_revision(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(mtime_secs), 0) FROM files",
            [],
            |row| row.get::<_, i64>(0),
        )?)
    }

    /// Count definitions with the same case folding as `symbols_named`.
    /// The total count accompanies the map so caller resolution avoids per-hit SQL.
    pub(crate) fn symbol_name_candidate_counts(
        &self,
        name: &str,
    ) -> Result<(Vec<(String, usize)>, usize)> {
        if let Ok(Some(bundle)) = self.read_bundle(None) {
            return Ok(crate::store::bundle_graph::symbol_name_counts(
                &bundle, name,
            ));
        }
        let mut statement = self.conn.prepare_cached(
            "SELECT f.path, COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE lower(s.name)=lower(?1) GROUP BY f.path ORDER BY f.path",
        )?;
        let rows = statement.query_map(params![name], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
        })?;
        let mut by_file = Vec::new();
        let mut total = 0usize;
        for row in rows {
            let (file, count) = row?;
            total = total.saturating_add(count);
            by_file.push((file, count));
        }
        Ok((by_file, total))
    }

    /// Visit bounded symbol context without buffering the whole repository.
    pub fn for_each_symbol_context(&self, mut visit: impl FnMut(&str, &str)) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT s.name, COALESCE(GROUP_CONCAT(SUBSTR(l.content, 1, 1600), ' '), '')
             FROM symbols s
             LEFT JOIN lines l
               ON l.file_id = s.file_id
              AND l.line_no BETWEEN MAX(1, s.line_start - 2) AND s.line_start + 2
             GROUP BY s.id
             ORDER BY s.id
             LIMIT 100000",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (name, context) = row?;
            visit(&name, &context);
        }
        Ok(())
    }
}

impl IndexStore {
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
}
