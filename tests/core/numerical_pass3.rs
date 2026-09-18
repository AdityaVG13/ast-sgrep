//! N3 metamorphic-relation oracles for the ast-sgrep-core float surface.
//!
//! N1 pins exact values on sane inputs; N2 pins totality on hostile inputs;
//! L2 pins the RRF/fusion happy path. This suite asserts RELATIONS that must
//! hold across input transformations — no value pins, no overlap with those
//! suites:
//!
//! * score monotonicity in rank (better rank never scores lower) and in
//!   relevance (more evidence never scores lower, def >= caller);
//! * fusion permutation invariance (input hit order and channel assignment
//!   order do not change the fused result);
//! * uniform weight scaling preserves pairwise ordering;
//! * determinism across repeated calls and across threads (bit-identical);
//! * empty-channel removal identity (absent channel == zero contribution);
//! * tie determinism (equal scores break ties identically every run).
//!
//! Epsilons below are hand-derived from IEEE-754 double rounding at the
//! stated magnitudes, with the bound in the comment. Bit-equality is asserted
//! only where the operation is exactly reproducible (same op order replayed,
//! or IEEE `+` commutativity for two-term swaps). No test touches env vars,
//! so this binary is parallel-safe.

use ast_sgrep_core::fusion::{
    analyze_weight_sensitivity, apply_weighted_rrf, learn_fusion_weights, weighted_rrf_score,
    ChannelRanks, FusionCandidate, FusionChannel, FusionExample,
};
use ast_sgrep_core::intent::{route_hits, ChannelWeights};
use ast_sgrep_core::rank::{fuse_rrf, rrf_score, score_caller, score_def, score_lexical_rrf};
use ast_sgrep_core::search::{HitKind, SearchHit, SpanHitInput};
use ast_sgrep_core::ParsedQuery;

fn n3_hit(kind: HitKind, file: &str, line: u32, score: f64) -> SearchHit {
    SearchHit::span(SpanHitInput {
        kind,
        file: file.to_string(),
        line_start: line,
        line_end: line,
        score,
        excerpt: format!("excerpt {file}:{line}"),
        symbol: None,
        language: None,
        byte_span: None,
    })
}

/// `SearchHit` has no `PartialEq`; compare the full observable fused row.
fn n3_key(hit: &SearchHit) -> (HitKind, String, u32, u32, u64, Vec<HitKind>) {
    (
        hit.kind,
        hit.file.clone(),
        hit.line_start,
        hit.line_end,
        hit.score.to_bits(),
        hit.contributors.clone(),
    )
}

fn n3_set_rank(ranks: &mut ChannelRanks, channel: FusionChannel, value: Option<usize>) {
    match channel {
        FusionChannel::Lexical => ranks.lexical = value,
        FusionChannel::Definition => ranks.definition = value,
        FusionChannel::Caller => ranks.caller = value,
        FusionChannel::Graph => ranks.graph = value,
        FusionChannel::Anchor => ranks.anchor = value,
        FusionChannel::Semantic => ranks.semantic = value,
        FusionChannel::Pattern => ranks.pattern = value,
        FusionChannel::Import => ranks.import = value,
    }
}

fn n3_set_weight(weights: &mut ChannelWeights, channel: FusionChannel, value: f64) {
    match channel {
        FusionChannel::Lexical => weights.lexical = value,
        FusionChannel::Definition => weights.def = value,
        FusionChannel::Caller => weights.caller = value,
        FusionChannel::Graph => weights.graph = value,
        FusionChannel::Anchor => weights.anchor = value,
        FusionChannel::Semantic => weights.embed = value,
        FusionChannel::Pattern => weights.pattern = value,
        FusionChannel::Import => weights.import = value,
    }
}

fn n3_single_rank(channel: FusionChannel, rank: usize) -> ChannelRanks {
    let mut ranks = ChannelRanks::default();
    n3_set_rank(&mut ranks, channel, Some(rank));
    ranks
}

// ---------------------------------------------------------------------------
// 1. rrf_score is strictly decreasing in rank and in k (sane domain)
// ---------------------------------------------------------------------------

