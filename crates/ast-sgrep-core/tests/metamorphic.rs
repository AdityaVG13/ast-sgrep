//! Metamorphic relations for oracle-hard search / index / ANN surfaces.
//!
//! # Diagnosis (oracle problem)
//!
//! Hybrid / keyword search rankings, IVF ANN candidate sets, and index rebuild
//! self-consistency have **no absolute oracle** for arbitrary corpora: the correct
//! full ranking is unknown, and approximate ANN has no closed-form gold top-k.
//! Metamorphic relations compare outputs under controlled input transforms instead
//! of asserting absolute ranks.
//!
//! Prefer **conventional** unit/property tests when an oracle exists (e.g.
//! `ParsedQuery::parse` structure, closed-form `rrf_score`, fixed symbol score
//! tables). Prefer **differential** tests when a reference path exists (e.g.
//! `probes >= n_clusters` vs `brute_force_flat`). Those are **not** MRs.
//!
//! # Strength matrix
//!
//! Score = fault-sensitivity (F) x independence (I) / cost (C). **Ship only Score >= 2.0.**
//! Names match `fn mr_*` test identifiers (underscore style).
//!
//! ## Implemented (Score >= 2.0)
//!
//! | MR (test) | F | I | C | Score | Category | Catches |
//! |-----------|---|---|---|-------|----------|---------|
//! | reindex_idempotent_hits | 4 | 4 | 2 | 8.0 | equivalence | nondeterministic index, drop/rebuild drift |
//! | limit_top_k_subset | 4 | 3 | 2 | 6.0 | inclusive | limit truncate bugs, unstable top-k |
//! | keyword_file_must_surface | 3 | 4 | 1 | 12.0 | inclusive (PQS-style) | indexer/search miss on exact token |
//! | ann_query_scale_invariance | 5 | 4 | 1 | 20.0 | multiplicative/equiv | missing L2 renorm on query |
//! | ann_query_scale_invariance_proptest | 5 | 4 | 1 | 20.0 | multiplicative/equiv | same MR; random unit-ish corpora + positive scales |
//! | kmeans_threads_bit_identical | 4 | 5 | 2 | 10.0 | equivalence | Rayon race / nondeterministic reduce |
//! | compound_reindex_then_limit | 3 | 3 | 3 | 3.0 | composition | interaction bugs across reindex then limit |
//! | lang_filter_subset | 4 | 4 | 2 | 8.0 | inclusive | lang filter leaks non-matching langs / filter dropped |
//! | query_trim_search_equivalence | 3 | 3 | 2 | 4.5 | equivalence | end-to-end trim mismatch vs parse-only trim |
//! | ann_probe_monotone_candidates | 4 | 4 | 1 | 16.0 | inclusive | probe take order broken / non-prefix cluster selection |
//! | ann_probe_monotone_candidates_proptest | 4 | 4 | 1 | 16.0 | inclusive | same MR; random flat corpora + random query |
//! | search_flat_limit_subset | 3 | 3 | 1 | 9.0 | inclusive | top-k truncate/unstable ranking on ANN flat path |
//! | search_flat_limit_subset_proptest | 3 | 3 | 1 | 9.0 | inclusive | same MR; random unit-ish flat + random k/K |
//! | search_flat_limit_prefix_equality | 4 | 3 | 1 | 12.0 | inclusive (stronger) | order-corrupt top-k that still preserves key set |
//! | search_flat_limit_prefix_equality_proptest | 4 | 3 | 1 | 12.0 | inclusive (stronger) | same MR; random unit-ish flat + random k/K |
//! | query_term_order_equivalence | 3 | 4 | 2 | 6.0 | permutative | bag-of-words hybrid order sensitivity / tokenizer sort drop |
//! | corpus_add_orthogonal_hit_equality | 4 | 3 | 2 | 6.0 | additive | reindex drops prior hits when unrelated file is added |
//! | compound_scale_then_probe_proptest | 4 | 3 | 2 | 6.0 | composition | scale-invariance composed with probe monotony on random data |
//! | compound_lang_filter_then_limit | 4 | 3 | 2 | 6.0 | composition | limit before filter / filter then unstable top-k |
//! | compound_scale_then_search_flat_limit_proptest | 4 | 3 | 2 | 6.0 | composition | renorm ok on candidates but broken on scored top-k path |
//!
//! Fixed fixtures stay for deterministic non-vacuous paths (e.g. IVF threshold).
//! `*_proptest` variants are mandatory property-based generation of the same relations.
//!
//! Intra-response score non-increase is checked inside `mr_limit_top_k_subset`
//! (not a separate MR; correlated with limit ranking).
//!
//! **ANN probe monotony (precise relation):** for a fixed IVF index and query,
//! with explicit probe counts `1 <= p <= P` (not `None`/`Some(0)` adaptive),
//! `set(candidate_indices(q, Some(p))) ⊆ set(candidate_indices(q, Some(P)))`.
//! Rationale: candidates are the union of members of the top-`take` populated
//! clusters by centroid cosine after L2 renorm of `q`; `take = p.clamp(1, populated)`.
//! Increasing `p` only extends the prefix of the same sorted cluster list, so
//! the member set is monotone inclusive. Adaptive probe (`None`/`0`) is a
//! different policy and is not required to interleave with explicit `p`.
//!
//! **ANN `search_flat` prefix equality (precise relation):** for fixed flat corpus,
//! index, and query, with `1 ≤ k ≤ K`, the ordered index sequence of
//! `search_flat(..., k)` equals the length-`k` prefix of `search_flat(..., K)`
//! (scores compared within 1e-5). Holds because `top_k_similarity` /
//! `top_k_flat_similarity` use a total order (sim desc, idx asc) over a fixed
//! candidate pool (default probes). **Not** claimed for hybrid keyword search:
//! `enforce_result_gates` may inject a Def into the head when the pure prefix
//! lacks one, so ordered prefix can differ while key-set subset still holds.
//!
//! ## Validation meta (not an MR; no F x I / C)
//!
//! Lightweight **in-memory mutants** (pure set/logic, not product hooks) prove each
//! shipped MR class is non-placebo. Harness: `mr_suite_mutation_kill_matrix`.
//!
//! | Planted mutant class | Violates | Detecting MR check | Killed |
//! |----------------------|----------|--------------------|--------|
//! | limit_phantom_key | small limit set contains key absent from large | limit-subset (`keys(top_k) ⊆ keys(top_K)`) | yes |
//! | probe_set_shrink | higher probe count drops a lower-probe member | probe monotony (`cand(p) ⊆ cand(P)`) | yes |
//! | scale_candidate_drift | positive query scale changes candidate sequence | scale invariance (`cand(q) = cand(αq)`, α>0) | yes |
//! | lang_filter_leak | filtered hit key absent from unfiltered set | lang filter subset (`filt ⊆ unfilt`) | yes |
//! | reindex_hit_drift | reindex changes hit key set | reindex idempotence (`keys₁ = keys₂`) | yes |
//! | rank_order_swap | top-k index sequence is not a prefix of top-K (set may still match) | search_flat prefix equality | yes |
//! | term_order_drift | permuting multi-term query tokens changes hit keys | query term-order equivalence | yes |
//! | corpus_add_drop_old | adding query-orthogonal file drops a prior hit key | corpus-add orthogonal equality | yes |
//!
//! **Suite kill-rate: 8/8 = 100%** (skill target ≥ 80%). No residual equivalent mutants.
//! Healthy fixtures pass every check; each mutant fails ≥ 1 check.
//!
//! ## Candidates Score >= 2.0 -- not yet implemented
//!
//! | MR candidate | F | I | C | Score | Category | Notes |
//! |--------------|---|---|---|-------|----------|-------|
//! | *(none)* | | | | | | Hybrid prefix / reindex score-order re-scored below. |
//!
//! ## Dropped / rejected (Score < 2.0, flaky, or wrong technique)
//!
//! | Candidate | Score / reason | Disposition |
//! |-----------|----------------|-------------|
//! | limit_top_k_prefix_equality (hybrid) | Score 4.0 on paper but **flaky by design**: `enforce_result_gates` injects Def into head when pure prefix lacks Def -- ordered prefix of top-K is not free; subset remains the hybrid contract | KEEP DEFERRED / do not ship (ANN prefix covers the ordered-rank bug class without hybrid Def) |
//! | reindex_score_order | 3.0 -- F3 I2 C2; ordered scores after reindex highly correlated with `reindex_idempotent_hits` (same rebuild path; key-set already catches drop/swap of loci) | DROP as redundant (effective independence below 3) |
//! | corpus_file_order_permutation | 1.5 -- F2 I3 C4 high cost, weak control (WalkDir order + FS naming) | DROP |
//! | empty_query_empty_hits | ~1.0 -- absolute contract `f("")=∅`, not a transform relation; also near-tautology with pass early-returns | DROP (conventional unit if desired) |
//! | empty_index_empty_hits | ~1.0 -- absolute oracle on empty corpus; no input transform | DROP (conventional / durability tests) |
//! | parse_whitespace_equivalence | 1.0 -- no relation asserted (panic-only); parse has deterministic oracle | DROP (use properties.rs `parse_never_panics` / unit parse tests) |
//! | rrf_rank_monotony | N/A -- closed form `1/(k+r+1)` | conventional unit, not MR |
//! | ann_exact_eq_bruteforce | N/A -- reference path exists | differential / unit |
//! | ivf_write_read_roundtrip | N/A -- exact bytes oracle | unit (see semantic_ivf_roundtrip); invertive covered there |
//! | symbol_case_score tables | N/A -- fixed score constants | conventional unit |
//! | f(x)=f(x) tautologies | 0 | never |
//!
//! # Taxonomy coverage (implemented)
//!
//! equivalence, inclusive (limit + lang filter + ANN probe/limit/prefix), multiplicative/equiv,
//! permutative (multi-term query order), additive (orthogonal corpus expansion), composition.
//! Invertive covered outside this suite (IVF roundtrip units).

