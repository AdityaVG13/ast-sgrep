//! N2 nonfinite/degenerate-totality oracles for the ast-sgrep-core float surface.
//!
//! N1 pins exact values on sane inputs; L2 pins the RRF/fusion happy path.
//! This suite proves TOTALITY: every core scoring fn terminates (never panics)
//! on hostile numeric inputs, and each test pins the REAL outcome class.
//!
//! Totality table (OD = outcome discriminant asserted):
//!
//! | fn | hostile input | real outcome | OD |
//! |----|---------------|--------------|----|
//! | `rrf_score` | k=NaN | propagate NaN | `is_nan` |
//! | `rrf_score` | k=+inf / -inf | 0.0 / -0.0 | value + sign |
//! | `rrf_score` | k=-1 (zero divisor) / k=-61 | +inf / -(1/60) | values |
//! | `rrf_score` | rank=usize::MAX | finite tiny | range |
//! | `fuse_rrf` | k=NaN / k=-1 / k=-61 | NaN / +inf / finite neg | discriminants |
//! | `fuse_rrf` | exact-cancel k=-1.5 | 0.0 | value |
//! | `score_lexical_rrf` | 1000 ranks / huge rank | finite scaled sum | range |
//! | `weighted_rrf_score` | weight ±inf/NaN | sanitize to 1.0 | value |
//! | `weighted_rrf_score` | weight 1e308 / -3 / -0.0 | clamp 2.0 / 0.25 | value |
//! | `apply_weighted_rrf` | score ±inf/NaN | dropped from output | emptiness |
//! | `apply_weighted_rrf` | subnormal / 1e308 score | kept, finite fused | value |
//! | `route_hits` | score NaN | PROPAGATES (clamp no-op) | `is_nan` |
//! | `route_hits` | score ±inf / ±1e308 | clamp 1.0 / 0.0 | value |
//! | `learn_fusion_weights` | NaN relevance | pair skipped, loss 0 | value |
//! | `learn_fusion_weights` | empty ranks, split relevance | loss ln 2 | epsilon |
//! | `learn_fusion_weights` | NaN/inf/rail initial weights | sanitized rails | values |
//! | `analyze_weight_sensitivity` | step ±inf/NaN/neg/0 | sanitize to 0.1 | table equality |
//! | `finish_response` | NaN/±inf scores | retained, margin 0 | values |
//! | `margin_is_decisive` | NaN/inf/subnormal | false / ratio rule | bools |
//! | `search_flat` | NaN/inf/zero query | gated to empty | emptiness |
//! | `search_flat` | NaN row / f32-overflow query | row dropped / empty | values |
//! | `search_flat` | dim 0 / huge limit | empty / fine, no panic | discriminants |
//! | `expand_chain` | decay NaN/+inf/-2/-0.0 | propagated hops, no panic | bit-equality |
//! | `score_def/caller` | 100k terms | exact 1000003 / 1000001.5 | values |
//! | `score_def/caller` | empty / sub-floor terms | 0.0 (no base award) | value |
//! | `candidate_indices` | NaN query / usize::MAX probes | zero-equiv / clamp | equality |
//! | `reassign_all` | empty / dim-mismatch | false, no panic | bools |
//!
//! Deliberately NOT covered (same reason as N1): `search::critic`
//! multipliers and `search::field_weight` mixing are `pub(crate)` —
//! unreachable from integration tests. Pattern scoring is the constant
//! `SCORE_PATTERN` (no numeric surface beyond the N1-pinned ceiling).

