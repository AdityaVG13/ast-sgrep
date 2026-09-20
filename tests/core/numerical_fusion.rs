//! Canonical numerical contracts: RRF formulas, weighted fusion, learning, determinism.
//!
//! Consolidates the N1 (exactness) + N2 (totality) + N3 (metamorphic) legs of
//! `numerical_pass{1,2,3}.rs` into ONE contract test per function. Each test
//! carries INTENT + KILLS + ABSORBS. Overlap folds applied per
//! `tests/catalog/numerical-core.md`:
//!
//! * apply drop N1⊂N2 — the N1 `0/-1/NaN` drop assertions are dropped; the N2
//!   `±inf/NaN/0/neg` gate leg subsumes them.
//! * absent-channel/tie/permutation cluster — only unique halves kept: the
//!   removal-delta leg of `absent_channel_equals_zero_contribution` (the
//!   all-absent≡0.0 half is subsumed by the adding-absent leg) and the
//!   sorted-emission/key-rank leg of `tie_scores_break_identically` (the
//!   reversal half is subsumed by the permutation leg).
//!
//! Every expectation is HAND-DERIVED (rational or float derivation in the
//! comment above it). Nothing here snapshots production output.

use ast_sgrep_core::fusion::{
    analyze_weight_sensitivity, apply_weighted_rrf, learn_fusion_weights, weighted_rrf_score,
    ChannelRanks, FusionCandidate, FusionChannel, FusionExample,
};
use ast_sgrep_core::intent::{route_hits, ChannelWeights};
use ast_sgrep_core::rank::{fuse_rrf, rrf_score, score_caller, score_def, score_lexical_rrf};
use ast_sgrep_core::search::{HitKind, SearchHit};
use ast_sgrep_core::ParsedQuery;
use ast_sgrep_testkit::{
    fused_keys, mixed_hits, mk_hit, pair_examples, set_rank, set_weight, single_rank,
};