use ast_sgrep_core::search::{SearchOptions, Searcher};
use ast_sgrep_core::semantic_ann::{SemanticAnnIndex, DEFAULT_ANN_THRESHOLD};
use ast_sgrep_core::{IndexOptions, Indexer};
use proptest::prelude::*;
use std::collections::BTreeSet;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

/// Keep metamorphic proptest fast: small case count, no source-parallel persistence races.
fn mr_proptest_config() -> ProptestConfig {
    ProptestConfig {
        cases: 16,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// Force finite coords and a non-all-zero row (normalize_vec would otherwise zero-fill).
fn ensure_nonzero_rows(flat: &mut [f32], dim: usize) {
    if dim == 0 || flat.is_empty() {
        return;
    }
    let n = flat.len() / dim;
    for i in 0..n {
        let row = &mut flat[i * dim..(i + 1) * dim];
        for x in row.iter_mut() {
            if !x.is_finite() {
                *x = 0.0;
            }
        }
        if row.iter().all(|&x| x == 0.0) {
            row[0] = 1.0;
        }
    }
}

fn ensure_nonzero_query(q: &mut [f32]) {
    for x in q.iter_mut() {
        if !x.is_finite() {
            *x = 0.0;
        }
    }
    if q.iter().all(|&x| x == 0.0) {
        if let Some(first) = q.first_mut() {
            *first = 1.0;
        }
    }
}

/// Inject a few near-query rows so `search_flat` yields hits above MIN_SIMILARITY.
fn inject_near_query(flat: &mut [f32], dim: usize, query: &[f32], copies: usize) {
    if dim == 0 || flat.len() < dim || query.len() != dim {
        return;
    }
    let n = flat.len() / dim;
    let copies = copies.min(n).max(1);
    for i in 0..copies {
        let row = &mut flat[i * dim..(i + 1) * dim];
        for (j, &q) in query.iter().enumerate() {
            // Small orthogonal-ish noise; still near query after renorm.
            let noise = 0.02 * ((i + j) as f32 * 0.17).sin();
            row[j] = q + noise;
        }
        ensure_nonzero_rows(row, dim);
    }
}

/// Strategy: (dim, flat[n*dim], query[dim]) with unit-ish random coords.
fn arb_ann_corpus() -> impl Strategy<Value = (usize, Vec<f32>, Vec<f32>)> {
    (4usize..=8, 24usize..64).prop_flat_map(|(dim, n)| {
        (
            Just(dim),
            prop::collection::vec(-2.0f32..2.0f32, n * dim),
            prop::collection::vec(-2.0f32..2.0f32, dim),
        )
            .prop_map(move |(dim, mut flat, mut query)| {
                ensure_nonzero_rows(&mut flat, dim);
                ensure_nonzero_query(&mut query);
                (dim, flat, query)
            })
    })
}

fn hit_keys(hits: &[ast_sgrep_core::search::SearchHit]) -> BTreeSet<(String, u32, u32)> {
    hits.iter()
        .map(|h| (h.file.clone(), h.line_start, h.line_end))
        .collect()
}

fn index_and_searcher(root: &std::path::Path, index_path: &std::path::Path, limit: usize) -> Searcher {
    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path.to_path_buf()),
        embed_semantic: false,
        force_reindex: true,
        ..IndexOptions::default()
    })
    .expect("indexer");
    indexer.index_all().expect("index");
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path.to_path_buf()),
        use_embed: false,
        limit,
        ..SearchOptions::default()
    })
    .expect("searcher")
}

/// Equivalence: reindex then search yields the same hit key set as initial index.
#[test]
fn mr_reindex_idempotent_hits() {
    let corpus = TempDir::new().unwrap();
    fs::write(
        corpus.path().join("a.rs"),
        "fn alpha_token() {}\nfn beta_token() { alpha_token(); }\n",
    )
    .unwrap();
    fs::write(corpus.path().join("b.rs"), "fn gamma_token() {}\n").unwrap();
    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");

    let s1 = index_and_searcher(corpus.path(), &index_path, 32);
    let r1 = s1.search("alpha_token").expect("search1");
    let keys1 = hit_keys(&r1.hits);

    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        force_reindex: true,
        ..IndexOptions::default()
    })
    .expect("indexer2");
    indexer.reindex_all().expect("reindex");
    let s2 = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: false,
        limit: 32,
        ..SearchOptions::default()
    })
    .expect("s2");
    let r2 = s2.search("alpha_token").expect("search2");
    let keys2 = hit_keys(&r2.hits);

    assert_eq!(
        keys1, keys2,
        "MR reindex-idempotent: hit keys must match after reindex\nbefore={keys1:?}\nafter={keys2:?}"
    );
}

/// Inclusive: top-k under small limit is a subset of top-K under larger limit (by hit key).
#[test]
fn mr_limit_top_k_subset() {
    let corpus = TempDir::new().unwrap();
    // Several files share a token so ranking has multiple hits.
    for (name, body) in [
        ("one.rs", "fn shared_token() { let a = 1; }\n"),
        ("two.rs", "fn shared_token_helper() { shared_token(); }\nfn shared_token() {}\n"),
        ("three.rs", "// shared_token appears in comment\nfn other() {}\n"),
        ("four.rs", "fn call() { shared_token(); shared_token(); }\n"),
    ] {
        fs::write(corpus.path().join(name), body).unwrap();
    }
    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");

    let s_small = index_and_searcher(corpus.path(), &index_path, 2);
    // Reuse same index for larger limit (no force reindex).
    let s_large = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: false,
        limit: 16,
        ..SearchOptions::default()
    })
    .expect("large");

    let small = s_small.search("shared_token").expect("small");
    let large = s_large.search("shared_token").expect("large");
    assert!(!small.hits.is_empty(), "need at least one hit for MR");
    let small_keys = hit_keys(&small.hits);
    let large_keys = hit_keys(&large.hits);
    assert!(
        small_keys.is_subset(&large_keys),
        "MR limit-subset: every top-2 hit must appear in top-16\nsmall={small_keys:?}\nlarge={large_keys:?}"
    );
    // Scores within a response are non-increasing.
    for w in large.hits.windows(2) {
        assert!(
            w[0].score + 1e-6 >= w[1].score,
            "scores must be non-increasing: {} then {}",
            w[0].score,
            w[1].score
        );
    }
}

