use super::IndexStore;
use crate::Result;
use rusqlite::params;

impl IndexStore {
    /// Remove learned associations as part of the caller's active transaction.
    /// The dirty marker suppresses repeated generation bumps in bulk indexing.
    pub(crate) fn invalidate_lexicon(&self) -> Result<()> {
        if self.get_meta("lexicon_dirty")?.as_deref() == Some("1") {
            return Ok(());
        }
        self.conn.execute("DELETE FROM lexicon", [])?;
        self.bump_meta_u64("lexicon_data_version", 1)?;
        self.set_meta("lexicon_dirty", "1")
    }

    pub(crate) fn lexicon_is_dirty(&self) -> Result<bool> {
        Ok(self.get_meta("lexicon_dirty")?.as_deref() == Some("1"))
    }

    /// Replace the repository lexicon in one transaction (ufk7).
    pub fn replace_lexicon(&self, associations: &[crate::lexicon::Association]) -> Result<()> {
        if associations.len() > crate::lexicon::MAX_PAIRS {
            return Err(crate::StoreError::Other(format!(
                "lexicon exceeds maximum of {} associations",
                crate::lexicon::MAX_PAIRS
            )));
        }
        if associations.iter().any(|association| {
            association.term.chars().count() > crate::lexicon::MAX_TERM_CHARS
                || association.related.chars().count() > crate::lexicon::MAX_TERM_CHARS
        }) {
            return Err(crate::StoreError::Other(format!(
                "lexicon term exceeds maximum of {} characters",
                crate::lexicon::MAX_TERM_CHARS
            )));
        }
        if associations
            .iter()
            .any(|association| !association.ppmi.is_finite())
        {
            return Err(crate::StoreError::Other(
                "lexicon contains a non-finite score".into(),
            ));
        }
        self.with_file_tx(|| {
            self.conn.execute("DELETE FROM lexicon", [])?;
            {
                let mut stmt = self.conn.prepare_cached(
                    "INSERT OR REPLACE INTO lexicon(term, related, ppmi, support) VALUES(?1,?2,?3,?4)",
                )?;
                for association in associations {
                    stmt.execute(params![
                        association.term,
                        association.related,
                        association.ppmi,
                        association.support
                    ])?;
                }
            }
            self.delete_meta("lexicon_dirty")?;
            self.bump_meta_u64("lexicon_data_version", 1)
        })
    }

    /// Decode one stored lexicon row, failing closed on oversized terms,
    /// non-finite scores, and out-of-range support. Shared by the full and
    /// targeted loads so their validation cannot diverge.
    fn decode_lexicon_row(
        term: String,
        related: String,
        ppmi: f64,
        support: i64,
        term_chars: i64,
        related_chars: i64,
    ) -> Result<crate::lexicon::Association> {
        if term_chars > crate::lexicon::MAX_TERM_CHARS as i64
            || related_chars > crate::lexicon::MAX_TERM_CHARS as i64
        {
            return Err(crate::StoreError::Other(format!(
                "stored lexicon term exceeds maximum of {} characters",
                crate::lexicon::MAX_TERM_CHARS
            )));
        }
        if !ppmi.is_finite() {
            return Err(crate::StoreError::Other(
                "stored lexicon contains a non-finite score".into(),
            ));
        }
        let support = u32::try_from(support).map_err(|_| {
            crate::StoreError::Other("stored lexicon support is out of range".into())
        })?;
        Ok(crate::lexicon::Association {
            term,
            related,
            ppmi,
            support,
        })
    }