#[test]
fn rrf_score_decreases_in_rank_and_k() {
    // rrf = 1/(k + rank + 1): larger divisor -> smaller quotient. Strictness
    // holds because adjacent divisors differ by 1.0 (exactly representable at
    // these magnitudes) and the quotients differ by ~1/61-1/62 ~= 2.6e-4,
    // far above 1 ulp (~3e-20 here).
    for rank in 0..200usize {
        assert!(
            rrf_score(rank, 60.0) > rrf_score(rank + 1, 60.0),
            "rank {rank} must outscore rank {}",
            rank + 1
        );
    }
    // Same relation in k: holding rank fixed, a larger k never scores higher.
    let ks = [0.0, 0.5, 1.0, 10.0, 60.0, 61.0, 100.0];
    for window in ks.windows(2) {
        assert!(
            rrf_score(3, window[0]) > rrf_score(3, window[1]),
            "k={} must outscore k={}",
            window[0],
            window[1]
        );
    }
}

// ---------------------------------------------------------------------------
// 2. fuse_rrf: superset monotone; two-term input order is bit-irrelevant
// ---------------------------------------------------------------------------

#[test]
fn fuse_rrf_superset_monotone_and_pair_commutative() {
    // Every term is positive (k = 60), so extending the rank list strictly
    // raises the sum: the added term (~1/61) dwarfs rounding (~1e-16).
    let mut ranks: Vec<usize> = vec![5, 3, 9];
    let before = fuse_rrf(&ranks, 60.0);
    ranks.push(0);
    assert!(fuse_rrf(&ranks, 60.0) > before);
    ranks.extend([1, 2, 100]);
    assert!(fuse_rrf(&ranks, 60.0) > before);
    // Two-term swap is BIT-identical: sum() starts from 0.0 (0.0 + x == x
    // exactly) and IEEE addition is commutative (a + b == b + a, same
    // rounding). So input order cannot leak into a two-rank fusion.
    assert_eq!(fuse_rrf(&[0, 1], 60.0), fuse_rrf(&[1, 0], 60.0));
    // Lexical scale-up: more per-term ranks never lower the scaled sum.
    for n in 1..16usize {
        let small = score_lexical_rrf(&vec![0; n]);
        let big = score_lexical_rrf(&vec![0; n + 1]);
        assert!(big > small, "n={n} must outscore n={}", n + 1);
    }
}

// ---------------------------------------------------------------------------
// 3. weighted_rrf_score: improving any single channel rank never lowers
// ---------------------------------------------------------------------------