/// INTENT: `rrf_score` totality + rank/k monotonicity (pure-formula contract).
/// KILLS: guard-insertion-mutant, formula-sign-mutant.
/// ABSORBS: `rrf_score_hostile_k_and_extreme_rank` (N2) +
/// `rrf_score_decreases_in_rank_and_k` (N3).
#[test]
fn rrf_score_contract() {
    // --- Totality: rrf = 1/(k + rank + 1), no guards, IEEE all the way down.
    assert!(rrf_score(0, f64::NAN).is_nan()); // NaN k poisons the divisor -> NaN
    assert_eq!(rrf_score(0, f64::INFINITY), 0.0); // 1/inf
    assert_eq!(rrf_score(0, f64::NEG_INFINITY), -0.0); // 1/-inf
    assert!(rrf_score(0, f64::NEG_INFINITY).is_sign_negative());
    assert_eq!(rrf_score(0, -1.0), f64::INFINITY); // 1/(−1+0+1) = 1/+0
                                                   // 1/(-61+0+1) = 1/-60 = -(1/60): negation is exact, so bit-equality holds.
    assert_eq!(rrf_score(0, -61.0), -1.0 / 60.0);
    // 1/(-62+0+1) = 1/-61 = -(1/61): negation is exact, so bit-equality holds.
    assert_eq!(rrf_score(0, -62.0), -1.0 / 61.0);
    // usize::MAX as f64 is ~1.8e19: divisor finite, result finite ~5.4e-20.
    let huge = rrf_score(usize::MAX, 60.0);
    assert!(huge.is_finite() && huge > 0.0 && huge < 1.0 / 61.0);
    // Negative k below -61 makes every rank negative but still ordered: rank 0
    // (1/-61) is the largest (least negative), rank 1 (1/-60) smaller.
    assert!(rrf_score(0, -62.0) > rrf_score(1, -62.0));

    // --- Metamorphic: strictly decreasing in rank and in k (sane domain).
    // Larger divisor -> smaller quotient. Strictness holds because adjacent
    // divisors differ by 1.0 (exactly representable at these magnitudes) and
    // the quotients differ by ~1/61-1/62 ~= 2.6e-4, far above 1 ulp (~3e-20).
    for rank in 0..200usize {
        assert!(
            rrf_score(rank, 60.0) > rrf_score(rank + 1, 60.0),
            "rank {rank} must outscore rank {}",
            rank + 1
        );
    }
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

/// INTENT: `fuse_rrf`/`score_lexical_rrf` propagation + bulk finiteness +
/// superset-monotone/pair-commutative (sum contract).
/// KILLS: sum/scale-mutant, sum/max-swap-mutant.
/// ABSORBS: `fuse_rrf_nonfinite_propagation_and_bulk_sum` (N2) +
/// `fuse_rrf_superset_monotone_and_pair_commutative` (N3).
#[test]
fn fuse_rrf_contract() {
    // --- Totality: a single NaN term poisons the sequential sum.
    assert!(fuse_rrf(&[0, 1], f64::NAN).is_nan());
    // k=-1: every rank-0 term is +inf, sum is +inf (no saturation, no panic).
    assert_eq!(fuse_rrf(&[0], -1.0), f64::INFINITY);
    assert_eq!(fuse_rrf(&[0, 0, 0], -1.0), f64::INFINITY);
    // k=-61: finite negative terms 1/-60 + 1/-59; expected written with
    // production's op order (sequential sum of the two divisions).
    assert_eq!(
        fuse_rrf(&[0, 1], -61.0),
        1.0 / (-61.0 + 0.0 + 1.0) + 1.0 / (-61.0 + 1.0 + 1.0)
    );
    // k=-1.5: rrf(0) = 1/-0.5 = -2.0, rrf(1) = 1/0.5 = 2.0, exact cancel.
    assert_eq!(fuse_rrf(&[0, 1], -1.5), 0.0);
    // 1000 identical rank-0 terms: sequential sum then x200. Epsilon 1e-6
    // swamps accumulation error (~1e-13) while the *200-drop mutant misses by
    // ~3262 and a sum/max swap by ~16.
    let bulk = score_lexical_rrf(&[0; 1000]);
    assert!(bulk.is_finite());
    assert!((bulk - 200_000.0 / 61.0).abs() < 1e-6, "bulk = {bulk}");
    // Huge rank stays finite through the x200 scale.
    let one = score_lexical_rrf(&[usize::MAX]);
    assert!(one.is_finite() && one > 0.0);

    // --- Metamorphic: every term is positive (k = 60), so extending the rank
    // list strictly raises the sum: the added term (~1/61) dwarfs rounding.
    let mut ranks: Vec<usize> = vec![5, 3, 9];
    let before = fuse_rrf(&ranks, 60.0);
    ranks.push(0);
    assert!(fuse_rrf(&ranks, 60.0) > before);
    ranks.extend([1, 2, 100]);
    assert!(fuse_rrf(&ranks, 60.0) > before);
    // Two-term swap is BIT-identical: sum() starts from 0.0 (0.0 + x == x
    // exactly) and IEEE addition is commutative. Input order cannot leak in.
    assert_eq!(fuse_rrf(&[0, 1], 60.0), fuse_rrf(&[1, 0], 60.0));
    // Lexical scale-up: more per-term ranks never lower the scaled sum.
    for n in 1..16usize {
        let small = score_lexical_rrf(&vec![0; n]);
        let big = score_lexical_rrf(&vec![0; n + 1]);
        assert!(big > small, "n={n} must outscore n={}", n + 1);
    }
}

/// INTENT: `weighted_rrf_score` sanitize rails + rank/absent/scale/channel
/// metamorphics (weighted-sum contract).
/// KILLS: clamp-rail-mutant, rank-direction-mutant, absent-term-mutant,
/// channel-index-mutant, weight-application-mutant, empty-sum-mutant.
/// ABSORBS: `weighted_rrf_score_hostile_weights_and_extreme_ranks` (N2) +
/// `weighted_rrf_rank_improvement_never_lowers` (N3) +
/// `weighted_rrf_adding_absent_channel_never_lowers` (N3) +
/// `weighted_rrf_channel_pair_swap_invariant` (N3) +
/// `uniform_weight_scaling_preserves_pairwise_order` (N3) +
/// `absent_channel_equals_zero_contribution` removal-delta half only (N3; the
/// all-absent≡0.0 half is subsumed by the adding-absent leg).
#[test]
fn weighted_rrf_score_contract() {
    let unit = ChannelWeights::default();
    let one = single_rank(FusionChannel::Lexical, 0);

    // --- Totality: clamp_channel_weight rails.
    // Non-finite -> 1.0 (L2 pins NaN; inf takes the same branch and must agree
    // bit-for-bit).
    for w in [f64::INFINITY, f64::NEG_INFINITY] {
        let weights = ChannelWeights {
            lexical: w,
            ..Default::default()
        };
        assert_eq!(weighted_rrf_score(&one, &weights), 1.0 / 61.0);
    }
    // Finite but out of range clamps to the rails: -3.0 -> 0.25.
    // Expected written with production's op order (0.25 * (1/61)).
    let neg = ChannelWeights {
        lexical: -3.0,
        ..Default::default()
    };
    assert_eq!(weighted_rrf_score(&one, &neg), 0.25 * (1.0 / 61.0));
    // -0.0 is finite, so it clamps (up) to the same 0.25 rail, not to 1.0.
    let neg_zero = ChannelWeights {
        lexical: -0.0,
        ..Default::default()
    };
    assert_eq!(
        weighted_rrf_score(&one, &neg_zero),
        weighted_rrf_score(&one, &neg)
    );
    // 1e308 -> 2.0 rail.
    let huge_w = ChannelWeights {
        lexical: 1e308,
        ..Default::default()
    };
    assert_eq!(weighted_rrf_score(&one, &huge_w), 2.0 * (1.0 / 61.0));
    // Extreme rank: single tiny-but-finite positive contribution.
    let max_rank = single_rank(FusionChannel::Lexical, usize::MAX);
    let got = weighted_rrf_score(&max_rank, &unit);
    assert!(got.is_finite() && got > 0.0 && got < 1.0 / 61.0);
    // All 8 channels at rank 0 with 1e308 weights: 8 clamped terms sum to
    // ~16/61, finite. Epsilon 1e-12 vs multi-term accumulation (kills
    // clamp-removal, which would yield ~1e306, and absent-channel mutants).
    let mut all = ChannelRanks::default();
    for channel in FusionChannel::ALL {
        set_rank(&mut all, channel, Some(0));
    }
    let all_huge = ChannelWeights {
        lexical: 1e308,
        def: 1e308,
        caller: 1e308,
        graph: 1e308,
        anchor: 1e308,
        embed: 1e308,
        pattern: 1e308,
        import: 1e308,
    };
    let fused = weighted_rrf_score(&all, &all_huge);
    assert!(fused.is_finite());
    assert!((fused - 16.0 / 61.0).abs() < 1e-12, "fused = {fused}");

    // --- Metamorphic 1: improving any single channel rank never lowers.
    // Unit weights (clamp identity): rank 5 -> rank 0 swaps 1/66 for 1/61, a
    // gain of ~1.3e-3 against ~1e-16 rounding.
    for channel in FusionChannel::ALL {
        let worse = weighted_rrf_score(&single_rank(channel, 5), &unit);
        let better = weighted_rrf_score(&single_rank(channel, 0), &unit);
        assert!(
            better > worse,
            "{channel:?}: rank 0 must outscore rank 5 ({better} vs {worse})"
        );
    }
    // Same relation against a populated background: other channels held fixed.
    let mut background = ChannelRanks::default();
    for channel in FusionChannel::ALL {
        set_rank(&mut background, channel, Some(4));
    }
    for channel in FusionChannel::ALL {
        let mut improved = background.clone();
        set_rank(&mut improved, channel, Some(0));
        assert!(
            weighted_rrf_score(&improved, &unit) > weighted_rrf_score(&background, &unit),
            "{channel:?}: rank improvement over a background must raise the sum"
        );
    }

    // --- Metamorphic 2: adding an absent channel never lowers.
    // None contributes no term; Some(rank) contributes a positive term, so the
    // sum strictly grows (gain >= 0.25/(61+rank) >> rounding).
    let empty = ChannelRanks::default();
    let empty_score = weighted_rrf_score(&empty, &unit);
    for channel in FusionChannel::ALL {
        let added = weighted_rrf_score(&single_rank(channel, 2), &unit);
        assert!(
            added > empty_score,
            "{channel:?}: adding a channel must raise the sum"
        );
    }
    // Incremental build-up over all 8 channels is a strictly rising chain.
    let mut ranks = ChannelRanks::default();
    let mut prev = weighted_rrf_score(&ranks, &unit);
    for (i, channel) in FusionChannel::ALL.iter().enumerate() {
        set_rank(&mut ranks, *channel, Some(i));
        let next = weighted_rrf_score(&ranks, &unit);
        assert!(next > prev, "chain must rise at {channel:?}");
        prev = next;
    }

    // --- Metamorphic 3: channel assignment order is irrelevant.
    // Swap the (rank, weight) pairs of two channels: the two fused terms are
    // identical as a multiset and IEEE `+` is commutative, so the two-term
    // sum is BIT-identical. Weights 1.5/0.5 sit inside the [0.25, 2.0] rails
    // so the clamp is the identity on both sides.
    let mut weights_a = ChannelWeights::default();
    set_weight(&mut weights_a, FusionChannel::Lexical, 1.5);
    set_weight(&mut weights_a, FusionChannel::Definition, 0.5);
    let mut ranks_a = ChannelRanks::default();
    set_rank(&mut ranks_a, FusionChannel::Lexical, Some(0));
    set_rank(&mut ranks_a, FusionChannel::Definition, Some(3));
    let mut weights_b = ChannelWeights::default();
    set_weight(&mut weights_b, FusionChannel::Lexical, 0.5);
    set_weight(&mut weights_b, FusionChannel::Definition, 1.5);
    let mut ranks_b = ChannelRanks::default();
    set_rank(&mut ranks_b, FusionChannel::Lexical, Some(3));
    set_rank(&mut ranks_b, FusionChannel::Definition, Some(0));
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
        set_rank(&mut ranks_f, *channel, Some(i));
        set_weight(&mut weights_f, *channel, rail_weights[i]);
        set_rank(&mut ranks_r, *channel, Some(7 - i));
        set_weight(&mut weights_r, *channel, rail_weights[7 - i]);
    }
    let forward = weighted_rrf_score(&ranks_f, &weights_f);
    let reversed = weighted_rrf_score(&ranks_r, &weights_r);
    assert!(
        (forward - reversed).abs() < 1e-14,
        "8-channel reversal must agree: {forward} vs {reversed}"
    );

    // --- Metamorphic 4: uniform weight scaling preserves pairwise ordering.
    // Scale every channel 1.0 -> 1.5 (inside the rails, so no clamp skew):
    // each fused score multiplies by ~1.5, so every pairwise ordering sign
    // is preserved. Candidates are separated by >= ~1.3e-3 (a full rank step
    // at low ranks), ~1e13x above rounding, so no order can flip.
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
        set_rank(&mut all_zero, channel, Some(0));
    }
    let mut two_zero = ChannelRanks::default();
    set_rank(&mut two_zero, FusionChannel::Lexical, Some(0));
    set_rank(&mut two_zero, FusionChannel::Definition, Some(0));
    let candidates = [
        ChannelRanks::default(),
        single_rank(FusionChannel::Lexical, 5),
        single_rank(FusionChannel::Lexical, 0),
        two_zero,
        all_zero,
    ];
    let base: Vec<f64> = candidates
        .iter()
        .map(|c| weighted_rrf_score(c, &unit))
        .collect();
    let up: Vec<f64> = candidates
        .iter()
        .map(|c| weighted_rrf_score(c, &scaled))
        .collect();
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

    // --- Metamorphic 5 (unique half): removing one present channel lowers the
    // sum by exactly that channel's term, up to sequential-sum reassociation
    // (~1e-16 at sum magnitude ~0.2; 1e-12 keeps 10000x headroom). Weights sit
    // on the rails' interior so the clamp is the identity and the oracle term
    // needs no clamp model.
    let rail_weights = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 0.3];
    let mut ranks = ChannelRanks::default();
    let mut weights = ChannelWeights::default();
    for (i, channel) in FusionChannel::ALL.iter().enumerate() {
        set_rank(&mut ranks, *channel, Some(2 * i));
        set_weight(&mut weights, *channel, rail_weights[i]);
    }
    let full = weighted_rrf_score(&ranks, &weights);
    for (i, channel) in FusionChannel::ALL.iter().enumerate() {
        let mut without = ranks.clone();
        set_rank(&mut without, *channel, None);
        let reduced = weighted_rrf_score(&without, &weights);
        assert!(reduced < full, "{channel:?}: removal must lower the sum");
        let term = rail_weights[i] * rrf_score(2 * i, 60.0);
        assert!(
            (full - reduced - term).abs() < 1e-12,
            "{channel:?}: removal delta must equal the channel term"
        );
    }
}