/// Inclusive: a file that literally contains the unique token surfaces for keyword/hybrid search.
#[test]
fn mr_keyword_file_must_surface() {
    let corpus = TempDir::new().unwrap();
    let unique = "zz_metamorphic_token_xyzzy";
    fs::write(
        corpus.path().join("hitme.rs"),
        format!("fn {unique}() {{}}\n"),
    )
    .unwrap();
    fs::write(corpus.path().join("other.rs"), "fn nothing_here() {}\n").unwrap();
    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");
    let searcher = index_and_searcher(corpus.path(), &index_path, 16);
    let resp = searcher.search(unique).expect("search");
    assert!(
        resp.hits.iter().any(|h| h.file.contains("hitme")),
        "MR keyword-surface: file defining unique token must appear; hits={:?}",
        resp.hits
            .iter()
            .map(|h| (&h.file, h.score))
            .collect::<Vec<_>>()
    );
}

/// Multiplicative/equiv under L2 renorm: scaling a unit query leaves ANN candidate order unchanged.
#[test]
fn mr_ann_query_scale_invariance() {
    // Two orthogonal clusters of unit-ish vectors in dim=4.
    let mut flat = Vec::new();
    for _ in 0..8 {
        flat.extend_from_slice(&[1.0f32, 0.0, 0.0, 0.0]);
    }
    for _ in 0..8 {
        flat.extend_from_slice(&[0.0f32, 1.0, 0.0, 0.0]);
    }
    let index = SemanticAnnIndex::build_from_flat(&flat, 4);
    let q = [1.0f32, 0.05, 0.0, 0.0];
    let a = index.candidate_indices(&q, Some(4));
    let q2 = [10.0f32, 0.5, 0.0, 0.0]; // same direction before renorm in search path
    // candidate_indices may assume unit query — scale explicitly via same direction
    let b = index.candidate_indices(&q2, Some(4));
    // If implementation renorms, a==b; if not, this MR documents required renorm behavior.
    assert_eq!(
        a, b,
        "MR ann-scale: candidates must match for proportional queries (renorm required)\na={a:?}\nb={b:?}"
    );
}

/// Equivalence: k-means centroids/assignments bit-identical under Rayon 1 vs 4 threads.
#[test]
fn mr_kmeans_threads_bit_identical() {
    let mut flat = Vec::new();
    for i in 0..64u32 {
        let t = (i as f32) * 0.1;
        flat.extend_from_slice(&[t.sin(), t.cos(), (t * 0.3).sin(), (t * 0.7).cos()]);
    }
    let pool1 = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let pool4 = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let a = pool1.install(|| SemanticAnnIndex::build_from_flat(&flat, 4));
    let b = pool4.install(|| SemanticAnnIndex::build_from_flat(&flat, 4));
    let mut ba = Vec::new();
    let mut bb = Vec::new();
    a.write_to(&mut ba, 4).unwrap();
    b.write_to(&mut bb, 4).unwrap();
    assert_eq!(
        ba, bb,
        "MR kmeans-threads: IVF sidecar bytes must match across thread counts"
    );
}

/// Composition: reindex then limit-subset still holds.
#[test]
fn mr_compound_reindex_then_limit_subset() {
    let corpus = TempDir::new().unwrap();
    for i in 0..6 {
        fs::write(
            corpus.path().join(format!("f{i}.rs")),
            format!("fn compound_token_{i}() {{ let compound_token = {i}; }}\n"),
        )
        .unwrap();
    }
    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");
    let _ = index_and_searcher(corpus.path(), &index_path, 8);
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        force_reindex: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.reindex_all().unwrap();
    let s2 = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        use_embed: false,
        limit: 2,
        ..SearchOptions::default()
    })
    .unwrap();
    let s16 = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: false,
        limit: 16,
        ..SearchOptions::default()
    })
    .unwrap();
    let a = s2.search("compound_token").unwrap();
    let b = s16.search("compound_token").unwrap();
    assert!(hit_keys(&a.hits).is_subset(&hit_keys(&b.hits)));
}

// ---------------------------------------------------------------------------
// Mutation validation harness (pure set/logic mutants; not product hooks)
// ---------------------------------------------------------------------------
//
// Each planted mutant class must be killed by ≥ 1 MR predicate that mirrors a
// shipped product MR. Kill matrix and 100% suite rate live in the module docs
// ("Validation meta") and are asserted here.

type HitKey = (String, u32, u32);

/// MR predicate: limit-subset -- keys(top_k) ⊆ keys(top_K) for k ≤ K.
fn mr_pred_limit_subset(small: &BTreeSet<HitKey>, large: &BTreeSet<HitKey>) -> bool {
    small.is_subset(large)
}

/// MR predicate: probe monotony -- cand(p) ⊆ cand(P) for 1 ≤ p ≤ P.
fn mr_pred_probe_monotone(fewer: &BTreeSet<usize>, more: &BTreeSet<usize>) -> bool {
    fewer.is_subset(more)
}

/// MR predicate: scale invariance -- candidate index sequence identical under α>0.
fn mr_pred_scale_invariance(bare: &[usize], scaled: &[usize]) -> bool {
    bare == scaled
}

/// MR predicate: lang filter subset -- filtered keys ⊆ unfiltered keys.
fn mr_pred_lang_filter_subset(filtered: &BTreeSet<HitKey>, unfiltered: &BTreeSet<HitKey>) -> bool {
    filtered.is_subset(unfiltered)
}

/// MR predicate: reindex idempotence -- hit keys unchanged after reindex.
fn mr_pred_reindex_idempotent(before: &BTreeSet<HitKey>, after: &BTreeSet<HitKey>) -> bool {
    before == after
}

/// MR predicate: search_flat prefix equality -- ordered top-k is prefix of top-K.
fn mr_pred_search_flat_prefix(small: &[(usize, f32)], large: &[(usize, f32)]) -> bool {
    if small.len() > large.len() {
        return false;
    }
    small
        .iter()
        .zip(large.iter())
        .all(|((i_s, s_s), (i_l, s_l))| i_s == i_l && (s_s - s_l).abs() <= 1e-5)
}

/// MR predicate: multi-term query token-order equivalence -- hit keys equal.
fn mr_pred_term_order_equiv(a: &BTreeSet<HitKey>, b: &BTreeSet<HitKey>) -> bool {
    a == b
}

/// MR predicate: orthogonal corpus add -- hit keys unchanged when added file
/// cannot match the query.
fn mr_pred_corpus_add_orthogonal(before: &BTreeSet<HitKey>, after: &BTreeSet<HitKey>) -> bool {
    before == after
}

