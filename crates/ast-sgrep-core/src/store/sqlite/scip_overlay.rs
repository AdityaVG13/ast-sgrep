use super::IndexStore;
use crate::Result;
use rusqlite::params;
use std::collections::HashSet;

impl IndexStore {
    /// Persist SCIP defs/refs that match existing symbols/callers (kgvi.2).
    /// Unmatched occurrences are skipped; no new graph edges are invented.
    pub fn apply_scip(
        &self,
        index: &crate::scip::ScipIndex,
    ) -> Result<crate::scip::ScipApplyStats> {
        self.with_file_tx(|| self.apply_scip_inner(index))
    }

    fn apply_scip_inner(
        &self,
        index: &crate::scip::ScipIndex,
    ) -> Result<crate::scip::ScipApplyStats> {
        self.conn.execute("DELETE FROM scip_facts", [])?;
        let mut files = Vec::new();
        {
            let mut stmt = self.conn.prepare("SELECT id, path FROM files")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, i64>(0)?))
            })?;
            for row in rows {
                files.push(row?);
            }
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        let mut symbols = Vec::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT file_id, line_start, line_end, name FROM symbols")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            for row in rows {
                symbols.push(row?);
            }
        }
        let mut callers = HashSet::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT file_id, line_no, callee FROM callers")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                callers.insert(row?);
            }
        }
        let mut stats = crate::scip::ScipApplyStats::default();
        let mut insert = self.conn.prepare(
            "INSERT OR IGNORE INTO scip_facts(file_id, line_no, name, is_def) VALUES(?1, ?2, ?3, ?4)",
        )?;
        for doc in &index.documents {
            // Indexed paths already have host-correct identity. Prefer an
            // exact document key before accepting alternate wire separators.
            let trimmed = doc.relative_path.trim_start_matches("./");
            let normalized = crate::scip::normalize_scip_path(&doc.relative_path);
            let Some(file_id) = files
                .binary_search_by(|(path, _)| path.as_str().cmp(trimmed))
                .ok()
                .or_else(|| {
                    files
                        .binary_search_by(|(path, _)| path.as_str().cmp(&normalized))
                        .ok()
                })
                .map(|i| files[i].1)
            else {
                stats.skipped += doc.occurrences.len();
                continue;
            };
            for occ in &doc.occurrences {
                let Some(name) = crate::scip::scip_symbol_ident(&occ.symbol) else {
                    stats.skipped += 1;
                    continue;
                };
                let Some(line) = occ.start_line_1based() else {
                    stats.skipped += 1;
                    continue;
                };
                let fact_line = if occ.is_definition() {
                    symbols.iter().find_map(|(fid, start, end, n)| {
                        (*fid == file_id && n == &name && line >= *start && line <= *end)
                            .then_some(*start)
                    })
                } else {
                    callers
                        .contains(&(file_id, line, name.clone()))
                        .then_some(line)
                };
                let Some(fact_line) = fact_line else {
                    stats.skipped += 1;
                    continue;
                };
                insert.execute(params![
                    file_id,
                    i64::from(fact_line),
                    name,
                    i64::from(u8::from(occ.is_definition()))
                ])?;
                if occ.is_definition() {
                    stats.defs_upgraded += 1;
                } else {
                    stats.refs_upgraded += 1;
                }
            }
        }
        Ok(stats)
    }

    pub fn scip_fact_set(&self, is_def: bool) -> Result<Vec<(String, u32, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.path, s.line_no, s.name FROM scip_facts s JOIN files f ON f.id = s.file_id WHERE s.is_def = ?1 ORDER BY f.path, s.line_no, s.name",
        )?;
        let rows = stmt.query_map(params![i64::from(u8::from(is_def))], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}
