//! Rarest-trigram df picker for the literal trigram scan (bead br-umh).
//!
//! The scan's cost grows with TERM LENGTH because the FTS5 phrase machinery
//! intersects every trigram of the needle. Picking only the rarest trigram as
//! the MATCH term bounds that cost to a single posting list; the existing
//! `content_matches_literal` reverify in `passes::literal` keeps output
//! exact (subset postings are a superset of phrase matches by construction:
//! every line containing the full needle necessarily contains each of its
//! trigrams, and FTS5 phrase matching is itself trigram-intersection).
//!
//! Document frequencies come from an ephemeral `temp` fts5vocab virtual
//! table over the live `lines_trigram` index — no persisted sidecar, so the
//! df view can never drift from any writer path (insert, delete,
//! bulk rebuild). Results are memoized per store keyed on
//! `index_data_version`; every miss or error degrades silently to the
//! previous full-phrase MATCH behavior.
use crate::store::IndexStore;
use rusqlite::OptionalExtension as _;
use std::collections::HashMap;
use std::sync::Mutex;

/// Vocab table name inside the temp schema. `IF NOT EXISTS` keeps steady-state
/// ensure cost sub-microsecond after first use on a connection.
const VOCAB_TABLE: &str = "temp.asgrep_trigram_vocab";
/// Ephemeral fts5vocab instance over the live external-content trigram field.
/// 'row' variant: (term TEXT PRIMARY KEY, doc INTEGER, cnt INTEGER) with doc =
/// number of distinct indexed rows containing the term.
const VOCAB_DDL: &str = "CREATE VIRTUAL TABLE IF NOT EXISTS temp.asgrep_trigram_vocab \
                         USING fts5vocab('main', 'lines_trigram', 'row')";
/// Above this many distinct trigrams the needle is already selective enough
/// that extra df lookups cannot pay for themselves (~34us per lookup measured).
const MAX_DF_LOOKUPS: usize = 24;
/// Trigram byte length of the trigram tokenizer.
const TRIGRAM_LEN: usize = 3;
/// A df at or below this count is treated as "rare enough". Tuned by A/B on
/// the self corpus (benchmarks/results/speed.md::2026-08-23 trigram df):
/// 256 rarely engaged (excludes p75-p90 trigrams); 4096 engaged everywhere
/// and won ~20% p50; 2048 keeps the win while bounding the worst-case
/// single-trigram scan (~2k rows x ~1us) on corpora far larger than this one.
const RARE_ENOUGH_DF: i64 = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TrigramShortcut {
    /// Scan only the rarest trigram's posting list. Safety argument: only
    /// trigrams DERIVED FROM THE NEEDLE are ever candidates, so poisoned or
    /// stale document frequencies can influence WHICH trigram is scanned but
    /// never what the scan reads; every line containing the full needle
    /// necessarily contains each of its trigrams, so any candidate's posting
    /// list is a superset of true matches, and the caller's
    /// `content_matches_literal` reverify restores exactness. Absence is
    /// therefore inferable safely: an empty reverified scan proves no line
    /// contains the needle (RED-proven by c2b/c3 regressions).
    Match(String),
    /// No trustworthy df data (or no rare trigram): scan with the previous
    /// full-phrase MATCH. Identical to pre-lever behavior.
    Full,
}

#[derive(Default)]
struct DfCacheInner {
    /// gen when the vocab table was last ensured + per-term document counts.
    entries: HashMap<String, i64>,
}

/// Per-Searcher memoization of trigram document frequencies. Invalidated by
/// generation bump; never authoritative (all misses fall back).
pub(crate) struct TrigramDfCache {
    inner: Mutex<DfState>,
}

struct DfState {
    cache: DfCacheInner,
    gen: i64,
    /// Set once the vocab table could not be created (e.g. SQLite built
    /// without fts5vocab): stop retrying for this store generation.
    unavailable: bool,
}