/// INTENT: `apply_weighted_rrf` fused sums + canonical member + rank order +
/// nonfinite gate + permutation/tie invariance (fusion contract).
/// KILLS: fuse-sum/canonical-mutant, rank-order/gate-mutant,
/// gate/sanitize-mutant, order-leak-mutant, tiebreak-mutant.
/// ABSORBS: `apply_weighted_rrf_single_and_merge_exact` (N1) +
/// `apply_weighted_rrf_rank_order_and_zero_drop` rank-order half only (N1; the
/// 0/-1/NaN drop half is N1⊂N2, subsumed by the gate leg) +
/// `apply_weighted_rrf_drops_nonfinite_keeps_extreme_finite` (N2) +
/// `apply_weighted_rrf_input_permutation_bit_identical` (N3) +
/// `tie_scores_break_identically` sorted-emission/key-rank half only (N3; the
/// reversal half is subsumed by the permutation leg).
#[test]
fn apply_weighted_rrf_contract() {
    let weights = ChannelWeights::default(); // all 1.0 -> weight is identity

    // --- Exactness: single hit and two-channel merge.
    // Single hit: the only channel rank is 0 -> fused = 1*1/(60+0+1) = 1/61.
    let mut single = vec![mk_hit(HitKind::Asgrep, "a.rs", 1, 3.0)];
    apply_weighted_rrf(&mut single, &weights);
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].score, 1.0 / 61.0);
    // Same (file, line), two kinds: each channel's rank is 0, fused is the
    // SUM 1/61 + 1/61 (written as the literal sum: 2/61 can differ by 1 ulp
    // because doubling crosses a binade boundary vs. one division).
    // Canonical member: Def priority 0 beats Caller priority 1.
    let mut merged = vec![
        mk_hit(HitKind::Caller, "m.rs", 7, 8.0),
        mk_hit(HitKind::Def, "m.rs", 7, 5.0),
    ];
    apply_weighted_rrf(&mut merged, &weights);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].score, 1.0 / 61.0 + 1.0 / 61.0);
    assert_eq!(merged[0].kind, HitKind::Def);
    assert_eq!(
        merged[0].contributors,
        vec![HitKind::Def, HitKind::Caller],
        "contributors sort by channel index: Definition(1) < Caller(2)"
    );

    // --- Exactness: within-channel rank order + empty no-op.
    // One channel, two result keys: within-channel rank = score-desc order,
    // so a.rs (9.0) is rank 0 -> 1/61, b.rs (8.0) is rank 1 -> 1/62.
    // Emission order is sorted (file, line): [a.rs, b.rs].
    let mut hits = vec![
        mk_hit(HitKind::Def, "b.rs", 1, 8.0),
        mk_hit(HitKind::Def, "a.rs", 1, 9.0),
    ];
    apply_weighted_rrf(&mut hits, &weights);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].file, "a.rs");
    assert_eq!(hits[0].score, 1.0 / 61.0);
    assert_eq!(hits[1].file, "b.rs");
    assert_eq!(hits[1].score, 1.0 / 62.0);
    // Empty input is a no-op, not a panic.
    let mut empty: Vec<SearchHit> = vec![];
    apply_weighted_rrf(&mut empty, &weights);
    assert!(empty.is_empty());

    // --- Totality: gate is `is_finite() && > 0.0`.
    // NaN, +-inf, zero, and negatives never enter a channel or the member map
    // (this leg subsumes the N1 0/-1/NaN drop half: N1⊂N2).
    let mut bad = vec![
        mk_hit(HitKind::Def, "a.rs", 1, f64::INFINITY),
        mk_hit(HitKind::Def, "b.rs", 2, f64::NEG_INFINITY),
        mk_hit(HitKind::Def, "c.rs", 3, f64::NAN),
        mk_hit(HitKind::Def, "d.rs", 4, 0.0),
        mk_hit(HitKind::Def, "e.rs", 5, -1e308),
    ];
    apply_weighted_rrf(&mut bad, &weights);
    assert!(bad.is_empty());
    // Subnormal and 1e308 are finite and positive -> kept. Fused scores depend
    // only on rank order, so each lone hit in its channel is rank 0 -> 1/61.
    let mut extreme = vec![
        mk_hit(HitKind::Def, "s.rs", 1, 5e-324),
        mk_hit(HitKind::Caller, "l.rs", 2, 1e308),
    ];
    apply_weighted_rrf(&mut extreme, &weights);
    assert_eq!(extreme.len(), 2);
    for hit in &extreme {
        assert_eq!(hit.score, 1.0 / 61.0);
        assert!(hit.score.is_finite());
    }
    // Same (file, line): the +inf Asgrep member is excluded before ranking, so
    // only the finite Def member survives -> single channel, rank 0 -> 1/61.
    let mut mixed = vec![
        mk_hit(HitKind::Asgrep, "m.rs", 7, f64::INFINITY),
        mk_hit(HitKind::Def, "m.rs", 7, 0.5),
    ];
    apply_weighted_rrf(&mut mixed, &weights);
    assert_eq!(mixed.len(), 1);
    assert_eq!(mixed[0].score, 1.0 / 61.0);
    assert_eq!(mixed[0].contributors, vec![HitKind::Def]);
    // Hostile weights are sanitized per-channel (weighted_rrf rails): finite.
    let hostile = ChannelWeights {
        lexical: f64::NAN,
        def: f64::INFINITY,
        caller: f64::NEG_INFINITY,
        graph: -1e308,
        anchor: 1e308,
        embed: -0.0,
        pattern: 5e-324,
        import: 2.5,
    };
    let mut hits = vec![
        mk_hit(HitKind::Def, "a.rs", 1, 3.0),
        mk_hit(HitKind::Caller, "b.rs", 2, 2.0),
    ];
    apply_weighted_rrf(&mut hits, &hostile);
    assert_eq!(hits.len(), 2);
    for hit in &hits {
        assert!(hit.score.is_finite() && hit.score > 0.0);
    }

    // --- Metamorphic: input hit order is bit-irrelevant.
    // Fusion groups by channel, ranks within channel by (score, file, line),
    // and emits in sorted key order — so the input Vec order (reverse,
    // rotation, halved swap) must vanish bit-for-bit, including contributors.
    let base = mixed_hits();
    let expected = fused_keys(&base);
    let mut reversed = base.clone();
    reversed.reverse();
    assert_eq!(fused_keys(&reversed), expected, "reversed input");
    let mut rotated = base.clone();
    rotated.rotate_left(3);
    assert_eq!(fused_keys(&rotated), expected, "rotated input");
    let mut swapped = base.clone();
    let half = swapped.len() / 2;
    swapped[..half].reverse();
    swapped[half..].reverse();
    assert_eq!(fused_keys(&swapped), expected, "halved input");

    // --- Metamorphic (unique tie half): every input score is the same 5.0, so
    // within-channel ranks come purely from the (file, line) tiebreaks: Def
    // a=0 b=1 c=2 -> 1/61, 1/62, 1/63; Caller/Asgrep singletons -> 1/61.
    // Emission is in sorted key order: the tie order is total.
    let mut tied = vec![
        mk_hit(HitKind::Def, "a.rs", 1, 5.0),
        mk_hit(HitKind::Def, "b.rs", 1, 5.0),
        mk_hit(HitKind::Def, "c.rs", 1, 5.0),
        mk_hit(HitKind::Caller, "a.rs", 2, 5.0),
        mk_hit(HitKind::Asgrep, "b.rs", 2, 5.0),
    ];
    apply_weighted_rrf(&mut tied, &weights);
    assert_eq!(tied.len(), 5);
    let by_key = |f: &str, line: u32| {
        tied.iter()
            .find(|h| h.file == f && h.line_start == line)
            .unwrap()
            .score
    };
    assert_eq!(by_key("a.rs", 1), 1.0 / 61.0);
    assert_eq!(by_key("b.rs", 1), 1.0 / 62.0);
    assert_eq!(by_key("c.rs", 1), 1.0 / 63.0);
    assert_eq!(by_key("a.rs", 2), 1.0 / 61.0);
    assert_eq!(by_key("b.rs", 2), 1.0 / 61.0);
    let keys: Vec<(&str, u32)> = tied
        .iter()
        .map(|h| (h.file.as_str(), h.line_start))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted, "fused emission must be in sorted key order");
}

