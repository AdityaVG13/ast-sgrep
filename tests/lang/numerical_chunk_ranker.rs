//! Chunk-ranker contract suite: rank_chunk_indices_by_vector in one test.
//!
//! Absorbs the four per-function chunk-ranker clauses from the N1/N2/N4 pass
//! files. Tolerance table: BIT-EXACT (`assert_eq!`) for hand 1.0 heads,
//! filter outcomes, shaping guards, fail-closed rows, extreme limits, final
//! orders; 1e-6 inline (f64) for the 7/(5*sqrt(2)) and straddle formulas via
//! independent f64 arithmetic.

use ast_sgrep_embed::{rank_chunk_indices_by_vector, MIN_SIMILARITY};
use ast_sgrep_testkit::chunk_row;

/// INTENT: chunk-ranker total contract — hand cosine values with dim/
/// orthogonal/zero/negative filters and limit shaping, empty/limit shaping
/// guards, nonfinite query/row fail-closed with extreme-limit totality, and
/// the MIN_SIMILARITY threshold straddle.
/// KILLS: chunk-score, dim/threshold-filter-removal, limit/empty-guard-removal,
/// nonfinite-fail-open, extreme-limit, MIN_SIMILARITY-cut mutants.
/// ABSORBS: rank_chunk_indices_pins_hand_cosine_and_filters (N1), rank_chunk_indices_empty_and_limit_shape (N1),
/// chunk_ranker_nonfinite_query_and_rows_fail_closed (N2), chunk_ranker_threshold_straddle_keeps_above_drops_below (N4).
/// LINE-DROPS: none.
#[test]
fn chunk_ranker_contract() {
    // ── Clause chunk_hand (N1): hand cosines + filters ──
    // query=[3,4] (L2=5). Row0 [6,8]: dot=18+32=50, L2=10, 50/(5*10)=1.0
    // BIT-EXACT. Row1 [1,1]: dot=7, L2=sqrt(2), 7/(5*sqrt(2)) ~= 0.98994949
    // (1e-6, independent f64 formula). Row2 [4,-3]: dot=12-12=0 -> dropped
    // by the MIN_SIMILARITY threshold. Row3 [0,0]: den=0 -> 0.0 dropped.
    // Row4 [-6,-8]: -1.0 dropped. Row5 wrong dim -> filtered pre-score.
    assert_eq!(MIN_SIMILARITY, 0.08);
    let chunks = vec![
        chunk_row(vec![6.0, 8.0]),
        chunk_row(vec![1.0, 1.0]),
        chunk_row(vec![4.0, -3.0]),
        chunk_row(vec![0.0, 0.0]),
        chunk_row(vec![-6.0, -8.0]),
        chunk_row(vec![1.0, 2.0, 3.0]),
    ];
    let ranked = rank_chunk_indices_by_vector(&[3.0, 4.0], &chunks, 10);
    assert_eq!(ranked.len(), 2, "got {ranked:?}");
    assert_eq!(ranked[0], (0, 1.0));
    assert_eq!(ranked[1].0, 1);
    let expected = 7.0f64 / (5.0 * 2.0f64.sqrt());
    assert!((f64::from(ranked[1].1) - expected).abs() < 1e-6, "got {ranked:?}");
    // Limit shaping keeps the head of the same order.
    assert_eq!(
        rank_chunk_indices_by_vector(&[3.0, 4.0], &chunks, 1),
        vec![(0, 1.0)]
    );
    // Zero query: qn=0 so every den is 0 -> all 0.0 -> threshold drops all.
    let zero_q = rank_chunk_indices_by_vector(&[0.0, 0.0], &chunks, 10);
    assert!(zero_q.is_empty(), "got {zero_q:?}");

    // ── Clause chunk_empty (N1): shaping guards ──
    // Shaping guards: empty corpus, zero limit, all-filtered, all-dropped.
    let good = vec![chunk_row(vec![6.0, 8.0])];
    assert!(rank_chunk_indices_by_vector(&[3.0, 4.0], &[], 10).is_empty());
    assert!(rank_chunk_indices_by_vector(&[3.0, 4.0], &good, 0).is_empty());
    let wrong_dim = vec![chunk_row(vec![1.0])];
    assert!(rank_chunk_indices_by_vector(&[3.0, 4.0], &wrong_dim, 10).is_empty());
    let orthogonal = vec![chunk_row(vec![4.0, -3.0])];
    assert!(rank_chunk_indices_by_vector(&[3.0, 4.0], &orthogonal, 10).is_empty());

    // ── Clause chunk_nonfinite (N2): fail-closed totality ──
    let good = chunk_row(vec![6.0, 8.0]);
    // NaN query: unguarded l2 is NaN, den is NaN, `den > 0.0` is false,
    // so every row scores 0.0 and the threshold drops all. Empty.
    let nan_q = rank_chunk_indices_by_vector(
        &[f32::NAN, f32::NAN],
        &[chunk_row(vec![6.0, 8.0])],
        10,
    );
    assert!(nan_q.is_empty(), "got {nan_q:?}");
    // NaN/inf rows score 0.0 (NaN den / 0.0-per-nonfinite-dot) and drop;
    // the good row still scores exactly 1.0. BIT-EXACT.
    let mixed = vec![
        good,
        chunk_row(vec![f32::NAN, f32::NAN]),
        chunk_row(vec![f32::INFINITY, f32::INFINITY]),
    ];
    assert_eq!(
        rank_chunk_indices_by_vector(&[3.0, 4.0], &mixed, 10),
        vec![(0, 1.0)]
    );
    // Empty-dim degenerate: zero-length query and row pass the dim
    // filter but den is 0.0 -> 0.0 -> threshold drops. Empty.
    let empty_dim = vec![chunk_row(Vec::new())];
    assert!(rank_chunk_indices_by_vector(&[], &empty_dim, 10).is_empty());
    // Extreme limit admits every survivor without padding or panic.
    let one = vec![chunk_row(vec![6.0, 8.0])];
    assert_eq!(
        rank_chunk_indices_by_vector(&[3.0, 4.0], &one, usize::MAX),
        vec![(0, 1.0)]
    );

    // ── Clause straddle_drill (N4): MIN_SIMILARITY straddle ──
    // query=[1,0] against MIN_SIMILARITY (design guard below checks the
    // straddle against the constant; the literal is N1's pin, not this one's).
    // idx0 [2,25]: 2/sqrt(629) = 2/25.079872 = 0.079745 < min -> dropped.
    // idx1 [2,24]: 2/sqrt(580) = 2/24.083189 = 0.083045 > next(min) -> kept.
    // idx2 [1,0] -> 1.0. Margins (2.5e-4 / 3e-3) dwarf f32 rounding.
    // Exact final order: [(2, 1.0), (1, 0.083045)].
    let below_ref = 2.0f64 / 629.0f64.sqrt();
    let above_ref = 2.0f64 / 580.0f64.sqrt();
    assert!(below_ref < f64::from(MIN_SIMILARITY));
    assert!(above_ref > f64::from(MIN_SIMILARITY));
    let chunks = vec![
        chunk_row(vec![2.0, 25.0]),
        chunk_row(vec![2.0, 24.0]),
        chunk_row(vec![1.0, 0.0]),
    ];
    let ranked = rank_chunk_indices_by_vector(&[1.0, 0.0], &chunks, 10);
    assert_eq!(ranked.len(), 2, "got {ranked:?}");
    assert_eq!(ranked[0], (2, 1.0));
    assert_eq!(ranked[1].0, 1);
    assert!(
        (f64::from(ranked[1].1) - above_ref).abs() < 1e-6,
        "got {ranked:?} expected {above_ref}"
    );
}
