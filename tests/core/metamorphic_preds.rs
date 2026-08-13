//! MR predicate leaf helpers for metamorphic relations (pure set/logic).
//! Included from `metamorphic.rs` via `#[path]` — not a Cargo [[test]] target.

use std::collections::BTreeSet;

pub(super) type HitKey = (String, u32, u32);

/// MR predicate: limit-subset -- keys(top_k) ⊆ keys(top_K) for k ≤ K.
pub(super) fn mr_pred_limit_subset(small: &BTreeSet<HitKey>, large: &BTreeSet<HitKey>) -> bool {
    small.is_subset(large)
}

/// MR predicate: probe monotony -- cand(p) ⊆ cand(P) for 1 ≤ p ≤ P.
pub(super) fn mr_pred_probe_monotone(fewer: &BTreeSet<usize>, more: &BTreeSet<usize>) -> bool {
    fewer.is_subset(more)
}

/// MR predicate: scale invariance -- candidate index sequence identical under α>0.
pub(super) fn mr_pred_scale_invariance(bare: &[usize], scaled: &[usize]) -> bool {
    bare == scaled
}

/// MR predicate: lang filter subset -- filtered keys ⊆ unfiltered keys.
pub(super) fn mr_pred_lang_filter_subset(
    filtered: &BTreeSet<HitKey>,
    unfiltered: &BTreeSet<HitKey>,
) -> bool {
    filtered.is_subset(unfiltered)
}

/// MR predicate: reindex idempotence -- hit keys unchanged after reindex.
pub(super) fn mr_pred_reindex_idempotent(
    before: &BTreeSet<HitKey>,
    after: &BTreeSet<HitKey>,
) -> bool {
    before == after
}

/// MR predicate: search_flat prefix equality -- ordered top-k is prefix of top-K.
pub(super) fn mr_pred_search_flat_prefix(small: &[(usize, f32)], large: &[(usize, f32)]) -> bool {
    if small.len() > large.len() {
        return false;
    }
    small
        .iter()
        .zip(large.iter())
        .all(|((i_s, s_s), (i_l, s_l))| i_s == i_l && (s_s - s_l).abs() <= 1e-5)
}

/// MR predicate: multi-term query token-order equivalence -- hit keys equal.
pub(super) fn mr_pred_term_order_equiv(a: &BTreeSet<HitKey>, b: &BTreeSet<HitKey>) -> bool {
    a == b
}

/// MR predicate: orthogonal corpus add -- hit keys unchanged when added file
/// cannot match the query.
pub(super) fn mr_pred_corpus_add_orthogonal(
    before: &BTreeSet<HitKey>,
    after: &BTreeSet<HitKey>,
) -> bool {
    before == after
}