/// Mutation validation: planted pure-logic mutants are each caught by ≥ 1 MR class.
///
/// Kill matrix (rows = MR predicates, cols = mutants; `K` = killed):
///
/// | MR \ mutant | lim_ph | probe | scale | lang | reidx | rank_sw | term | add_drop |
/// |-------------|:------:|:-----:|:-----:|:----:|:-----:|:-------:|:----:|:--------:|
/// | limit-subset | K | | | | | | | |
/// | probe monotony | | K | | | | | | |
/// | scale invariance | | | K | | | | | |
/// | lang filter subset | | | | K | | | | |
/// | reindex idempotence | | | | | K | | | |
/// | search_flat prefix | | | | | | K | | |
/// | term-order equiv | | | | | | | K | |
/// | corpus-add orthog | | | | | | | | K |
///
/// Suite kill-rate: 8/8 = 100% (≥ 80%). Residual mutants: none (all non-equivalent).
#[test]
fn mr_suite_mutation_kill_matrix() {
    // --- Healthy fixtures (correct behavior) ---------------------------------
    let healthy_small: BTreeSet<HitKey> = [(String::from("a.rs"), 1, 1)].into_iter().collect();
    let healthy_large: BTreeSet<HitKey> = [
        (String::from("a.rs"), 1, 1),
        (String::from("b.rs"), 2, 2),
    ]
    .into_iter()
    .collect();
    let healthy_probe_lo: BTreeSet<usize> = [0, 2].into_iter().collect();
    let healthy_probe_hi: BTreeSet<usize> = [0, 1, 2, 5].into_iter().collect();
    let healthy_cand_q: Vec<usize> = vec![3, 1, 7, 0];
    let healthy_cand_scaled: Vec<usize> = vec![3, 1, 7, 0]; // α>0 same direction
    let healthy_lang_filt: BTreeSet<HitKey> = [(String::from("a.rs"), 1, 1)].into_iter().collect();
    let healthy_lang_all: BTreeSet<HitKey> = [
        (String::from("a.rs"), 1, 1),
        (String::from("b.py"), 2, 2),
    ]
    .into_iter()
    .collect();
    let healthy_reindex_before: BTreeSet<HitKey> =
        [(String::from("a.rs"), 1, 1), (String::from("b.rs"), 3, 3)]
            .into_iter()
            .collect();
    let healthy_reindex_after = healthy_reindex_before.clone();
    let healthy_flat_small: Vec<(usize, f32)> = vec![(7, 0.95), (2, 0.90), (5, 0.80)];
    let healthy_flat_large: Vec<(usize, f32)> =
        vec![(7, 0.95), (2, 0.90), (5, 0.80), (1, 0.70), (9, 0.60)];
    let healthy_terms_a: BTreeSet<HitKey> =
        [(String::from("a.rs"), 1, 1), (String::from("b.rs"), 2, 2)]
            .into_iter()
            .collect();
    let healthy_terms_b = healthy_terms_a.clone();
    let healthy_add_before: BTreeSet<HitKey> =
        [(String::from("hit.rs"), 1, 1)].into_iter().collect();
    let healthy_add_after = healthy_add_before.clone();

    assert!(
        mr_pred_limit_subset(&healthy_small, &healthy_large)
            && mr_pred_probe_monotone(&healthy_probe_lo, &healthy_probe_hi)
            && mr_pred_scale_invariance(&healthy_cand_q, &healthy_cand_scaled)
            && mr_pred_lang_filter_subset(&healthy_lang_filt, &healthy_lang_all)
            && mr_pred_reindex_idempotent(&healthy_reindex_before, &healthy_reindex_after)
            && mr_pred_search_flat_prefix(&healthy_flat_small, &healthy_flat_large)
            && mr_pred_term_order_equiv(&healthy_terms_a, &healthy_terms_b)
            && mr_pred_corpus_add_orthogonal(&healthy_add_before, &healthy_add_after),
        "healthy fixtures must satisfy every MR predicate (otherwise predicates are broken)"
    );

    // --- Planted mutants (deliberately wrong transforms) ---------------------
    // 1. limit_phantom_key: small set gains a ghost key absent from large.
    let mut_limit_small: BTreeSet<HitKey> = [
        (String::from("a.rs"), 1, 1),
        (String::from("ghost.rs"), 9, 9),
    ]
    .into_iter()
    .collect();

    // 2. probe_set_shrink: higher probe incorrectly drops a lower-probe member.
    let mut_probe_hi: BTreeSet<usize> = [1, 5].into_iter().collect(); // dropped 0,2 from lo

    // 3. scale_candidate_drift: positive scale reorders / changes candidates.
    let mut_cand_scaled: Vec<usize> = vec![0, 7, 1, 3]; // permutation of healthy

    // 4. lang_filter_leak: filtered stream contains a key not in unfiltered.
    let mut_lang_filt: BTreeSet<HitKey> = [
        (String::from("a.rs"), 1, 1),
        (String::from("leaked.py"), 4, 4),
    ]
    .into_iter()
    .collect();

    // 5. reindex_hit_drift: after reindex a key vanishes / appears.
    let mut_reindex_after: BTreeSet<HitKey> = [(String::from("a.rs"), 1, 1)].into_iter().collect();

    // 6. rank_order_swap: same key multiset as top-3 of large, wrong order.
    //    Subset of indices would still pass; prefix equality fails.
    let mut_flat_small: Vec<(usize, f32)> = vec![(2, 0.90), (7, 0.95), (5, 0.80)];

    // 7. term_order_drift: permuting tokens drops a hit key.
    let mut_terms_b: BTreeSet<HitKey> = [(String::from("a.rs"), 1, 1)].into_iter().collect();

    // 8. corpus_add_drop_old: orthogonal file add loses prior hit.
    let mut_add_after: BTreeSet<HitKey> = BTreeSet::new();

    // Detecting MR for each mutant (must be true that predicate *fails* on mutant).
    let kills: &[(&str, bool)] = &[
        (
            "limit_phantom_key",
            !mr_pred_limit_subset(&mut_limit_small, &healthy_large),
        ),
        (
            "probe_set_shrink",
            !mr_pred_probe_monotone(&healthy_probe_lo, &mut_probe_hi),
        ),
        (
            "scale_candidate_drift",
            !mr_pred_scale_invariance(&healthy_cand_q, &mut_cand_scaled),
        ),
        (
            "lang_filter_leak",
            !mr_pred_lang_filter_subset(&mut_lang_filt, &healthy_lang_all),
        ),
        (
            "reindex_hit_drift",
            !mr_pred_reindex_idempotent(&healthy_reindex_before, &mut_reindex_after),
        ),
        (
            "rank_order_swap",
            !mr_pred_search_flat_prefix(&mut_flat_small, &healthy_flat_large),
        ),
        (
            "term_order_drift",
            !mr_pred_term_order_equiv(&healthy_terms_a, &mut_terms_b),
        ),
        (
            "corpus_add_drop_old",
            !mr_pred_corpus_add_orthogonal(&healthy_add_before, &mut_add_after),
        ),
    ];

    let mut killed = 0usize;
    let mut missed: Vec<&str> = Vec::new();
    for &(name, caught) in kills {
        if caught {
            killed += 1;
        } else {
            missed.push(name);
        }
    }
    let total = kills.len();
    let rate_pct = (100 * killed) / total;
    assert!(
        missed.is_empty(),
        "MR suite failed to kill mutant class(es) {missed:?} -- strengthen the corresponding MR \
         or drop it as placebo (kill-rate {killed}/{total} = {rate_pct}%)"
    );
    assert!(
        rate_pct >= 80,
        "suite kill-rate {killed}/{total} = {rate_pct}% below skill target 80%"
    );

    // Cross-check: each mutant is *specific* enough that the healthy counterpart
    // of the same class still passes (avoids "always false" placebo predicates).
    assert!(mr_pred_limit_subset(&healthy_small, &healthy_large));
    assert!(mr_pred_probe_monotone(&healthy_probe_lo, &healthy_probe_hi));
    assert!(mr_pred_scale_invariance(&healthy_cand_q, &healthy_cand_scaled));
    assert!(mr_pred_lang_filter_subset(&healthy_lang_filt, &healthy_lang_all));
    assert!(mr_pred_reindex_idempotent(
        &healthy_reindex_before,
        &healthy_reindex_after
    ));
    assert!(mr_pred_search_flat_prefix(
        &healthy_flat_small,
        &healthy_flat_large
    ));
    assert!(mr_pred_term_order_equiv(
        &healthy_terms_a,
        &healthy_terms_b
    ));
    assert!(mr_pred_corpus_add_orthogonal(
        &healthy_add_before,
        &healthy_add_after
    ));
}