#[test]
fn weighted_rrf_rank_improvement_never_lowers() {
    // Unit weights (clamp identity): moving one channel from rank 5 to rank 0
    // swaps 1/66 for 1/61, a gain of ~1.3e-3 against ~1e-16 rounding.
    let weights = ChannelWeights::default();
    for channel in FusionChannel::ALL {
        let worse = weighted_rrf_score(&n3_single_rank(channel, 5), &weights);
        let better = weighted_rrf_score(&n3_single_rank(channel, 0), &weights);
        assert!(
            better > worse,
            "{channel:?}: rank 0 must outscore rank 5 ({better} vs {worse})"
        );
    }
    // Same relation against a populated background: other channels held fixed.
    let mut background = ChannelRanks::default();
    for channel in FusionChannel::ALL {
        n3_set_rank(&mut background, channel, Some(4));
    }
    for channel in FusionChannel::ALL {
        let mut improved = background.clone();
        n3_set_rank(&mut improved, channel, Some(0));
        assert!(
            weighted_rrf_score(&improved, &weights) > weighted_rrf_score(&background, &weights),
            "{channel:?}: rank improvement over a background must raise the sum"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. weighted_rrf_score: adding an absent channel never lowers
// ---------------------------------------------------------------------------

#[test]
fn weighted_rrf_adding_absent_channel_never_lowers() {
    // None contributes no term; Some(rank) contributes a positive term, so the
    // sum strictly grows (gain >= 0.25/(61+rank) >> rounding).
    let weights = ChannelWeights::default();
    let empty = ChannelRanks::default();
    let empty_score = weighted_rrf_score(&empty, &weights);
    for channel in FusionChannel::ALL {
        let added = weighted_rrf_score(&n3_single_rank(channel, 2), &weights);
        assert!(
            added > empty_score,
            "{channel:?}: adding a channel must raise the sum"
        );
    }
    // Incremental build-up over all 8 channels is a strictly rising chain.
    let mut ranks = ChannelRanks::default();
    let mut prev = weighted_rrf_score(&ranks, &weights);
    for (i, channel) in FusionChannel::ALL.iter().enumerate() {
        n3_set_rank(&mut ranks, *channel, Some(i));
        let next = weighted_rrf_score(&ranks, &weights);
        assert!(next > prev, "chain must rise at {channel:?}");
        prev = next;
    }
}

// ---------------------------------------------------------------------------
// 5. weighted_rrf_score: channel assignment order is irrelevant
// ---------------------------------------------------------------------------

#[test]
fn weighted_rrf_channel_pair_swap_invariant() {
    // Swap the (rank, weight) pairs of two channels: the two fused terms are
    // identical as a multiset and IEEE `+` is commutative, so the two-term
    // sum is BIT-identical. Weights 1.5/0.5 sit inside the [0.25, 2.0] rails
    // so the clamp is the identity on both sides.
    let mut weights_a = ChannelWeights::default();
    n3_set_weight(&mut weights_a, FusionChannel::Lexical, 1.5);
    n3_set_weight(&mut weights_a, FusionChannel::Definition, 0.5);
    let mut ranks_a = ChannelRanks::default();
    n3_set_rank(&mut ranks_a, FusionChannel::Lexical, Some(0));
    n3_set_rank(&mut ranks_a, FusionChannel::Definition, Some(3));
    let mut weights_b = ChannelWeights::default();
    n3_set_weight(&mut weights_b, FusionChannel::Lexical, 0.5);
    n3_set_weight(&mut weights_b, FusionChannel::Definition, 1.5);
    let mut ranks_b = ChannelRanks::default();
    n3_set_rank(&mut ranks_b, FusionChannel::Lexical, Some(3));
    n3_set_rank(&mut ranks_b, FusionChannel::Definition, Some(0));
    assert_eq!(
        weighted_rrf_score(&ranks_a, &weights_a),
        weighted_rrf_score(&ranks_b, &weights_b),
        "swapping (rank, weight) pairs between channels must be bit-identical"
    );

    // Full 8-channel reversal: same 8-term multiset, different sequential-sum
    // order. Each partial rounding is <= 0.5 ulp of a partial <= ~0.26, so the
    // two sums agree to ~3e-16; 1e-14 keeps 30x headroom while a dropped term
    // (min ~0.25/75 ~= 3e-3) or a clamp mutant misses by orders more.
    let rail_weights = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 0.25];
    let mut ranks_f = ChannelRanks::default();
    let mut weights_f = ChannelWeights::default();
    let mut ranks_r = ChannelRanks::default();
    let mut weights_r = ChannelWeights::default();
    for (i, channel) in FusionChannel::ALL.iter().enumerate() {
        n3_set_rank(&mut ranks_f, *channel, Some(i));
        n3_set_weight(&mut weights_f, *channel, rail_weights[i]);
        n3_set_rank(&mut ranks_r, *channel, Some(7 - i));
        n3_set_weight(&mut weights_r, *channel, rail_weights[7 - i]);
    }
    let forward = weighted_rrf_score(&ranks_f, &weights_f);
    let reversed = weighted_rrf_score(&ranks_r, &weights_r);
    assert!(
        (forward - reversed).abs() < 1e-14,
        "8-channel reversal must agree: {forward} vs {reversed}"
    );
}

// ---------------------------------------------------------------------------
// 6. apply_weighted_rrf: input hit order is bit-irrelevant
// ---------------------------------------------------------------------------

fn n3_mixed_hits() -> Vec<SearchHit> {
    vec![
        n3_hit(HitKind::Caller, "m.rs", 7, 8.0),
        n3_hit(HitKind::Def, "m.rs", 7, 5.0),
        n3_hit(HitKind::Asgrep, "a.rs", 1, 3.0),
        n3_hit(HitKind::Graph, "g.rs", 2, 6.0),
        n3_hit(HitKind::Anchor, "n.rs", 3, 1.0),
        n3_hit(HitKind::Embed, "e.rs", 4, 2.0),
        n3_hit(HitKind::Pattern, "p.rs", 5, 4.0),
        n3_hit(HitKind::Import, "i.rs", 6, 7.0),
        n3_hit(HitKind::Def, "z.rs", 9, 9.0),
    ]
}

fn n3_fused_keys(hits: &[SearchHit]) -> Vec<(HitKind, String, u32, u32, u64, Vec<HitKind>)> {
    let mut fused = hits.to_vec();
    apply_weighted_rrf(&mut fused, &ChannelWeights::default());
    fused.iter().map(n3_key).collect()
}

#[test]
fn apply_weighted_rrf_input_permutation_bit_identical() {
    // Fusion groups by channel, ranks within channel by (score, file, line),
    // and emits in sorted key order — so the input Vec order (reverse,
    // rotation, halved swap) must vanish bit-for-bit, including contributors.
    let base = n3_mixed_hits();
    let expected = n3_fused_keys(&base);
    let mut reversed = base.clone();
    reversed.reverse();
    assert_eq!(n3_fused_keys(&reversed), expected, "reversed input");
    let mut rotated = base.clone();
    rotated.rotate_left(3);
    assert_eq!(n3_fused_keys(&rotated), expected, "rotated input");
    let mut swapped = base.clone();
    let half = swapped.len() / 2;
    swapped[..half].reverse();
    swapped[half..].reverse();
    assert_eq!(n3_fused_keys(&swapped), expected, "halved input");
}

// ---------------------------------------------------------------------------
// 7. Uniform weight scaling preserves pairwise ordering
// ---------------------------------------------------------------------------

#[test]
fn uniform_weight_scaling_preserves_pairwise_order() {
    // Scale every channel 1.0 -> 1.5 (inside the rails, so no clamp skew):
    // each fused score multiplies by ~1.5, so every pairwise ordering sign
    // is preserved. Candidates are separated by >= ~1.3e-3 (a full rank step
    // at low ranks), ~1e13x above rounding, so no order can flip.
    let unit = ChannelWeights::default();
    let scaled = ChannelWeights {
        lexical: 1.5,
        def: 1.5,
        caller: 1.5,
        graph: 1.5,
        anchor: 1.5,
        embed: 1.5,
        pattern: 1.5,
        import: 1.5,
    };
    let mut all_zero = ChannelRanks::default();
    for channel in FusionChannel::ALL {
        n3_set_rank(&mut all_zero, channel, Some(0));
    }
    let mut two_zero = ChannelRanks::default();
    n3_set_rank(&mut two_zero, FusionChannel::Lexical, Some(0));
    n3_set_rank(&mut two_zero, FusionChannel::Definition, Some(0));
    let candidates = vec![
        ChannelRanks::default(),
        n3_single_rank(FusionChannel::Lexical, 5),
        n3_single_rank(FusionChannel::Lexical, 0),
        two_zero,
        all_zero,
    ];
    let base: Vec<f64> = candidates.iter().map(|c| weighted_rrf_score(c, &unit)).collect();
    let up: Vec<f64> = candidates.iter().map(|c| weighted_rrf_score(c, &scaled)).collect();
    for i in 0..candidates.len() {
        for j in 0..candidates.len() {
            assert_eq!(
                base[i].total_cmp(&base[j]),
                up[i].total_cmp(&up[j]),
                "pair ({i}, {j}) ordering must survive uniform scaling"
            );
        }
        // Proportionality: sum(1.5*t)/sum(t) ~= 1.5 to ~1e-15 relative (8
        // terms); 1e-12 keeps 1000x headroom. The all-absent candidate scores
        // exactly 0.0 on both sides (empty sum), so it is skipped here.
        if base[i] > 0.0 {
            let ratio = up[i] / base[i];
            assert!(
                ((ratio - 1.5) / 1.5).abs() < 1e-12,
                "candidate {i}: ratio {ratio} must be ~1.5"
            );
        } else {
            assert_eq!(up[i], 0.0);
        }
    }
}

// ---------------------------------------------------------------------------
// 8. Empty-channel removal identity: absent == zero contribution
// ---------------------------------------------------------------------------

#[test]
fn absent_channel_equals_zero_contribution() {
    // All-absent ranks sum over zero terms: exactly 0.0 (empty-iterator sum).
    assert_eq!(
        weighted_rrf_score(&ChannelRanks::default(), &ChannelWeights::default()),
        0.0
    );
    // Removing one present channel lowers the sum by exactly that channel's
    // term, up to sequential-sum reassociation (~1e-16 at sum magnitude ~0.2;
    // 1e-12 keeps 10000x headroom). Weights sit on the rails' interior so the
    // clamp is the identity and the oracle term needs no clamp model.
    let rail_weights = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 0.3];
    let mut ranks = ChannelRanks::default();
    let mut weights = ChannelWeights::default();
    for (i, channel) in FusionChannel::ALL.iter().enumerate() {
        n3_set_rank(&mut ranks, *channel, Some(2 * i));
        n3_set_weight(&mut weights, *channel, rail_weights[i]);
    }
    let full = weighted_rrf_score(&ranks, &weights);
    for (i, channel) in FusionChannel::ALL.iter().enumerate() {
        let mut without = ranks.clone();
        n3_set_rank(&mut without, *channel, None);
        let reduced = weighted_rrf_score(&without, &weights);
        assert!(reduced < full, "{channel:?}: removal must lower the sum");
        let term = rail_weights[i] * rrf_score(2 * i, 60.0);
        assert!(
            (full - reduced - term).abs() < 1e-12,
            "{channel:?}: removal delta must equal the channel term"
        );
    }
}

// ---------------------------------------------------------------------------
// 9. Determinism: repeated calls are bit-identical
// ---------------------------------------------------------------------------

#[test]
fn repeated_calls_bit_identical() {
    // Same inputs replayed bit-for-bit must reproduce bit-for-bit: every fn
    // below is a pure function of its arguments (no HashMap iteration leaks
    // into outputs, no time/randomness).
    let mut ranks = ChannelRanks::default();
    n3_set_rank(&mut ranks, FusionChannel::Lexical, Some(1));
    n3_set_rank(&mut ranks, FusionChannel::Definition, Some(0));
    let weights = ChannelWeights::default();
    assert_eq!(
        weighted_rrf_score(&ranks, &weights),
        weighted_rrf_score(&ranks, &weights)
    );
    assert_eq!(fuse_rrf(&[0, 1, 2], 60.0), fuse_rrf(&[0, 1, 2], 60.0));
    let terms = vec!["foo".to_string(), "bar".to_string()];
    assert_eq!(score_def(&terms, "foo bar"), score_def(&terms, "foo bar"));
    assert_eq!(score_caller(&terms, "foo"), score_caller(&terms, "foo"));

    let parsed = ParsedQuery::parse("foo bar");
    let route_once = |scores: &[f64]| {
        let mut hits: Vec<SearchHit> = scores
            .iter()
            .enumerate()
            .map(|(i, s)| n3_hit(HitKind::Embed, "r.rs", i as u32, *s))
            .collect();
        route_hits(&parsed, &mut hits);
        hits.iter().map(|h| h.score.to_bits()).collect::<Vec<_>>()
    };
    assert_eq!(route_once(&[0.5, 2.0, 9.0]), route_once(&[0.5, 2.0, 9.0]));
    assert_eq!(n3_fused_keys(&n3_mixed_hits()), n3_fused_keys(&n3_mixed_hits()));

    let examples = n3_pair_examples();
    assert_eq!(
        learn_fusion_weights(&examples, ChannelWeights::default()),
        learn_fusion_weights(&examples, ChannelWeights::default())
    );
    assert_eq!(
        analyze_weight_sensitivity(&examples, &ChannelWeights::default(), 0.1),
        analyze_weight_sensitivity(&examples, &ChannelWeights::default(), 0.1)
    );
}

fn n3_pair_examples() -> Vec<FusionExample> {
    vec![FusionExample {
        query: "q".to_string(),
        candidates: vec![
            FusionCandidate {
                id: "worse".to_string(),
                relevance: 0.0,
                ranks: n3_single_rank(FusionChannel::Lexical, 1),
            },
            FusionCandidate {
                id: "better".to_string(),
                relevance: 1.0,
                ranks: n3_single_rank(FusionChannel::Lexical, 0),
            },
        ],
    }]
}

// ---------------------------------------------------------------------------
// 10. Determinism: concurrent scoring is bit-identical to serial scoring
// ---------------------------------------------------------------------------

#[test]
fn multithreaded_scoring_bit_identical() {
    // 8 threads score the same fusion + learning workload; every thread must
    // agree bit-for-bit with the serial result (no shared mutable state, no
    // HashMap-order leakage — apply_weighted_rrf emits in sorted key order).
    let expected_fused = n3_fused_keys(&n3_mixed_hits());
    let expected_model = learn_fusion_weights(&n3_pair_examples(), ChannelWeights::default());
    let expected_sens =
        analyze_weight_sensitivity(&n3_pair_examples(), &ChannelWeights::default(), 0.1);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..8 {
            handles.push(scope.spawn(|| {
                let fused = n3_fused_keys(&n3_mixed_hits());
                let model = learn_fusion_weights(&n3_pair_examples(), ChannelWeights::default());
                let sens =
                    analyze_weight_sensitivity(&n3_pair_examples(), &ChannelWeights::default(), 0.1);
                (fused, model, sens)
            }));
        }
        for handle in handles {
            let (fused, model, sens) = handle.join().expect("worker must not panic");
            assert_eq!(fused, expected_fused, "fused rows must match serial");
            assert_eq!(model, expected_model, "learned model must match serial");
            assert_eq!(sens, expected_sens, "sensitivity table must match serial");
        }
    });
}