use ast_sgrep_core::chain::{expand_chain, ChainConfig};
use ast_sgrep_core::fusion::{
    analyze_weight_sensitivity, apply_weighted_rrf, learn_fusion_weights, weighted_rrf_score,
    ChannelRanks, FusionCandidate, FusionChannel, FusionExample,
};
use ast_sgrep_core::intent::{route_hits, ChannelWeights};
use ast_sgrep_core::rank::{fuse_rrf, rrf_score, score_caller, score_def, score_lexical_rrf};
use ast_sgrep_core::search::{
    finish_response, margin_is_decisive, HitKind, SearchHit, SearchOptions, SpanHitInput,
};
use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use ast_sgrep_core::store::{CallerRow, SymbolRow, UpsertFileInput};
use ast_sgrep_core::{IndexStore, ParsedQuery};
use std::f64::consts::LN_2;

fn n2_hit(kind: HitKind, file: &str, line: u32, score: f64) -> SearchHit {
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

fn n2_options(root: &std::path::Path, limit: usize) -> SearchOptions {
    SearchOptions {
        root: root.to_path_buf(),
        limit,
        file_filter: None,
        count_only: false,
        use_rerank: false,
        ..SearchOptions::default()
    }
}

fn n2_pair_example() -> Vec<FusionExample> {
    // Same live pair as N1 (worse first): lexical ranks 1 vs 0.
    vec![FusionExample {
        query: "q".to_string(),
        candidates: vec![
            FusionCandidate {
                id: "worse".to_string(),
                relevance: 0.0,
                ranks: ChannelRanks {
                    lexical: Some(1),
                    ..ChannelRanks::default()
                },
            },
            FusionCandidate {
                id: "better".to_string(),
                relevance: 1.0,
                ranks: ChannelRanks {
                    lexical: Some(0),
                    ..ChannelRanks::default()
                },
            },
        ],
    }]
}

// ---------------------------------------------------------------------------
// 1. rrf_score: pure formula, propagates nonfinite k, total on huge ranks
// ---------------------------------------------------------------------------

#[test]
fn rrf_score_hostile_k_and_extreme_rank() {
    // rrf = 1/(k + rank + 1): no guards, IEEE semantics all the way down.
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
}

// ---------------------------------------------------------------------------
// 2. fuse_rrf / score_lexical_rrf: sum propagation + bulk-sum finiteness
// ---------------------------------------------------------------------------

#[test]
fn fuse_rrf_nonfinite_propagation_and_bulk_sum() {
    // A single NaN term poisons the sequential sum.
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
}

// ---------------------------------------------------------------------------
// 3. weighted_rrf_score: weight sanitize rails + extreme ranks
// ---------------------------------------------------------------------------

#[test]
fn weighted_rrf_score_hostile_weights_and_extreme_ranks() {
    let one = ChannelRanks {
        lexical: Some(0),
        ..ChannelRanks::default()
    };
    // clamp_channel_weight: non-finite -> 1.0 (L2 pins NaN; inf takes the same
    // branch and must agree bit-for-bit).
    for w in [f64::INFINITY, f64::NEG_INFINITY] {
        let weights = ChannelWeights {
            lexical: w,
            ..ChannelWeights::default()
        };
        assert_eq!(weighted_rrf_score(&one, &weights), 1.0 / 61.0);
    }
    // Finite but out of range clamps to the rails: -3.0 -> 0.25.
    // Expected written with production's op order (0.25 * (1/61)).
    let neg = ChannelWeights {
        lexical: -3.0,
        ..ChannelWeights::default()
    };
    assert_eq!(weighted_rrf_score(&one, &neg), 0.25 * (1.0 / 61.0));
    // -0.0 is finite, so it clamps (up) to the same 0.25 rail, not to 1.0.
    let neg_zero = ChannelWeights {
        lexical: -0.0,
        ..ChannelWeights::default()
    };
    assert_eq!(
        weighted_rrf_score(&one, &neg_zero),
        weighted_rrf_score(&one, &neg)
    );
    // 1e308 -> 2.0 rail.
    let huge = ChannelWeights {
        lexical: 1e308,
        ..ChannelWeights::default()
    };
    assert_eq!(weighted_rrf_score(&one, &huge), 2.0 * (1.0 / 61.0));
    // Extreme rank: single tiny-but-finite positive contribution.
    let max_rank = ChannelRanks {
        lexical: Some(usize::MAX),
        ..ChannelRanks::default()
    };
    let got = weighted_rrf_score(&max_rank, &ChannelWeights::default());
    assert!(got.is_finite() && got > 0.0 && got < 1.0 / 61.0);
    // All 8 channels at rank 0 with 1e308 weights: 8 clamped terms sum to
    // ~16/61, finite. Epsilon 1e-12 vs multi-term accumulation (kills
    // clamp-removal, which would yield ~1e306, and absent-channel mutants).
    let all = ChannelRanks {
        lexical: Some(0),
        definition: Some(0),
        caller: Some(0),
        graph: Some(0),
        anchor: Some(0),
        semantic: Some(0),
        pattern: Some(0),
        import: Some(0),
    };
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
}

// ---------------------------------------------------------------------------
// 4. apply_weighted_rrf: non-finite input scores are dropped, extremes kept
// ---------------------------------------------------------------------------

#[test]
fn apply_weighted_rrf_drops_nonfinite_keeps_extreme_finite() {
    let weights = ChannelWeights::default();
    // Gate is `is_finite() && > 0.0`: NaN, +-inf, zero, and negatives never
    // enter a channel or the member map (N1 pins 0/-1/NaN; inf is new here).
    let mut bad = vec![
        n2_hit(HitKind::Def, "a.rs", 1, f64::INFINITY),
        n2_hit(HitKind::Def, "b.rs", 2, f64::NEG_INFINITY),
        n2_hit(HitKind::Def, "c.rs", 3, f64::NAN),
        n2_hit(HitKind::Def, "d.rs", 4, 0.0),
        n2_hit(HitKind::Def, "e.rs", 5, -1e308),
    ];
    apply_weighted_rrf(&mut bad, &weights);
    assert!(bad.is_empty());

    // Subnormal and 1e308 are finite and positive -> kept. Fused scores depend
    // only on rank order, so each lone hit in its channel is rank 0 -> 1/61.
    let mut extreme = vec![
        n2_hit(HitKind::Def, "s.rs", 1, 5e-324),
        n2_hit(HitKind::Caller, "l.rs", 2, 1e308),
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
        n2_hit(HitKind::Asgrep, "m.rs", 7, f64::INFINITY),
        n2_hit(HitKind::Def, "m.rs", 7, 0.5),
    ];
    apply_weighted_rrf(&mut mixed, &weights);
    assert_eq!(mixed.len(), 1);
    assert_eq!(mixed[0].score, 1.0 / 61.0);
    assert_eq!(mixed[0].contributors, vec![HitKind::Def]);

    // Hostile weights are sanitized per-channel (test 3 rails): output finite.
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
        n2_hit(HitKind::Def, "a.rs", 1, 3.0),
        n2_hit(HitKind::Caller, "b.rs", 2, 2.0),
    ];
    apply_weighted_rrf(&mut hits, &hostile);
    assert_eq!(hits.len(), 2);
    for hit in &hits {
        assert!(hit.score.is_finite() && hit.score > 0.0);
    }
}