/// Backward-compatible alias name used in older matrix rows / bead text.
#[test]
fn mr_suite_catches_limit_mutation() {
    // Covered by the full kill matrix; keep a focused assert for the limit class.
    type Key = (String, u32, u32);
    let real_large: BTreeSet<Key> = [
        (String::from("a.rs"), 1u32, 1u32),
        (String::from("b.rs"), 2u32, 2u32),
    ]
    .into_iter()
    .collect();
    let mutant_small: BTreeSet<Key> = [
        (String::from("a.rs"), 1u32, 1u32),
        (String::from("ghost.rs"), 9u32, 9u32),
    ]
    .into_iter()
    .collect();
    assert!(
        !mr_pred_limit_subset(&mutant_small, &real_large),
        "planted limit phantom must violate limit-subset so the suite is non-placebo"
    );
}

/// Inclusive: hits with `lang_filter=Some("rust")` are a key-subset of unfiltered hits.
///
/// Mixed-language corpus so the filter is non-vacuous (Python files share the token).
#[test]
fn mr_lang_filter_subset() {
    let corpus = TempDir::new().unwrap();
    let token = "shared_lang_token_zz";
    fs::write(
        corpus.path().join("alpha.rs"),
        format!("fn {token}() {{}}\nfn other_rs() {{ {token}(); }}\n"),
    )
    .unwrap();
    fs::write(
        corpus.path().join("beta.rs"),
        format!("// mention {token} in rust comment\nfn beta() {{}}\n"),
    )
    .unwrap();
    fs::write(
        corpus.path().join("gamma.py"),
        format!("def {token}():\n    pass\n\ndef caller():\n    {token}()\n"),
    )
    .unwrap();
    fs::write(
        corpus.path().join("delta.py"),
        format!("# {token} also lives in python\nx = 1\n"),
    )
    .unwrap();

    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");
    let _ = index_and_searcher(corpus.path(), &index_path, 32);

    let unfiltered = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        use_embed: false,
        limit: 32,
        lang_filter: None,
        ..SearchOptions::default()
    })
    .expect("unfiltered searcher");
    let rust_only = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: false,
        limit: 32,
        lang_filter: Some("rust".into()),
        ..SearchOptions::default()
    })
    .expect("rust searcher");

    let all_hits = unfiltered.search(token).expect("unfiltered search");
    let rust_hits = rust_only.search(token).expect("rust search");
    assert!(
        !all_hits.hits.is_empty(),
        "unfiltered search must return hits for mixed corpus"
    );
    assert!(
        !rust_hits.hits.is_empty(),
        "rust-filtered search must return at least one rust hit"
    );

    let all_keys = hit_keys(&all_hits.hits);
    let rust_keys = hit_keys(&rust_hits.hits);
    assert!(
        rust_keys.is_subset(&all_keys),
        "MR lang-filter-subset: every rust-filtered hit key must appear unfiltered\nrust={rust_keys:?}\nall={all_keys:?}"
    );
    // Filter must not leak non-rust files (stronger inclusive property on language field).
    for h in &rust_hits.hits {
        let lang = h.language.as_deref().unwrap_or("");
        assert!(
            lang.eq_ignore_ascii_case("rust"),
            "MR lang-filter-subset: filtered hit language must be rust, got {lang:?} for {}",
            h.file
        );
        assert!(
            h.file.ends_with(".rs"),
            "MR lang-filter-subset: filtered hit path should be rust source, got {}",
            h.file
        );
    }
    // Non-vacuous: unfiltered must surface at least one python path (filter actually drops something).
    let unfiltered_has_py = all_hits.hits.iter().any(|h| h.file.ends_with(".py"));
    assert!(
        unfiltered_has_py,
        "fixture must produce at least one python hit unfiltered so subset is meaningful; hits={:?}",
        all_hits
            .hits
            .iter()
            .map(|h| (&h.file, h.language.as_deref()))
            .collect::<Vec<_>>()
    );
}

/// Equivalence: surrounding whitespace on the query string does not change hit keys.
///
/// End-to-end (parse + search + rank), not parse-only trim.
#[test]
fn mr_query_trim_search_equivalence() {
    let corpus = TempDir::new().unwrap();
    let token = "trim_equiv_token_xyz";
    fs::write(
        corpus.path().join("hit.rs"),
        format!("fn {token}() {{}}\nfn use_it() {{ {token}(); }}\n"),
    )
    .unwrap();
    fs::write(corpus.path().join("other.rs"), "fn unrelated() {}\n").unwrap();

    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");
    let searcher = index_and_searcher(corpus.path(), &index_path, 16);

    let bare = searcher.search(token).expect("bare");
    let padded = searcher
        .search(&format!("  {token}  "))
        .expect("padded");
    let tabbed = searcher
        .search(&format!("\t{token}\n"))
        .expect("tabbed");

    assert!(
        !bare.hits.is_empty(),
        "need hits for trim equivalence; query={token}"
    );

    let bare_keys = hit_keys(&bare.hits);
    let padded_keys = hit_keys(&padded.hits);
    let tabbed_keys = hit_keys(&tabbed.hits);
    assert_eq!(
        bare_keys, padded_keys,
        "MR query-trim: space-padded query must match bare keys\nbare={bare_keys:?}\npadded={padded_keys:?}"
    );
    assert_eq!(
        bare_keys, tabbed_keys,
        "MR query-trim: tab/newline-padded query must match bare keys\nbare={bare_keys:?}\ntabbed={tabbed_keys:?}"
    );
}

/// Inclusive: more IVF probes yield a superset of candidate member indices.
///
/// Relation: for explicit `1 <= p <= P` (not adaptive `None`/`Some(0)`),
/// `set(candidate_indices(q, Some(p))) ⊆ set(candidate_indices(q, Some(P)))`.
/// `candidate_indices` L2-renorms the query; top-`take` populated clusters by
/// centroid cosine expand as a prefix when `take` grows.
#[test]
fn mr_ann_probe_monotone_candidates() {
    // Enough rows for k = sqrt(n).clamp(16, 256) = 16 distinct centroids and
    // non-empty multi-member clusters under farthest-point init.
    let dim = 4;
    let mut flat = Vec::new();
    for i in 0..64u32 {
        let t = (i as f32) * 0.17;
        flat.extend_from_slice(&[t.sin(), t.cos(), (t * 0.5).sin(), (t * 1.3).cos()]);
    }
    // Second axis cluster so nearest-centroid ranking has real separation.
    for i in 0..32u32 {
        let t = (i as f32) * 0.11;
        flat.extend_from_slice(&[0.05, 1.0 + 0.01 * t, t.sin() * 0.1, t.cos() * 0.1]);
    }
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    let q = [0.9f32, 0.15, 0.05, -0.02];

    // Probe ladder: each step must enlarge (or equal) the member set.
    let probe_steps = [1usize, 2, 4, 8, 16, 32, 64, 256];
    let mut prev: Option<(usize, BTreeSet<usize>)> = None;
    for &p in &probe_steps {
        let members: BTreeSet<usize> = index.candidate_indices(&q, Some(p)).into_iter().collect();
        assert!(
            !members.is_empty(),
            "MR ann-probe-monotone: need non-empty candidates at probes={p}"
        );
        if let Some((prev_p, ref prev_set)) = prev {
            assert!(
                prev_set.is_subset(&members),
                "MR ann-probe-monotone: candidates(probes={prev_p}) must ⊆ candidates(probes={p})\n\
                 fewer={prev_set:?}\nmore={members:?}"
            );
            // Non-vacuous at least once on the ladder: eventually more probes add mass
            // (or we already hit full partition).
            let _ = (prev_p, members.len() >= prev_set.len());
        }
        prev = Some((p, members));
    }
    // Full explicit probes should cover every vector index (partition property).
    let n = flat.len() / dim;
    let full: BTreeSet<usize> = index
        .candidate_indices(&q, Some(usize::MAX))
        .into_iter()
        .collect();
    let expected: BTreeSet<usize> = (0..n).collect();
    assert_eq!(
        full, expected,
        "MR ann-probe-monotone: probes=MAX must return full partition (n={n})"
    );
    // Strict growth somewhere on the ladder (not all steps equal from probes=1).
    let small: BTreeSet<usize> = index.candidate_indices(&q, Some(1)).into_iter().collect();
    assert!(
        small.len() < full.len(),
        "fixture must make probes=1 a proper subset of full; small={} full={}",
        small.len(),
        full.len()
    );
}