/// INTENT: `learn_fusion_weights` softplus loss + rail convergence + degenerate
/// totality + pair-order invariance (learner contract).
/// KILLS: loss-formula/search-step-mutant, pair-filter/entry-sanitize-mutant,
/// pair-normalization-mutant.
/// ABSORBS: `learn_fusion_weights_single_pair_exact` (N1) +
/// `learn_fusion_weights_degenerate_relevance_and_initial_weights` (N2) +
/// `learn_pair_candidate_order_invariant` (N3).
#[test]
fn learn_fusion_weights_contract() {
    use std::f64::consts::LN_2;

    // --- Exactness: better ranks {lexical: 0} -> s = 1/61; worse {lexical: 1}
    // -> 1/62. delta = (1/61 - 1/62)*100 = (1/3782)*100 = 50/1891 > 0,
    // loss = ln(1 + e^-delta) (single pair -> mean over 1).
    // Independent oracle (python3 math.log1p(math.exp(-50/1891))):
    // loss_before = 0.680014050821341.
    // Epsilon 1e-12: production computes delta as (1/61 - 1/62)*100 (a few
    // ulps from 50/1891) and exp/log1p are libm (<=1 ulp each); 1e-12 swamps
    // that while still killing formula mutants (missing *100 shifts loss by
    // ~0.013, wrong branch by orders more).
    let examples = pair_examples();
    let model = learn_fusion_weights(&examples, ChannelWeights::default());
    assert!(
        (model.loss_before - 0.680014050821341).abs() < 1e-12,
        "loss_before = {}, want 0.680014050821341",
        model.loss_before
    );
    // Learner trace: only Lexical is stiff; loss decreases monotonically in
    // w_lex (delta = w*100/3782 > 0), so +steps always accept: 1.0 -> 1.25
    // -> 1.475 -> 1.6775 -> 1.85975 -> clamp(2.023775) = 2.0 exactly; then
    // -steps lose and +steps clamp (equal loss, rejected by the 1e-12
    // improvement gate) until step < 1e-3. loss_after = ln(1+e^(-100/1891))
    // = 0.6670556675524925 (same python oracle).
    assert!(model.loss_after < model.loss_before);
    assert!(
        (model.loss_after - 0.6670556675524925).abs() < 1e-12,
        "loss_after = {}, want 0.6670556675524925",
        model.loss_after
    );
    assert_eq!(model.weights.lexical, 2.0);
    for (name, v) in [
        ("def", model.weights.def),
        ("caller", model.weights.caller),
        ("graph", model.weights.graph),
        ("anchor", model.weights.anchor),
        ("embed", model.weights.embed),
        ("pattern", model.weights.pattern),
        ("import", model.weights.import),
    ] {
        assert_eq!(v, 1.0, "non-stiff channel {name} must stay at 1.0");
    }
    // Degenerate inputs: no pairs -> loss exactly 0.
    let tied = vec![FusionExample {
        query: "q".to_string(),
        candidates: vec![
            FusionCandidate {
                id: "a".to_string(),
                relevance: 1.0,
                ranks: ChannelRanks::default(),
            },
            FusionCandidate {
                id: "b".to_string(),
                relevance: 1.0,
                ranks: ChannelRanks::default(),
            },
        ],
    }];
    let tied_model = learn_fusion_weights(&tied, ChannelWeights::default());
    assert_eq!(tied_model.loss_before, 0.0);
    assert_eq!(tied_model.loss_after, 0.0);
    let empty_model = learn_fusion_weights(&[], ChannelWeights::default());
    assert_eq!(empty_model.loss_before, 0.0);
    assert_eq!(empty_model.loss_after, 0.0);

    // --- Totality: NaN relevance: both `>` comparisons are false -> `continue`,
    // exactly as if the pair were tied. No pairs -> both losses exactly 0.0,
    // weights untouched (unit initial survives entry sanitize bit-identical).
    let nan_rel = vec![FusionExample {
        query: "q".to_string(),
        candidates: vec![
            FusionCandidate {
                id: "a".to_string(),
                relevance: f64::NAN,
                ranks: single_rank(FusionChannel::Lexical, 0),
            },
            FusionCandidate {
                id: "b".to_string(),
                relevance: f64::NAN,
                ranks: single_rank(FusionChannel::Lexical, 1),
            },
        ],
    }];
    let nan_model = learn_fusion_weights(&nan_rel, ChannelWeights::default());
    assert_eq!(nan_model.loss_before, 0.0);
    assert_eq!(nan_model.loss_after, 0.0);
    assert_eq!(nan_model.weights, ChannelWeights::default());
    // +-inf relevance still orders (+inf is "better"); but with empty rank
    // sets both weighted scores are 0.0 -> delta = 0 -> loss ln(1+e^0) = ln 2.
    // Epsilon 1e-12: libm ln_1p is <=1 ulp; the empty-pair/0.0 mutant misses
    // by 0.69 and a relevance-ignored mutant can only match by accident here.
    let inf_rel = vec![FusionExample {
        query: "q".to_string(),
        candidates: vec![
            FusionCandidate {
                id: "lo".to_string(),
                relevance: f64::NEG_INFINITY,
                ranks: ChannelRanks::default(),
            },
            FusionCandidate {
                id: "hi".to_string(),
                relevance: f64::INFINITY,
                ranks: ChannelRanks::default(),
            },
        ],
    }];
    let inf_model = learn_fusion_weights(&inf_rel, ChannelWeights::default());
    assert!(
        (inf_model.loss_before - LN_2).abs() < 1e-12,
        "loss = {}, want ln 2",
        inf_model.loss_before
    );
    // No channel touches the loss (no ranks anywhere) -> nothing stiff ->
    // coordinate search never moves -> loss_after is bit-identical.
    assert_eq!(inf_model.loss_after, inf_model.loss_before);
    assert_eq!(inf_model.weights, ChannelWeights::default());
    // Hostile initial weights are sanitized at learn entry (weight+set_weight
    // rails), before any loss is computed. Empty examples -> losses 0.
    let hostile = ChannelWeights {
        lexical: f64::NAN,         // non-finite -> 1.0
        def: f64::INFINITY,        // non-finite -> 1.0
        caller: f64::NEG_INFINITY, // non-finite -> 1.0
        graph: 1e308,              // -> 2.0 rail
        anchor: -1e308,            // -> 0.25 rail
        embed: -0.0,               // finite -> 0.25 rail
        pattern: 5e-324,           // finite tiny -> 0.25 rail
        import: 2.5,               // -> 2.0 rail
    };
    let m = learn_fusion_weights(&[], hostile);
    assert_eq!(m.loss_before, 0.0);
    assert_eq!(m.loss_after, 0.0);
    assert_eq!(m.weights.lexical, 1.0);
    assert_eq!(m.weights.def, 1.0);
    assert_eq!(m.weights.caller, 1.0);
    assert_eq!(m.weights.graph, 2.0);
    assert_eq!(m.weights.anchor, 0.25);
    assert_eq!(m.weights.embed, 0.25);
    assert_eq!(m.weights.pattern, 0.25);
    assert_eq!(m.weights.import, 2.0);

    // --- Metamorphic: pairwise_loss normalizes the pair by the relevance
    // comparison, so listing the worse candidate first or second feeds the
    // identical delta into the identical coordinate search — bit-identical
    // losses, weights, and sensitivity table.
    let worse_first = pair_examples();
    let mut better_first = pair_examples();
    better_first[0].candidates.reverse();
    assert_eq!(
        learn_fusion_weights(&worse_first, ChannelWeights::default()),
        learn_fusion_weights(&better_first, ChannelWeights::default())
    );
}