// ---------------------------------------------------------------------------
// 11. Tie determinism: equal input scores break ties identically every run
// ---------------------------------------------------------------------------

#[test]
fn tie_scores_break_identically() {
    // Every input score is the same 5.0, so within-channel ranks come purely
    // from the (file, line) tiebreaks and emission is in sorted key order:
    // reversing the input must reproduce the fused stream bit-for-bit, and a
    // second serial run must agree as well.
    let tied = vec![
        n3_hit(HitKind::Def, "a.rs", 1, 5.0),
        n3_hit(HitKind::Def, "b.rs", 1, 5.0),
        n3_hit(HitKind::Def, "c.rs", 1, 5.0),
        n3_hit(HitKind::Caller, "a.rs", 2, 5.0),
        n3_hit(HitKind::Asgrep, "b.rs", 2, 5.0),
    ];
    let first = n3_fused_keys(&tied);
    let mut reversed = tied.clone();
    reversed.reverse();
    assert_eq!(
        n3_fused_keys(&reversed),
        first,
        "ties must break by key, not by input order"
    );
    assert_eq!(n3_fused_keys(&tied), first, "ties must repeat run to run");
    // Emission order itself is sorted by (file, line): the tie order is total.
    let keys: Vec<(&str, u32)> = first.iter().map(|k| (k.1.as_str(), k.2)).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted, "fused emission must be in sorted key order");
}