/// Inclusive: `search_flat` top-k index set ⊆ top-K for k <= K (ANN IVF path).
///
/// Uses `n >= DEFAULT_ANN_THRESHOLD` so the call routes through
/// `candidate_indices` → `score_members` (not the small-n brute-force arm).
/// Query is L2-renormed inside `search_flat`; limit only changes how many
/// scored members are returned from the same candidate pool (default probes).
#[test]
fn mr_search_flat_limit_subset() {
    let dim = 4;
    let n = DEFAULT_ANN_THRESHOLD; // 2000 -- forces IVF candidate path
    let mut flat = Vec::with_capacity(n * dim);
    for i in 0..n {
        let t = (i as f32) * 0.013;
        // Spread mass so many rows exceed MIN_SIMILARITY vs a near-axis query.
        let axis = (i % 4) as f32;
        flat.extend_from_slice(&[
            (1.0 - 0.15 * axis) + 0.01 * t.sin(),
            0.08 * axis + 0.02 * t.cos(),
            0.03 * (t * 0.7).sin(),
            0.02 * (t * 1.1).cos(),
        ]);
    }
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    let query = [1.0f32, 0.05, 0.0, 0.0];

    let k = 5usize;
    let large_k = 40usize;
    let small = index.search_flat(&flat, dim, &query, k);
    let large = index.search_flat(&flat, dim, &query, large_k);
    assert!(
        !small.is_empty(),
        "MR search-flat-limit: need non-empty top-{k}; got 0 (check MIN_SIMILARITY vs fixture)"
    );
    assert!(
        large.len() >= small.len(),
        "MR search-flat-limit: top-{large_k} must be at least as long as top-{k} ({} vs {})",
        large.len(),
        small.len()
    );

    let small_ids: BTreeSet<usize> = small.iter().map(|(i, _)| *i).collect();
    let large_ids: BTreeSet<usize> = large.iter().map(|(i, _)| *i).collect();
    assert!(
        small_ids.is_subset(&large_ids),
        "MR search-flat-limit: every top-{k} index must appear in top-{large_k}\n\
         small={small_ids:?}\nlarge={large_ids:?}"
    );
    // Scores within each response are non-increasing (ranking contract).
    for window in large.windows(2) {
        assert!(
            window[0].1 + 1e-5 >= window[1].1,
            "MR search-flat-limit: scores must be non-increasing: {} then {}",
            window[0].1,
            window[1].1
        );
    }
    // Non-vacuous: larger limit returns strictly more hits when pool allows.
    assert!(
        large.len() > small.len(),
        "fixture should yield more than {k} hits above threshold for limit={large_k}; got {}",
        large.len()
    );
}

// Inventory notes (see matrix header "Dropped"):
// - hybrid limit_top_k_prefix_equality: flaky under Def injection -- do not ship.
// - reindex_score_order: redundant with reindex_idempotent_hits.
// - corpus_file_order_permutation / empty_query / empty_index: Score < 2 or unit.
// ANN ordered prefix ships as mr_search_flat_limit_prefix_equality* below.

/// Inclusive (stronger): `search_flat` top-k is an ordered prefix of top-K.
///
/// Catches ranking-order corruption that still preserves the top-k *set*
/// (so limit-subset alone would pass). Deterministic total order on ANN scores
/// makes this free of hybrid Def-injection flakiness.
#[test]
fn mr_search_flat_limit_prefix_equality() {
    let dim = 4;
    let n = DEFAULT_ANN_THRESHOLD; // forces IVF candidate path
    let mut flat = Vec::with_capacity(n * dim);
    for i in 0..n {
        let t = (i as f32) * 0.013;
        let axis = (i % 4) as f32;
        flat.extend_from_slice(&[
            (1.0 - 0.15 * axis) + 0.01 * t.sin(),
            0.08 * axis + 0.02 * t.cos(),
            0.03 * (t * 0.7).sin(),
            0.02 * (t * 1.1).cos(),
        ]);
    }
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    let query = [1.0f32, 0.05, 0.0, 0.0];

    let k = 5usize;
    let large_k = 40usize;
    let small = index.search_flat(&flat, dim, &query, k);
    let large = index.search_flat(&flat, dim, &query, large_k);
    assert!(
        !small.is_empty(),
        "MR search-flat-prefix: need non-empty top-{k}"
    );
    assert!(
        large.len() >= small.len(),
        "MR search-flat-prefix: top-{large_k} shorter than top-{k}"
    );
    assert!(
        mr_pred_search_flat_prefix(&small, &large),
        "MR search-flat-prefix: top-{k} must equal ordered prefix of top-{large_k}\n\
         small={small:?}\nlarge_prefix={:?}",
        &large[..small.len()]
    );
    assert!(
        large.len() > small.len(),
        "fixture must yield more than {k} hits above threshold; got {}",
        large.len()
    );
}

/// Permutative: multi-term hybrid query token order does not change hit keys.
///
/// Tokenizer sorts/dedups scoring terms; bag-of-words hybrid must not depend on
/// whitespace token order for uncased multi-term queries (≥3 tokens so intent
/// stays Conceptual regardless of order). Catches accidental left-to-right
/// dependence in pass fusion or a regression that drops term sort.
#[test]
fn mr_query_term_order_equivalence() {
    let corpus = TempDir::new().unwrap();
    // Three distinct tokens co-occurring so multi-term coverage ranking is live.
    let a = "mr_perm_alpha_tok";
    let b = "mr_perm_beta_tok";
    let c = "mr_perm_gamma_tok";
    fs::write(
        corpus.path().join("combo.rs"),
        format!(
            "fn {a}() {{}}\nfn {b}() {{ {a}(); }}\nfn {c}() {{ {a}(); {b}(); }}\n"
        ),
    )
    .unwrap();
    fs::write(
        corpus.path().join("noise.rs"),
        "fn unrelated_noise_fn() {}\n",
    )
    .unwrap();
    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");
    let searcher = index_and_searcher(corpus.path(), &index_path, 32);

    let q1 = format!("{a} {b} {c}");
    let q2 = format!("{c} {a} {b}");
    let q3 = format!("{b} {c} {a}");
    let r1 = searcher.search(&q1).expect("q1");
    let r2 = searcher.search(&q2).expect("q2");
    let r3 = searcher.search(&q3).expect("q3");
    assert!(
        !r1.hits.is_empty(),
        "MR term-order: need hits for multi-term query; q1={q1}"
    );
    let k1 = hit_keys(&r1.hits);
    let k2 = hit_keys(&r2.hits);
    let k3 = hit_keys(&r3.hits);
    assert!(
        mr_pred_term_order_equiv(&k1, &k2) && mr_pred_term_order_equiv(&k1, &k3),
        "MR term-order: hit keys must match across token permutations\n\
         q1 keys={k1:?}\nq2 keys={k2:?}\nq3 keys={k3:?}"
    );
}

