use super::IndexStore;
use crate::Result;
use rusqlite::params;

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