// ---------------------------------------------------------------------------
// 12. learn_fusion_weights: candidate order within a pair is irrelevant
// ---------------------------------------------------------------------------

#[test]
fn learn_pair_candidate_order_invariant() {
    // A single (better, worse) pair: pairwise_loss normalizes the pair by the
    // relevance comparison, so listing the worse candidate first or second
    // feeds the identical delta into the identical coordinate search —
    // bit-identical losses, weights, and sensitivity table.
    let worse_first = n3_pair_examples();
    let mut better_first = n3_pair_examples();
    better_first[0].candidates.reverse();
    assert_eq!(
        learn_fusion_weights(&worse_first, ChannelWeights::default()),
        learn_fusion_weights(&better_first, ChannelWeights::default())
    );
}

// ---------------------------------------------------------------------------
// 13. route_hits is monotone within a fixed-ceiling channel
// ---------------------------------------------------------------------------

#[test]
fn route_hits_monotone_within_fixed_ceiling_channel() {
    // Embed divides by the fixed ceiling 4.0 and clamps to [0, 1]: x/4 is
    // strictly increasing on finite positives and clamping only collapses
    // values (never inverts), so ascending inputs stay non-decreasing. (This
    // relation holds ONLY for fixed-ceiling channels: Def/Caller ceilings
    // vary per hit via the symbol match and can legitimately reorder.)
    let parsed = ParsedQuery::parse("foo");
    let mut hits: Vec<SearchHit> = [0.3, 1.1, 2.7, 4.0, 9.0]
        .iter()
        .enumerate()
        .map(|(i, s)| n3_hit(HitKind::Embed, "m.rs", i as u32, *s))
        .collect();
    route_hits(&parsed, &mut hits);
    for window in hits.windows(2) {
        assert!(
            window[1].score >= window[0].score,
            "routed scores must be non-decreasing: {} then {}",
            window[0].score,
            window[1].score
        );
    }
    for hit in &hits {
        assert!(
            (0.0..=1.0).contains(&hit.score),
            "routed score {} must sit in [0, 1]",
            hit.score
        );
    }
    // The two over-ceiling inputs collapse to a tie rather than inverting.
    assert_eq!(hits[3].score, hits[4].score);
}