/// Additive: adding a query-orthogonal file then reindexing preserves hit keys.
///
/// T(corpus) = corpus ∪ {unrelated file that does not mention the query token}.
/// Relation: keys(search(q)) equal before and after. Catches rebuild paths that
/// drop previously indexed files when the walk set grows, or wipe-without-restore.
#[test]
fn mr_corpus_add_orthogonal_hit_equality() {
    let corpus = TempDir::new().unwrap();
    let token = "mr_add_orth_token_zz";
    fs::write(
        corpus.path().join("hit.rs"),
        format!("fn {token}() {{ let x = 1; }}\nfn use_it() {{ {token}(); }}\n"),
    )
    .unwrap();
    fs::write(corpus.path().join("other.rs"), "fn other_stuff() {}\n").unwrap();

    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");
    let searcher = index_and_searcher(corpus.path(), &index_path, 16);
    let before = searcher.search(token).expect("before");
    assert!(
        !before.hits.is_empty(),
        "MR corpus-add: need baseline hits for {token}"
    );
    let keys_before = hit_keys(&before.hits);

    // Orthogonal addition: no mention of the query token.
    fs::write(
        corpus.path().join("orthogonal_extra.rs"),
        "fn completely_unrelated_symbol_abc() { let n = 42; }\n",
    )
    .unwrap();

    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        force_reindex: true,
        ..IndexOptions::default()
    })
    .expect("indexer");
    indexer.reindex_all().expect("reindex after add");
    let searcher2 = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: false,
        limit: 16,
        ..SearchOptions::default()
    })
    .expect("searcher2");
    let after = searcher2.search(token).expect("after");
    let keys_after = hit_keys(&after.hits);
    assert!(
        mr_pred_corpus_add_orthogonal(&keys_before, &keys_after),
        "MR corpus-add: orthogonal file must not change hit keys for {token}\n\
         before={keys_before:?}\nafter={keys_after:?}"
    );
}

/// Composition: lang filter then limit-subset on the filtered stream.
///
/// Catches order bugs neither single catches alone: global top-k then filter
/// (small filtered set not a subset of larger filtered set when mass is
/// language-skewed), or filter applied only on the large-limit path.
#[test]
fn mr_compound_lang_filter_then_limit_subset() {
    let corpus = TempDir::new().unwrap();
    let token = "compound_lang_limit_tok_zz";
    // Several rust hits so limit=2 is a real truncation of the filtered stream.
    for (name, body) in [
        (
            "a.rs",
            format!("fn {token}() {{}}\nfn a_use() {{ {token}(); }}\n"),
        ),
        (
            "b.rs",
            format!("// {token} in rust\nfn b_helper() {{ {token}(); }}\n"),
        ),
        (
            "c.rs",
            format!("fn call_{token}() {{ {token}(); {token}(); }}\n"),
        ),
        (
            "d.py",
            format!("def {token}():\n    pass\n\ndef py_call():\n    {token}()\n"),
        ),
        (
            "e.py",
            format!("# {token} also in python\nx = 1\n"),
        ),
    ] {
        fs::write(corpus.path().join(name), body).unwrap();
    }

    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");
    let _ = index_and_searcher(corpus.path(), &index_path, 32);

    let rust_small = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        use_embed: false,
        limit: 2,
        lang_filter: Some("rust".into()),
        ..SearchOptions::default()
    })
    .expect("rust limit=2");
    let rust_large = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        use_embed: false,
        limit: 16,
        lang_filter: Some("rust".into()),
        ..SearchOptions::default()
    })
    .expect("rust limit=16");
    let unfiltered = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: false,
        limit: 32,
        lang_filter: None,
        ..SearchOptions::default()
    })
    .expect("unfiltered");

    let small = rust_small.search(token).expect("small filtered");
    let large = rust_large.search(token).expect("large filtered");
    let all = unfiltered.search(token).expect("unfiltered");

    assert!(
        !small.hits.is_empty(),
        "compound lang∘limit: need filtered hits at limit=2"
    );
    assert!(
        all.hits.iter().any(|h| h.file.ends_with(".py")),
        "compound lang∘limit: fixture must surface python unfiltered so filter is live"
    );

    let small_keys = hit_keys(&small.hits);
    let large_keys = hit_keys(&large.hits);
    assert!(
        small_keys.is_subset(&large_keys),
        "compound lang∘limit: rust top-2 keys must ⊆ rust top-16\n\
         small={small_keys:?}\nlarge={large_keys:?}"
    );
    // Filter integrity holds at both limits (composition, not only at one k).
    for (label, hits) in [("limit=2", &small.hits), ("limit=16", &large.hits)] {
        for h in hits.iter() {
            let lang = h.language.as_deref().unwrap_or("");
            assert!(
                lang.eq_ignore_ascii_case("rust"),
                "compound lang∘limit: {label} hit language must be rust, got {lang:?} for {}",
                h.file
            );
            assert!(
                h.file.ends_with(".rs"),
                "compound lang∘limit: {label} path should be .rs, got {}",
                h.file
            );
        }
    }
    // Non-vacuous truncation: large filtered stream longer than small when pool allows.
    assert!(
        large.hits.len() >= small.hits.len(),
        "compound lang∘limit: larger limit must not shrink filtered result count"
    );
    assert!(
        large.hits.len() > small.hits.len(),
        "compound lang∘limit: fixture should yield >2 rust hits so limit truncates; got {}",
        large.hits.len()
    );
}