// ---------------------------------------------------------------------------
// 5. route_hits: NaN propagates through clamp; infinities clamp to rails
// ---------------------------------------------------------------------------

#[test]
fn route_hits_hostile_scores_propagate_or_clamp() {
    // f64::clamp is comparison-based: NaN fails both comparisons and passes
    // through UNCHANGED. route_hits therefore propagates NaN (no panic, no
    // silent zeroing); infinities and extremes hit the [0,1] rails.
    let parsed = ParsedQuery::parse("foo");
    let mut hits = vec![
        n2_hit(HitKind::Embed, "nan.rs", 1, f64::NAN),
        n2_hit(HitKind::Embed, "pinf.rs", 2, f64::INFINITY),
        n2_hit(HitKind::Embed, "ninf.rs", 3, f64::NEG_INFINITY),
        n2_hit(HitKind::Embed, "big.rs", 4, 1e308),
        n2_hit(HitKind::Embed, "negbig.rs", 5, -1e308),
        n2_hit(HitKind::Embed, "neg.rs", 6, -5.0),
        n2_hit(HitKind::Embed, "tiny.rs", 7, 1e-308),
        n2_hit(HitKind::Embed, "sub.rs", 8, 5e-324),
    ];
    route_hits(&parsed, &mut hits);
    assert!(hits[0].score.is_nan(), "NaN must propagate, not clamp");
    assert_eq!(hits[1].score, 1.0);
    assert_eq!(hits[2].score, 0.0);
    assert_eq!(hits[3].score, 1.0);
    assert_eq!(hits[4].score, 0.0);
    assert_eq!(hits[5].score, 0.0);
    // 1e-308/4: finite, positive, far below any sane threshold.
    assert!(hits[6].score.is_finite() && hits[6].score > 0.0 && hits[6].score < 1e-300);
    // 5e-324 is 2^-1074; /4 is 2^-1076 = 0.25 ulp -> rounds to +0.0.
    assert_eq!(hits[7].score, 0.0);

    // Propagation is channel-independent: Def (ceiling 13, no symbol ->
    // matched.max(1)=1) and Asgrep ceilings behave the same.
    let mut def = [n2_hit(HitKind::Def, "d.rs", 1, f64::NAN)];
    route_hits(&parsed, &mut def);
    assert!(def[0].score.is_nan());
    let mut asg = [n2_hit(HitKind::Asgrep, "a.rs", 1, f64::INFINITY)];
    route_hits(&parsed, &mut asg);
    assert_eq!(asg[0].score, 1.0);
}