// ---------------------------------------------------------------------------
// 14. score_def / score_caller relevance ladder relations
// ---------------------------------------------------------------------------

#[test]
fn def_caller_relevance_ladder_relations() {
    // Same coverage feeds both: def = 2c + 3, caller = 2c + 1.5, so def is
    // strictly above caller exactly when coverage is positive, and the two
    // agree exactly when nothing matches (no base awarded either way).
    let matched = vec!["foo".to_string()];
    assert!(score_def(&matched, "foo") > score_caller(&matched, "foo"));
    let multi = vec!["foo".to_string(), "bar".to_string()];
    assert!(score_def(&multi, "foo bar") > score_caller(&multi, "foo bar"));
    let unmatched = vec!["zzz".to_string()];
    assert_eq!(score_def(&unmatched, "foo"), score_caller(&unmatched, "foo"));

    // Appending a matching term strictly raises coverage; appending a
    // non-matching term leaves the sum bit-identical (zero-score terms are
    // skipped before the accumulator is touched).
    let one = vec!["foo".to_string()];
    let two = vec!["foo".to_string(), "foo".to_string()];
    assert!(score_def(&two, "foo") > score_def(&one, "foo"));
    assert!(score_caller(&two, "foo") > score_caller(&one, "foo"));
    let with_noise = vec!["foo".to_string(), "zzz".to_string()];
    assert_eq!(score_def(&with_noise, "foo"), score_def(&one, "foo"));
    assert_eq!(score_caller(&with_noise, "foo"), score_caller(&one, "foo"));

    // Exact-vs-substring-vs-unrelated sweep: relevance ordering, no pins.
    for symbol in ["foobar", "render_widget", "parse"] {
        let exact = vec![symbol.to_string()];
        let sub: Vec<String> = vec![symbol.chars().take(symbol.len() - 1).collect()];
        let unrelated = vec!["zzzqqq".to_string()];
        assert!(
            score_def(&exact, symbol) > score_def(&sub, symbol),
            "{symbol}: exact must beat substring"
        );
        assert!(
            score_def(&sub, symbol) > score_def(&unrelated, symbol),
            "{symbol}: substring must beat unrelated"
        );
    }
}