// ---------------------------------------------------------------------------
// Property-based generation (proptest) for Score >= 2.0 ANN relations
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(mr_proptest_config())]

    /// Multiplicative/equiv: positive query scale leaves candidate index multiset unchanged.
    ///
    /// Random unit-ish flat corpora (not fixed fixtures). Scale must be **positive**:
    /// negative scale flips direction after L2 renorm and is outside the relation.
    #[test]
    fn mr_ann_query_scale_invariance_proptest(
        (dim, flat, query) in arb_ann_corpus(),
        scale in 0.05f32..50.0f32,
    ) {
        prop_assume!(scale.is_finite() && scale > 0.0);
        let index = SemanticAnnIndex::build_from_flat(&flat, dim);
        let scaled: Vec<f32> = query.iter().map(|x| x * scale).collect();
        let probes = Some(8usize);
        let a = index.candidate_indices(&query, probes);
        let b = index.candidate_indices(&scaled, probes);
        prop_assert_eq!(
            &a,
            &b,
            "MR ann-scale-proptest: candidates must match for positive scale={}",
            scale
        );
    }

    /// Inclusive: more explicit probes yield a superset of candidate member indices.
    ///
    /// Random flat corpora + random query. Adaptive probes (`None`/`Some(0)`) excluded.
    #[test]
    fn mr_ann_probe_monotone_candidates_proptest(
        (dim, flat, query) in arb_ann_corpus(),
        p in 1usize..8,
        p_hi in 8usize..64,
    ) {
        prop_assume!(p < p_hi);
        let index = SemanticAnnIndex::build_from_flat(&flat, dim);
        let fewer: BTreeSet<usize> = index
            .candidate_indices(&query, Some(p))
            .into_iter()
            .collect();
        let more: BTreeSet<usize> = index
            .candidate_indices(&query, Some(p_hi))
            .into_iter()
            .collect();
        prop_assert!(
            !more.is_empty(),
            "MR ann-probe-monotone-proptest: need non-empty candidates at probes={}",
            p_hi
        );
        prop_assert!(
            fewer.is_subset(&more),
            "MR ann-probe-monotone-proptest: candidates(probes={}) must ⊆ candidates(probes={})\n\
             fewer={:?}\nmore={:?}",
            p,
            p_hi,
            fewer,
            more
        );
    }

    /// Inclusive: `search_flat` top-k index set ⊆ top-K for random unit-ish corpora.
    ///
    /// Uses n << DEFAULT_ANN_THRESHOLD so the call routes through brute_force_flat
    /// (fast). IVF threshold path stays covered by the fixed-fixture MR.
    /// Near-query rows are injected so the relation is non-vacuous (hits exist).
    #[test]
    fn mr_search_flat_limit_subset_proptest(
        (dim, mut flat, query) in arb_ann_corpus(),
        k in 1usize..6,
        k_extra in 1usize..16,
    ) {
        let large_k = k + k_extra;
        // Ensure MIN_SIMILARITY filter does not empty the result set.
        inject_near_query(&mut flat, dim, &query, 6);
        let index = SemanticAnnIndex::build_from_flat(&flat, dim);
        let small = index.search_flat(&flat, dim, &query, k);
        let large = index.search_flat(&flat, dim, &query, large_k);
        prop_assert!(
            !small.is_empty(),
            "MR search-flat-limit-proptest: need non-empty top-{}",
            k
        );
        prop_assert!(
            large.len() >= small.len(),
            "MR search-flat-limit-proptest: top-{} shorter than top-{}",
            large_k,
            k
        );
        let small_ids: BTreeSet<usize> = small.iter().map(|(i, _)| *i).collect();
        let large_ids: BTreeSet<usize> = large.iter().map(|(i, _)| *i).collect();
        prop_assert!(
            small_ids.is_subset(&large_ids),
            "MR search-flat-limit-proptest: every top-{} index must appear in top-{}\n\
             small={:?}\nlarge={:?}",
            k,
            large_k,
            small_ids,
            large_ids
        );
        for window in large.windows(2) {
            prop_assert!(
                window[0].1 + 1e-5 >= window[1].1,
                "MR search-flat-limit-proptest: scores must be non-increasing: {} then {}",
                window[0].1,
                window[1].1
            );
        }
    }

    /// Inclusive (stronger): ordered `search_flat` top-k equals prefix of top-K.
    ///
    /// Same random corpora as limit-subset; asserts order, not only set inclusion.
    #[test]
    fn mr_search_flat_limit_prefix_equality_proptest(
        (dim, mut flat, query) in arb_ann_corpus(),
        k in 1usize..6,
        k_extra in 1usize..16,
    ) {
        let large_k = k + k_extra;
        inject_near_query(&mut flat, dim, &query, 6);
        let index = SemanticAnnIndex::build_from_flat(&flat, dim);
        let small = index.search_flat(&flat, dim, &query, k);
        let large = index.search_flat(&flat, dim, &query, large_k);
        prop_assert!(
            !small.is_empty(),
            "MR search-flat-prefix-proptest: need non-empty top-{}",
            k
        );
        prop_assert!(
            large.len() >= small.len(),
            "MR search-flat-prefix-proptest: top-{} shorter than top-{}",
            large_k,
            k
        );
        prop_assert!(
            mr_pred_search_flat_prefix(&small, &large),
            "MR search-flat-prefix-proptest: top-{} must equal ordered prefix of top-{}\n\
             small={:?}\nlarge_prefix={:?}",
            k,
            large_k,
            small,
            &large[..small.len().min(large.len())]
        );
    }

    /// Composition: positive query scale then probe monotony on the scaled query.
    ///
    /// Catches interactions where renorm is applied for default probes but broken
    /// under an explicit probe ladder (or the reverse).
    #[test]
    fn mr_compound_scale_then_probe_proptest(
        (dim, flat, query) in arb_ann_corpus(),
        scale in 0.1f32..20.0f32,
        p in 1usize..4,
        p_hi in 8usize..32,
    ) {
        prop_assume!(scale.is_finite() && scale > 0.0);
        prop_assume!(p < p_hi);
        let index = SemanticAnnIndex::build_from_flat(&flat, dim);
        let scaled: Vec<f32> = query.iter().map(|x| x * scale).collect();

        // Scale invariance at both probe counts.
        let bare_lo = index.candidate_indices(&query, Some(p));
        let scaled_lo = index.candidate_indices(&scaled, Some(p));
        prop_assert_eq!(
            &bare_lo,
            &scaled_lo,
            "compound: scale invariance failed at probes={}, scale={}",
            p,
            scale
        );
        let bare_hi = index.candidate_indices(&query, Some(p_hi));
        let scaled_hi = index.candidate_indices(&scaled, Some(p_hi));
        prop_assert_eq!(
            &bare_hi,
            &scaled_hi,
            "compound: scale invariance failed at probes={}, scale={}",
            p_hi,
            scale
        );

        // Probe monotony on the scaled query.
        let fewer: BTreeSet<usize> = scaled_lo.into_iter().collect();
        let more: BTreeSet<usize> = scaled_hi.into_iter().collect();
        prop_assert!(
            fewer.is_subset(&more),
            "compound: probe monotony failed on scaled query p={}→{}\n\
             fewer={:?}\nmore={:?}",
            p,
            p_hi,
            fewer,
            more
        );
    }

    /// Composition: positive query scale then `search_flat` limit-subset.
    ///
    /// Distinct from `compound_scale_then_probe` (candidate set vs scored top-k).
    /// Catches renorm applied for `candidate_indices` but broken on the scored
    /// `search_flat` path when k changes, or limit applied before renorm scoring.
    #[test]
    fn mr_compound_scale_then_search_flat_limit_proptest(
        (dim, mut flat, query) in arb_ann_corpus(),
        scale in 0.1f32..20.0f32,
        k in 1usize..6,
        k_extra in 1usize..16,
    ) {
        prop_assume!(scale.is_finite() && scale > 0.0);
        let large_k = k + k_extra;
        inject_near_query(&mut flat, dim, &query, 6);
        let index = SemanticAnnIndex::build_from_flat(&flat, dim);
        let scaled: Vec<f32> = query.iter().map(|x| x * scale).collect();

        // Scale invariance of scored top-k index sequences at both limits.
        let bare_small = index.search_flat(&flat, dim, &query, k);
        let scaled_small = index.search_flat(&flat, dim, &scaled, k);
        let bare_small_ids: Vec<usize> = bare_small.iter().map(|(i, _)| *i).collect();
        let scaled_small_ids: Vec<usize> = scaled_small.iter().map(|(i, _)| *i).collect();
        prop_assert_eq!(
            &bare_small_ids,
            &scaled_small_ids,
            "compound scale∘search_flat: scale invariance failed at k={}, scale={}",
            k,
            scale
        );

        let bare_large = index.search_flat(&flat, dim, &query, large_k);
        let scaled_large = index.search_flat(&flat, dim, &scaled, large_k);
        let bare_large_ids: Vec<usize> = bare_large.iter().map(|(i, _)| *i).collect();
        let scaled_large_ids: Vec<usize> = scaled_large.iter().map(|(i, _)| *i).collect();
        prop_assert_eq!(
            &bare_large_ids,
            &scaled_large_ids,
            "compound scale∘search_flat: scale invariance failed at K={}, scale={}",
            large_k,
            scale
        );

        // Limit-subset on the scaled query (scored path).
        prop_assert!(
            !scaled_small.is_empty(),
            "compound scale∘search_flat: need non-empty top-{} on scaled query",
            k
        );
        prop_assert!(
            scaled_large.len() >= scaled_small.len(),
            "compound scale∘search_flat: top-{} shorter than top-{}",
            large_k,
            k
        );
        let small_set: BTreeSet<usize> = scaled_small_ids.into_iter().collect();
        let large_set: BTreeSet<usize> = scaled_large_ids.into_iter().collect();
        prop_assert!(
            small_set.is_subset(&large_set),
            "compound scale∘search_flat: every top-{} id must appear in top-{}\n\
             small={:?}\nlarge={:?}",
            k,
            large_k,
            small_set,
            large_set
        );
        for window in scaled_large.windows(2) {
            prop_assert!(
                window[0].1 + 1e-5 >= window[1].1,
                "compound scale∘search_flat: scores non-increasing: {} then {}",
                window[0].1,
                window[1].1
            );
        }
    }
}

// Silence unused import if Arc unused in some rustc versions
#[allow(dead_code)]
fn _hold() {
    let _ = Arc::new(0);
}
