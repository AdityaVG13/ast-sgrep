//! Rarest-trigram df picker for the literal trigram scan.
//!
//! Document frequencies are baked into the read bundle at index commit. Query
//! time binary-searches that array. No process memo, no static prior, no
//! fts5vocab preload. The SQL control arm (`ASGREP_READPATH=sql`) has no
//! bundle and degrades to the full-phrase MATCH.

use crate::store::IndexStore;

/// Above this many distinct trigrams the needle is already selective enough
/// that extra df lookups cannot pay for themselves.
const MAX_DF_LOOKUPS: usize = 24;
/// Trigram byte length of the trigram tokenizer.
const TRIGRAM_LEN: usize = 3;
/// A df at or below this count is treated as "rare enough".
const RARE_ENOUGH_DF: i64 = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrigramShortcut {
    /// Scan the posting intersection of 1–2 rarest needle trigrams.
    Match(Vec<String>),
    /// No trustworthy df data: scan with the full-phrase MATCH.
    Full,
}

/// Shortcut decision for scanning `needle`. Never errors: every uncertain
/// outcome degrades to [`TrigramShortcut::Full`].
pub fn scan_shortcut(store: &IndexStore, needle: &str) -> TrigramShortcut {
    if !needle.is_ascii() {
        return TrigramShortcut::Full;
    }
    let needle_lower = needle.to_lowercase();
    let Some(trigrams) = distinct_trigrams(&needle_lower) else {
        return TrigramShortcut::Full;
    };
    let Ok(Some(bundle)) = store.read_bundle(None) else {
        return TrigramShortcut::Full;
    };
    if !crate::store::bundle_graph::df_section_present(&bundle) {
        return TrigramShortcut::Full;
    }
    let mut ranked: Vec<(i64, &str)> = Vec::with_capacity(trigrams.len());
    for tri in &trigrams {
        let Some(df) = crate::store::bundle_graph::trigram_df(&bundle, tri) else {
            return TrigramShortcut::Full;
        };
        ranked.push((df, tri));
    }
    pick_shortcut(&ranked)
}

/// Smallest trigram document frequency for `needle`. `None` means uncertain.
pub fn min_df(store: &IndexStore, needle: &str) -> Option<i64> {
    if !needle.is_ascii() {
        return None;
    }
    let needle_lower = needle.to_lowercase();
    let trigrams = distinct_trigrams(&needle_lower)?;
    let bundle = store.read_bundle(None).ok().flatten()?;
    trigrams
        .iter()
        .map(|tri| crate::store::bundle_graph::trigram_df(&bundle, tri))
        .collect::<Option<Vec<i64>>>()?
        .into_iter()
        .min()
}

/// Per-trigram df for ordering ASCII anchors. Missing bundle df sorts last.
pub fn df_or_unknown(store: &IndexStore, tri: &str) -> i64 {
    let Ok(Some(bundle)) = store.read_bundle(None) else {
        return i64::MAX;
    };
    crate::store::bundle_graph::trigram_df(&bundle, tri).unwrap_or(i64::MAX)
}

/// True when baked per-index df is available (exact, so the scan is unbounded).
pub fn df_ready(store: &IndexStore) -> bool {
    store
        .read_bundle(None)
        .ok()
        .flatten()
        .is_some_and(|bundle| crate::store::bundle_graph::df_section_present(&bundle))
}

/// Pick 1–2 rarest trigrams whose smallest df is rare enough to shortcut.
pub fn pick_shortcut(ranked: &[(i64, &str)]) -> TrigramShortcut {
    if ranked.is_empty() {
        return TrigramShortcut::Full;
    }
    let mut ranked = ranked.to_vec();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    if ranked[0].0 > RARE_ENOUGH_DF {
        return TrigramShortcut::Full;
    }
    let mut terms = vec![ranked[0].1.to_string()];
    if ranked.len() > 1 && ranked[0].1 != ranked[1].1 {
        terms.push(ranked[1].1.to_string());
    }
    TrigramShortcut::Match(terms)
}

/// Distinct lowercased ASCII trigrams, or None for non-ASCII, short, or
/// over-budget needles.
pub fn distinct_trigrams(needle_lower: &str) -> Option<Vec<&str>> {
    let bytes = needle_lower.as_bytes();
    if bytes.len() < TRIGRAM_LEN {
        return None;
    }
    let count = bytes.len() - TRIGRAM_LEN + 1;
    if count > MAX_DF_LOOKUPS || !needle_lower.is_ascii() {
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