    /// Read only the lexicon rows `terms` can expand. `Lexicon::expand`
    /// consults `related(term)` for each query term, and each per-term list is
    /// built solely from rows with `term` or `related` equal to that term, in
    /// the same `ORDER BY term, related` sequence the full load sees — so the
    /// per-term lists (and therefore every expansion) are identical to the
    /// full load for every query. Only unserved-row validation is skipped:
    /// corruption outside the served set no longer poisons the load.
    pub fn lexicon_rows_for_terms(
        &self,
        terms: &[String],
    ) -> Result<Vec<crate::lexicon::Association>> {
        // Packed rows are `ORDER BY term, related` at pack time. Hybrid critic
        // used to re-query SQL so SUBSTR/LENGTH fail-closed matched the control
        // arm; on a healthy lexicon (builder already caps MAX_TERM_CHARS) the
        // mmap filter is the same set in the same order. SQL remains the
        // `ASGREP_READPATH=sql` / no-bundle path.
        let mut unique: Vec<&str> = terms.iter().map(String::as_str).collect();
        unique.sort();
        unique.dedup();
        if unique.is_empty() {
            return Ok(Vec::new());
        }
        // Two binds per term; every caller derives terms from `prose_terms`
        // (capped at MAX_PROSE_TERMS), so binds stay at 2*64=128, far under
        // SQLite's legacy 999 variable limit. Anything beyond that cap falls
        // back to the full load, which is always correct.
        if unique.len() > crate::lexicon::MAX_PROSE_TERMS {
            return self.all_lexicon_rows();
        }
        if !self.lexicon_is_dirty().unwrap_or(true) {
            if let Ok(Some(bundle)) = self.read_bundle(None) {
                if crate::store::bundle_graph::lexicon_row_count(&bundle) > 0 {
                    let owned: Vec<String> = unique.iter().map(|s| (*s).to_owned()).collect();
                    let packed = crate::store::bundle_graph::lexicon_for_terms(&bundle, &owned);
                    let mut out = Vec::with_capacity(
                        owned.len().saturating_mul(crate::lexicon::MAX_PER_TERM),
                    );
                    for association in packed {
                        let term_chars = association.term.chars().count() as i64;
                        let related_chars = association.related.chars().count() as i64;
                        out.push(Self::decode_lexicon_row(
                            association.term,
                            association.related,
                            association.ppmi,
                            i64::from(association.support),
                            term_chars,
                            related_chars,
                        )?);
                    }
                    if out.len() > crate::lexicon::MAX_PAIRS {
                        return Err(crate::StoreError::Other(format!(
                            "stored lexicon exceeds maximum of {} associations",
                            crate::lexicon::MAX_PAIRS
                        ))
                        .into());
                    }
                    return Ok(out);
                }
            }
        }
        let placeholders = std::iter::repeat_n("?", unique.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT SUBSTR(term, 1, {}), SUBSTR(related, 1, {}), ppmi, support,
                    LENGTH(term), LENGTH(related)
             FROM lexicon
             WHERE term IN ({placeholders}) OR related IN ({placeholders})
             ORDER BY term, related
             LIMIT {}",
            crate::lexicon::MAX_TERM_CHARS,
            crate::lexicon::MAX_TERM_CHARS,
            crate::lexicon::MAX_PAIRS + 1,
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let binds: Vec<&str> = unique
            .iter()
            .copied()
            .chain(unique.iter().copied())
            .collect();
        // One truncated per-term list per query term; fetched rows beyond the
        // per-term truncation bound only cost a reallocation, never correctness.
        Self::collect_lexicon_rows(
            &mut stmt,
            rusqlite::params_from_iter(binds.iter()),
            unique.len().saturating_mul(crate::lexicon::MAX_PER_TERM),
        )
    }

    /// Read a bounded lexicon (ufk7). Ordered so loads are deterministic.
    pub fn all_lexicon_rows(&self) -> Result<Vec<crate::lexicon::Association>> {
        if !self.lexicon_is_dirty().unwrap_or(true) {
            if let Ok(Some(bundle)) = self.read_bundle(None) {
                let packed = crate::store::bundle_graph::lexicon_for_terms(&bundle, &[]);
                if !packed.is_empty() {
                    return Ok(packed);
                }
            }
        }
        let mut stmt = self.conn.prepare_cached(
            "SELECT SUBSTR(term, 1, ?1), SUBSTR(related, 1, ?1), ppmi, support,
                    LENGTH(term), LENGTH(related)
             FROM lexicon
             ORDER BY term, related
             LIMIT ?2",
        )?;
        Self::collect_lexicon_rows(
            &mut stmt,
            params![
                crate::lexicon::MAX_TERM_CHARS as i64,
                crate::lexicon::MAX_PAIRS as i64 + 1
            ],
            crate::lexicon::MAX_PAIRS.min(1024),
        )
    }

    /// Decode and enforce the row cap identically for full and targeted reads.
    /// SQL selection, ordering and initial allocation remain caller-specific.
    fn collect_lexicon_rows(
        stmt: &mut rusqlite::Statement<'_>,
        bind: impl rusqlite::Params,
        capacity: usize,
    ) -> Result<Vec<crate::lexicon::Association>> {
        let rows = stmt.query_map(bind, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;
        let mut out = Vec::with_capacity(capacity);
        for row in rows {
            let (term, related, ppmi, support, term_chars, related_chars) = row?;
            out.push(Self::decode_lexicon_row(
                term,
                related,
                ppmi,
                support,
                term_chars,
                related_chars,
            )?);
        }
        if out.len() > crate::lexicon::MAX_PAIRS {
            return Err(crate::StoreError::Other(format!(
                "stored lexicon exceeds maximum of {} associations",
                crate::lexicon::MAX_PAIRS
            )));
        }
        Ok(out)
    }
}