// ---------------------------------------------------------------------------
// 6. learn_fusion_weights: NaN relevance skips pairs; rails sanitize entry
// ---------------------------------------------------------------------------

#[test]
fn learn_fusion_weights_degenerate_relevance_and_initial_weights() {
    // NaN relevance: both `>` comparisons are false -> `continue`, exactly as
    // if the pair were tied. No pairs -> both losses exactly 0.0, weights
    // untouched (unit initial survives entry sanitize bit-identical).
    let nan_rel = vec![FusionExample {
        query: "q".to_string(),
        candidates: vec![
            FusionCandidate {
                id: "a".to_string(),
                relevance: f64::NAN,
                ranks: ChannelRanks {
                    lexical: Some(0),
                    ..ChannelRanks::default()
                },
            },
            FusionCandidate {
                id: "b".to_string(),
                relevance: f64::NAN,
                ranks: ChannelRanks {
                    lexical: Some(1),
                    ..ChannelRanks::default()
                },
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
        lexical: f64::NAN,   // non-finite -> 1.0
        def: f64::INFINITY,  // non-finite -> 1.0
        caller: f64::NEG_INFINITY, // non-finite -> 1.0
        graph: 1e308,        // -> 2.0 rail
        anchor: -1e308,      // -> 0.25 rail
        embed: -0.0,         // finite -> 0.25 rail
        pattern: 5e-324,     // finite tiny -> 0.25 rail
        import: 2.5,         // -> 2.0 rail
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
}

// ---------------------------------------------------------------------------
// 7. analyze_weight_sensitivity: hostile steps sanitize; degenerate tables zero
// ---------------------------------------------------------------------------

#[test]
fn sensitivity_hostile_step_and_degenerate_examples() {
    // Step sanitize: non-finite or non-positive -> 0.1; > 0.5 caps at 0.5.
    // Same sanitized value -> bit-identical tables (same arithmetic replayed).
    let live = n2_pair_example();
    let unit = ChannelWeights::default();
    let via_inf = analyze_weight_sensitivity(&live, &unit, f64::INFINITY);
    let via_nan = analyze_weight_sensitivity(&live, &unit, f64::NAN);
    assert_eq!(via_inf, via_nan, "inf and NaN steps must sanitize identically");
    let via_huge = analyze_weight_sensitivity(&live, &unit, 1e308);
    let via_half = analyze_weight_sensitivity(&live, &unit, 0.5);
    assert_eq!(via_huge, via_half, "1e308 step must cap at 0.5");
    let via_neg = analyze_weight_sensitivity(&live, &unit, -1.0);
    assert_eq!(via_neg, via_nan, "negative step must sanitize to 0.1");

    // Empty examples: loss is identically 0 for ANY sanitized step, so every
    // row is an exact zero and nothing is stiff. Proves hostile steps never
    // reach a division (no div-by-zero -> no NaN gradient/curvature).
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
                ranks: ChannelRanks {
                    lexical: Some(0),
                    ..ChannelRanks::default()
                },
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

// ---------------------------------------------------------------------------
// 8. finish_response: hostile scores retained with zero margin, never dropped
// ---------------------------------------------------------------------------

#[test]
fn finish_hostile_scores_survive_with_zero_margin() {
    let dir = tempfile::tempdir().unwrap();
    let parsed = ParsedQuery::parse("foo"); // single term -> score ordering
    // Embed group: 0.75/0.5 rank normally (margin 0.25 exact), NaN is gated
    // out of margin ranking (is_finite filter) but the HIT is retained.
    // Asgrep +-inf: same gate -> margin 0, retained. Def 1e308 vs Caller
    // 5e-324 share the Structural group: delta rounds back to exactly 1e308.
    let hits = vec![
        n2_hit(HitKind::Embed, "e1.rs", 1, 0.75),
        n2_hit(HitKind::Embed, "e2.rs", 2, 0.5),
        n2_hit(HitKind::Embed, "enan.rs", 3, f64::NAN),
        n2_hit(HitKind::Asgrep, "pinf.rs", 1, f64::INFINITY),
        n2_hit(HitKind::Asgrep, "ninf.rs", 2, f64::NEG_INFINITY),
        n2_hit(HitKind::Def, "big.rs", 1, 1e308),
        n2_hit(HitKind::Caller, "tiny.rs", 2, 5e-324),
    ];
    let response = finish_response(&parsed, &n2_options(dir.path(), 10), hits, false);
    assert_eq!(response.hits.len(), 7, "finish must not drop hostile scores");
    let by_file = |f: &str| response.hits.iter().find(|h| h.file == f).unwrap();
    assert_eq!(by_file("e1.rs").margin, 0.25); // 0.75-0.5 exact in binary
    assert_eq!(by_file("e2.rs").margin, 0.0); // last finite in group
    assert_eq!(by_file("enan.rs").margin, 0.0); // non-finite gated from ranking
    assert!(by_file("enan.rs").score.is_nan()); // ...but the score is kept
    assert_eq!(by_file("pinf.rs").margin, 0.0);
    assert_eq!(by_file("ninf.rs").margin, 0.0);
    assert_eq!(by_file("big.rs").margin, 1e308); // 1e308-5e-324 rounds to 1e308
    assert_eq!(by_file("tiny.rs").margin, 0.0);
    // Confidence never touches the score: always finite and within the cap.
    for hit in &response.hits {
        assert!(
            hit.confidence.is_finite() && (0.0..=0.99).contains(&hit.confidence),
            "{}: confidence = {}",
            hit.file,
            hit.confidence
        );
    }

    // margin_is_decisive totality: `score > 0 && margin >= 0.10 * score`.
    let mut h = n2_hit(HitKind::Def, "a.rs", 1, f64::NAN);
    h.margin = 1.0;
    assert!(!margin_is_decisive(&h)); // NaN > 0 is false
    h.score = f64::INFINITY;
    h.margin = 1e308;
    assert!(!margin_is_decisive(&h)); // 1e308 >= 0.1 * inf = inf is false
    h.margin = f64::INFINITY;
    assert!(margin_is_decisive(&h)); // inf > 0 and inf >= 0.1*inf
    h.score = 1e308;
    h.margin = 1e308;
    assert!(margin_is_decisive(&h)); // 1e308 >= ~1.0000000000000001e307
    // 0.1 * 5e-324 underflows to exactly 0.0, so any positive margin wins
    // while the score>0 gate still holds.
    h.score = 5e-324;
    h.margin = 5e-324;
    assert!(margin_is_decisive(&h));
    h.score = 0.0;
    h.margin = 5e-324;
    assert!(!margin_is_decisive(&h)); // score > 0 gate
}

// ---------------------------------------------------------------------------
// 9. search_flat: hostile vectors gate to empty or normalize, never panic
// ---------------------------------------------------------------------------

#[test]
fn search_flat_hostile_vectors_gate_or_normalize() {
    // Empty build -> brute-force path with the MIN_SIMILARITY (0.08) gate.
    let index = SemanticAnnIndex::build_from_flat(&[], 2);
    let flat = [1.0f32, 0.0, 0.0, 1.0, 1.0, 1.0];
    // Non-finite query components are zeroed before the norm; a fully-zeroed
    // query has norm 0 -> every cosine is 0.0 -> gated -> empty.
    for query in [
        [f32::NAN, 0.0],
        [f32::NAN, f32::NAN],
        [f32::INFINITY, 0.0],
        [f32::NEG_INFINITY, f32::INFINITY],
        [0.0, 0.0],
    ] {
        assert!(
            index.search_flat(&flat, 2, &query, 10).is_empty(),
            "query {query:?} must gate to empty"
        );
    }
    // NaN corpus row: cosine skips non-finite components, leaving nb == 0 ->
    // sim 0.0 -> gated. The good row still ranks first with sim exactly 1.0.
    let nan_flat = [1.0f32, 0.0, f32::NAN, f32::NAN];
    let got = index.search_flat(&nan_flat, 2, &[1.0, 0.0], 10);
    assert_eq!(got, vec![(0, 1.0)]);
    // f32 norm overflow: 1e30^2 overflows f32 -> norm +inf -> zero-fill ->
    // gated to empty (extreme magnitude drops, no panic, no garbage rank).
    assert!(index.search_flat(&flat, 2, &[1e30, 0.0], 10).is_empty());
    // f32 norm underflow: 1e-40^2 underflows to 0 -> norm 0 -> same drop.
    assert!(index.search_flat(&flat, 2, &[1e-40, 0.0], 10).is_empty());
    // Safe extremes normalize exactly: [1e10,0]/1e10 = [1,0]; row [1e-10,0]
    // has f64 cosine 1e-10/(1*1e-10) = 1.0 -> sim exactly 1.0.
    assert_eq!(
        index.search_flat(&flat, 2, &[1e10, 0.0], 10),
        index.search_flat(&flat, 2, &[1.0, 0.0], 10)
    );
    let tiny_row = [1e-10f32, 0.0];
    assert_eq!(
        index.search_flat(&tiny_row, 2, &[1.0, 0.0], 10),
        vec![(0, 1.0)]
    );
    // dim 0 is guarded before any `/ dim` (no integer-div panic); a huge
    // limit is just a cap, not an allocation.
    assert!(index.search_flat(&flat, 0, &[1.0, 0.0], 5).is_empty());
    assert_eq!(index.search_flat(&flat, 2, &[1.0, 0.0], usize::MAX).len(), 2);

    // build_from_flat on all-NaN vectors: rows zero-fill, k-means runs on
    // zeros deterministically, search still gates everything to empty.
    let nan_index = SemanticAnnIndex::build_from_flat(&[f32::NAN; 8], 2);
    assert!(nan_index
        .search_flat(&[f32::NAN; 8], 2, &[1.0, 0.0], 10)
        .is_empty());
}

// ---------------------------------------------------------------------------
// 10. expand_chain: hostile decay propagates to hops, ordering stays total
// ---------------------------------------------------------------------------

fn n2_base<'a>(
    path: &'a str,
    lines: &'a [(u32, String)],
    hash: &'a str,
    symbols: &'a [SymbolRow],
    callers: &'a [CallerRow],
) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path: path,
        language: Some("rust"),
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash: hash,
        lines,
        eol: "\n",
        symbols,
        callers,
        imports: &[],
        pattern_nodes: &[],
        depth_truncated: false,
        semantic_chunks: &[],
        embed_semantic: false,
        embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
    }
}

fn n2_chain_store(dir: &tempfile::TempDir) -> IndexStore {
    let store = IndexStore::open(dir.path(), None).unwrap();
    let caller_symbols = [SymbolRow {
        name: "FooBar".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 24,
    }];
    let callers = [CallerRow {
        caller: "FooBar".into(),
        callee: "Baz".into(),
        line_no: 1,
        byte_start: 14,
        byte_end: 17,
    }];
    store
        .upsert_file(n2_base(
            "caller.rs",
            &[(1, "fn FooBar() { Baz(); }".into())],
            "caller-hash",
            &caller_symbols,
            &callers,
        ))
        .unwrap();
    let callee_symbols = [SymbolRow {
        name: "baz".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 11,
    }];
    store
        .upsert_file(n2_base(
            "callee.rs",
            &[(1, "fn baz() {}".into())],
            "callee-hash",
            &callee_symbols,
            &[],
        ))
        .unwrap();
    store
}

#[test]
fn chain_decay_hostile_propagates_without_panic() {
    let temp = tempfile::tempdir().unwrap();
    let store = n2_chain_store(&temp);
    // hop_score = seed.score * decay, one multiply, no guards: the decay
    // propagates bit-exactly. Ordering uses partial_cmp+Equal fallback plus
    // file/symbol/line tiebreaks, so even NaN scores keep a total order.
    for decay in [f64::NAN, f64::INFINITY, -2.0, -0.0] {
        let response = expand_chain(
            &store,
            "defs:foobar",
            &ChainConfig {
                max_depth: 1,
                top_n: 4,
                limit: 8,
                decay_factor: decay,
            },
        )
        .unwrap();
        assert!(
            response.seeds.iter().any(|n| n.symbol.as_deref() == Some("FooBar")),
            "fixture must seed FooBar (decay {decay})"
        );
        assert!(
            response.seeds.iter().all(|n| n.score.is_finite()),
            "seeds come from the search pipeline: always finite"
        );
        let depth1: Vec<_> = response.nodes.iter().filter(|n| n.depth == 1).collect();
        assert_eq!(depth1.len(), 1, "one hop: FooBar -calls-> baz");
        assert_eq!(depth1[0].file, "callee.rs");
        // Every depth-1 node descends from some seed via the identical op, so
        // its bits must equal one of the per-seed expectations bit-for-bit.
        // (NaN hops: same multiply -> same payload on this machine.)
        let expected: Vec<u64> = response
            .seeds
            .iter()
            .map(|s| (s.score * decay).to_bits())
            .collect();
        for node in &depth1 {
            assert!(
                expected.contains(&node.score.to_bits()),
                "hop score {:x} not in {expected:x?} (decay {decay})",
                node.score.to_bits()
            );
        }
        if decay == -0.0 {
            assert_eq!(depth1[0].score, 0.0); // x * -0.0 == 0 either sign
        }
        if decay.is_nan() {
            assert!(depth1[0].score.is_nan());
        }
    }

    // NaN-decay determinism: same store + same query -> identical node stream
    // (file, symbol, depth, score bits), twice in a row.
    let key = |store: &IndexStore| {
        expand_chain(
            store,
            "defs:foobar",
            &ChainConfig {
                max_depth: 1,
                top_n: 4,
                limit: 8,
                decay_factor: f64::NAN,
            },
        )
        .unwrap()
        .nodes
        .iter()
        .map(|n| {
            (
                n.file.clone(),
                n.symbol.clone(),
                n.depth,
                n.score.to_bits(),
            )
        })
        .collect::<Vec<_>>()
    };
    assert_eq!(key(&store), key(&store));
}

// ---------------------------------------------------------------------------
// 11. score_def / score_caller: extreme term counts stay exact; zero stays 0
// ---------------------------------------------------------------------------

#[test]
fn score_def_caller_extreme_term_counts() {
    // 100k exact-match terms: coverage = 100000 * 5.0. Every partial sum is an
    // integer < 2^53, hence exact; final 500000.0 exact. score_def adds the
    // 2x+base scaling exactly: 2*500000+3 = 1000003.0 (and +1.5 for caller).
    let terms = vec!["foo".to_string(); 100_000];
    assert_eq!(score_def(&terms, "foo"), 1_000_003.0);
    assert_eq!(score_caller(&terms, "foo"), 1_000_001.5);
    // Zero coverage never earns the base (rank-pollution guard): empty terms
    // and sub-floor (1-char) terms both yield exactly 0.0.
    let empty: Vec<String> = vec![];
    assert_eq!(score_def(&empty, "foo"), 0.0);
    assert_eq!(score_caller(&empty, "foo"), 0.0);
    assert_eq!(score_def(&["a".to_string()], "abc"), 0.0);
    assert_eq!(score_caller(&["a".to_string()], "abc"), 0.0);
}

// ---------------------------------------------------------------------------
// 12. IVF candidate selection: NaN query zeroes, huge probes clamp, no panic
// ---------------------------------------------------------------------------

#[test]
fn ivf_candidate_selection_degenerate_inputs() {
    // n=4, dim=2: k = clamp(sqrt(4))=16 -> min(n) = 4 centroids, k-means over
    // normalized rows, deterministic (index-ordered parallel collect).
    let flat = [1.0f32, 0.0, 0.0, 1.0, 1.0, 1.0, -1.0, 0.0];
    let index = SemanticAnnIndex::build_from_flat(&flat, 2);
    assert_eq!(index.centroid_count(), 4);

    // NaN query components zero-fill before centroid dots, so a NaN query is
    // bit-identical to the zero query: total order via stable sort + sorted
    // member emission, every index in range.
    let via_nan = index.candidate_indices(&[f32::NAN, f32::NAN], None);
    let via_zero = index.candidate_indices(&[0.0, 0.0], None);
    assert_eq!(via_nan, via_zero);
    let mut sorted = via_nan.clone();
    sorted.sort_unstable();
    assert_eq!(via_nan, sorted, "members must be emitted sorted");
    assert!(via_nan.iter().all(|&i| i < 4));

    // probes clamps to [1, populated]: usize::MAX and 1000 take the same path
    // and must agree bit-for-bit; search honors the same clamp without panic.
    let q = [1.0f32, 0.0];
    assert_eq!(
        index.candidate_indices(&q, Some(usize::MAX)),
        index.candidate_indices(&q, Some(1000))
    );
    assert_eq!(
        index.search_flat_with_probes(&flat, 2, &q, 10, Some(usize::MAX)),
        index.search_flat_with_probes(&flat, 2, &q, 10, Some(1000))
    );

    // reassign_all fails closed (false, no panic): empty input, dim mismatch
    // against len-2 centroids, and an index with no centroids at all.
    let mut index = index;
    assert!(!index.reassign_all(&[], 2));
    assert!(!index.reassign_all(&flat, 3));
    let mut empty = SemanticAnnIndex::build_from_flat(&[], 2);
    assert!(!empty.reassign_all(&flat, 2));
}