impl TrigramDfCache {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(DfState {
                cache: DfCacheInner {
                    entries: HashMap::new(),
                },
                gen: 0,
                unavailable: false,
            }),
        }
    }

    /// Shortcut decision for scanning `needle`, per the contract on
    /// [`TrigramShortcut`]. Never errors: every uncertain outcome degrades to
    /// [`TrigramShortcut::Full`], preserving pre-lever behavior.
    pub(crate) fn scan_shortcut(&self, store: &IndexStore, needle: &str) -> TrigramShortcut {
        // The trigram tokenizer case-folds; ASCII lowercase folding is exact,
        // but Unicode folding is not reproduced here, so restrict the fast
        // path to pure-ASCII needles where fold identity holds.
        if !needle.is_ascii() {
            return TrigramShortcut::Full;
        }
        let needle_lower = needle.to_lowercase();
        let Some(trigrams) = distinct_trigrams(&needle_lower) else {
            return TrigramShortcut::Full;
        };
        let Ok(mut state) = self.inner.lock() else {
            return TrigramShortcut::Full;
        };
        let gen = match store.index_data_version() {
            Ok(gen) => gen,
            // Unreadable generation: no trustworthy invalidation signal.
            Err(_) => return TrigramShortcut::Full,
        };
        let cache_valid = state.gen == gen;
        if state.unavailable && cache_valid {
            return TrigramShortcut::Full;
        }
        if !cache_valid {
            if ensure_vocab_table(store).is_err() {
                state.unavailable = true;
                state.gen = gen;
                return TrigramShortcut::Full;
            }
            state.unavailable = false;
            state.gen = gen;
            state.cache.entries.clear();
        }
        let conn = store.connection();
        // Sequential probe-and-stop: ask only for the df values needed to
        // find ONE rare-enough trigram. Cached answers are free; each miss
        // costs one point lookup (~35us measured), so a needle whose first
        // probed trigram is rare pays a single lookup. A df of 0 is NOT
        // trusted as "absent" (poisonable within one generation); it just
        // wins the rarity contest, and the caller's reverify keeps the scan
        // exact while its empty result proves absence.
        let mut best: Option<(i64, &str)> = None;
        for tri in &trigrams {
            let df = match state.cache.entries.get(*tri) {
                Some(df) => *df,
                None => {
                    let Some(df) = fetch_one(conn, tri) else {
                        // Unknown df (lookup failed): abandon the fast path —
                        // never confuse "unknown" with "absent".
                        return TrigramShortcut::Full;
                    };
                    state.cache.entries.insert((*tri).to_string(), df);
                    df
                }
            };
            let better = match best {
                None => true,
                Some((bd, _)) => df < bd,
            };
            if better {
                best = Some((df, tri));
            }
            if best.is_some_and(|(bd, _)| bd <= RARE_ENOUGH_DF) {
                break;
            }
        }
        match best {
            Some((df, tri)) if df <= RARE_ENOUGH_DF => TrigramShortcut::Match((*tri).to_string()),
            _ => TrigramShortcut::Full,
        }
    }
}

/// Distinct lowercased trigrams, or None when the needle is too short for a
/// trigram or has too many for the df probe budget.
fn distinct_trigrams(needle_lower: &str) -> Option<Vec<&str>> {
    let bytes = needle_lower.as_bytes();
    if bytes.len() < TRIGRAM_LEN {
        return None;
    }
    let count = bytes.len() - TRIGRAM_LEN + 1;
    if count > MAX_DF_LOOKUPS {
        return None;
    }
    let mut seen = std::collections::HashSet::with_capacity(count);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let tri = &needle_lower[i..i + TRIGRAM_LEN];
        if seen.insert(tri) {
            out.push(tri);
        }
    }
    Some(out)
}

fn ensure_vocab_table(store: &IndexStore) -> Result<(), crate::StoreError> {
    let conn = store.connection();
    // Name-collision defense (RED-proven by c2_decoy_vocab_table_is_not_trusted):
    // a same-named temp vtab created by other in-tree code would hand us its
    // vocabulary as if it were ours. Drop any squatter before creating.
    conn.execute("DROP TABLE IF EXISTS temp.asgrep_trigram_vocab", [])
        .map_err(|e| crate::StoreError::Other(format!("fts5vocab unavailable: {e}")))?;
    conn.execute_batch(VOCAB_DDL)
        .map_err(|e| crate::StoreError::Other(format!("fts5vocab unavailable: {e}")))
}

/// Fetch a single term's document count. None means "unknown" (lookup or
/// decode failure) — distinct from a genuine df of 0, which the vocab reports
/// only as an absent row; callers treat None as fall-back-to-phrase and a 0
/// as merely the best rarity candidate (never trusted absence).
fn fetch_one(conn: &rusqlite::Connection, term: &str) -> Option<i64> {
    let sql = format!("SELECT doc FROM {VOCAB_TABLE} WHERE term = ?1");
    let mut stmt = conn.prepare_cached(&sql).ok()?;
    // No row = genuinely absent from the vocabulary = zero documents.
    stmt.query_row(rusqlite::params![term], |row| row.get::<_, i64>(0))
        .optional()
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_trigram_extraction_dedups_and_bounds() {
        let tris = distinct_trigrams("process_request").unwrap();
        // 15 chars -> 13 sliding windows; none repeat.
        assert_eq!(tris.len(), 13);
        assert_eq!(tris.first(), Some(&"pro"));
        assert_eq!(tris.last(), Some(&"est"));
        assert!(distinct_trigrams("ab").is_none());
        assert!(distinct_trigrams("").is_none());
        let long = "x".repeat(40);
        assert!(distinct_trigrams(&long).is_none(), "over lookup budget");
    }
}