/// INTENT: `analyze_weight_sensitivity` live gradient/curvature/stiff ladder +
/// hostile-step sanitize + degenerate all-zero tables (sensitivity contract).
/// KILLS: finite-difference/stiff-flag-mutant, step-sanitize-mutant.
/// ABSORBS: `sensitivity_empty_and_absent_channels` live/absent halves only
/// (N1; the empty-zero half is N1⊂N2, subsumed by the hostile-step loop) +
/// `sensitivity_hostile_step_and_degenerate_examples` (N2).
#[test]
fn sensitivity_contract() {
    let unit = ChannelWeights::default();

    // --- Exactness (live): raising the lexical weight widens delta and lowers
    // the softplus loss -> gradient < 0; softplus is strictly convex ->
    // curvature ~1.7e-4 > 0 (second difference >> ulp(0.68)); the pair order
    // never flips -> churn 0; lexical owns max curvature -> stiff.
    let live = analyze_weight_sensitivity(&pair_examples(), &unit, 0.1);
    let lex = live
        .iter()
        .find(|r| r.channel == FusionChannel::Lexical)
        .unwrap();
    assert!(lex.gradient < 0.0, "gradient = {}", lex.gradient);
    assert!(lex.curvature > 0.0, "curvature = {}", lex.curvature);
    assert_eq!(lex.rank_churn, 0.0);
    assert!(lex.stiff);
    // Channels absent from every rank never touch the loss: perturbing them
    // replays bit-identical arithmetic -> exact zeros, never stiff.
    for channel in [FusionChannel::Graph, FusionChannel::Pattern] {
        let row = live.iter().find(|r| r.channel == channel).unwrap();
        assert_eq!(row.gradient, 0.0);
        assert_eq!(row.curvature, 0.0);
        assert_eq!(row.rank_churn, 0.0);
        assert!(!row.stiff);
    }

    // --- Totality: step sanitize: non-finite or non-positive -> 0.1; > 0.5
    // caps at 0.5. Same sanitized value -> bit-identical tables (same
    // arithmetic replayed).
    let live_ex = pair_examples();
    let via_inf = analyze_weight_sensitivity(&live_ex, &unit, f64::INFINITY);
    let via_nan = analyze_weight_sensitivity(&live_ex, &unit, f64::NAN);
    assert_eq!(
        via_inf, via_nan,
        "inf and NaN steps must sanitize identically"
    );
    let via_huge = analyze_weight_sensitivity(&live_ex, &unit, 1e308);
    let via_half = analyze_weight_sensitivity(&live_ex, &unit, 0.5);
    assert_eq!(via_huge, via_half, "1e308 step must cap at 0.5");
    let via_neg = analyze_weight_sensitivity(&live_ex, &unit, -1.0);
    assert_eq!(via_neg, via_nan, "negative step must sanitize to 0.1");
    // Empty examples: loss is identically 0 for ANY sanitized step, so every
    // row is an exact zero and nothing is stiff. Proves hostile steps never
    // reach a division (no div-by-zero -> no NaN gradient/curvature). This
    // loop subsumes the N1 empty-zero half (N1⊂N2).
    for step in [f64::INFINITY, f64::NEG_INFINITY, -1.0, 0.0, 1e308, 5e-324] {
        let rows = analyze_weight_sensitivity(&[], &unit, step);
        assert_eq!(rows.len(), FusionChannel::ALL.len());
        for (row, channel) in rows.iter().zip(FusionChannel::ALL) {
            assert_eq!(row.channel, channel);
            assert_eq!(row.gradient, 0.0, "step {step}");
            assert_eq!(row.curvature, 0.0, "step {step}");
            assert_eq!(row.rank_churn, 0.0, "step {step}");
            assert!(!row.stiff, "step {step}");
        }
    }
    // NaN relevances -> zero pairs -> same all-zero table as empty examples.
    let nan_rel = vec![FusionExample {
        query: "q".to_string(),
        candidates: vec![
            FusionCandidate {
                id: "a".to_string(),
                relevance: f64::NAN,
                ranks: single_rank(FusionChannel::Lexical, 0),
            },
            FusionCandidate {
                id: "b".to_string(),
                relevance: f64::NAN,
                ranks: ChannelRanks::default(),
            },
        ],
    }];
    let rows = analyze_weight_sensitivity(&nan_rel, &unit, 0.1);
    assert_eq!(rows.len(), 8);
    for row in &rows {
        assert_eq!(row.gradient, 0.0);
        assert_eq!(row.curvature, 0.0);
        assert_eq!(row.rank_churn, 0.0);
        assert!(!row.stiff);
    }
}

/// INTENT: scoring determinism — serial replay and 8-thread concurrency are
/// bit-identical across all pure fns + learn/sensitivity (determinism contract).
/// KILLS: nondeterminism-mutant, shared-mutable-state-mutant.
/// ABSORBS: `repeated_calls_bit_identical` (N3) +
/// `multithreaded_scoring_bit_identical` (N3).
#[test]
fn determinism_contract() {
    // Same inputs replayed bit-for-bit must reproduce bit-for-bit: every fn
    // below is a pure function of its arguments (no HashMap iteration leaks
    // into outputs, no time/randomness).
    let mut ranks = ChannelRanks::default();
    set_rank(&mut ranks, FusionChannel::Lexical, Some(1));
    set_rank(&mut ranks, FusionChannel::Definition, Some(0));
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
            .map(|(i, s)| mk_hit(HitKind::Embed, "r.rs", i as u32, *s))
            .collect();
        route_hits(&parsed, &mut hits);
        hits.iter().map(|h| h.score.to_bits()).collect::<Vec<_>>()
    };
    assert_eq!(route_once(&[0.5, 2.0, 9.0]), route_once(&[0.5, 2.0, 9.0]));
    assert_eq!(fused_keys(&mixed_hits()), fused_keys(&mixed_hits()));

    let examples = pair_examples();
    assert_eq!(
        learn_fusion_weights(&examples, ChannelWeights::default()),
        learn_fusion_weights(&examples, ChannelWeights::default())
    );
    assert_eq!(
        analyze_weight_sensitivity(&examples, &ChannelWeights::default(), 0.1),
        analyze_weight_sensitivity(&examples, &ChannelWeights::default(), 0.1)
    );

    // 8 threads score the same fusion + learning workload; every thread must
    // agree bit-for-bit with the serial result (no shared mutable state, no
    // HashMap-order leakage — apply_weighted_rrf emits in sorted key order).
    let expected_fused = fused_keys(&mixed_hits());
    let expected_model = learn_fusion_weights(&pair_examples(), ChannelWeights::default());
    let expected_sens =
        analyze_weight_sensitivity(&pair_examples(), &ChannelWeights::default(), 0.1);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..8 {
            handles.push(scope.spawn(|| {
                let fused = fused_keys(&mixed_hits());
                let model = learn_fusion_weights(&pair_examples(), ChannelWeights::default());
                let sens =
                    analyze_weight_sensitivity(&pair_examples(), &ChannelWeights::default(), 0.1);
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
