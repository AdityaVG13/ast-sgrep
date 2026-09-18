//! N1 numerical-exactness oracles: the REST of the ast-sgrep-core float surface.
//!
//! L1/L2 (`oracle_foundry_pass2/3`) already pin `rrf_score`, `fuse_rrf`,
//! `score_lexical_rrf`, the `score_symbol` ladder, best/coverage split,
//! `weighted_rrf_score`, and `score_def`/`score_caller`. This suite pins
//! everything else that does scoring/weight/threshold math reachable from
//! the public API:
//!
//! * `intent::default_weights` / `weights_for` (per-intent weight tables,
//!   env-spec parsing + clamp to [0.25, 2.0])
//! * `intent::route_hits` (channel ceilings + [0, 1] clamp + empty-terms zeroing)
//! * `fusion::apply_weighted_rrf` (per-channel ranks -> fused sums, canonical
//!   member, zero-score drop)
//! * `fusion::learn_fusion_weights` (pairwise softplus loss, coordinate search
//!   to the clamp rail) and `analyze_weight_sensitivity` (finite-difference
//!   gradient/curvature/churn, stiff flags)
//! * `LearnedFusionModel::intent_weight_spec` (6-decimal rendering)
//! * `semantic_ann::{ann_threshold, should_use_ann, ann_result_is_sufficient}`
//!   (threshold boundaries) and `SemanticAnnIndex::search_flat` (cosine
//!   ranking + MIN_SIMILARITY gate on the brute-force path)
//! * `finish_response` margins/confidence + `margin_is_decisive` boundary
//!
//! Every expectation is HAND-DERIVED (rational or float derivation in the
//! comment above it). Nothing here snapshots production output. Where IEEE
//! rounding makes bit-equality unsafe the test states epsilon + why.
//!
//! Deliberately NOT covered (unreachable from integration tests without an
//! e2e store fixture): `search::critic` multipliers (`pub(crate)`),
//! `search::field_weight` mixing (`pub(crate)`), `chain` decay (`score *
//! decay` inside private `expand_one`; only observable via `expand_chain`
//! on a born store), literal/regex `1/(1+0.01*rank)` decay and the anchor
//! `sqrt` (private inline math, observable only through full search).

use ast_sgrep_core::fusion::{
    analyze_weight_sensitivity, apply_weighted_rrf, learn_fusion_weights, ChannelRanks,
    FusionCandidate, FusionChannel, FusionExample, LearnedFusionModel,
};
use ast_sgrep_core::intent::{default_weights, route_hits, weights_for, ChannelWeights, QueryIntent};
use ast_sgrep_core::query::{ParsedQuery, QueryMode};
use ast_sgrep_core::search::{
    finish_response, margin_is_decisive, HitKind, SearchHit, SearchOptions, SpanHitInput,
};
use ast_sgrep_core::semantic_ann::{
    ann_result_is_sufficient, ann_threshold, should_use_ann, SemanticAnnIndex,
    DEFAULT_ADAPTIVE_PROBE_PERCENT, DEFAULT_ANN_THRESHOLD,
};

fn n1_hit(kind: HitKind, file: &str, line: u32, score: f64) -> SearchHit {
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

fn n1_parsed(raw: &str, terms: &[&str]) -> ParsedQuery {
    ParsedQuery {
        raw: raw.to_string(),
        mode: QueryMode::Hybrid,
        target: None,
        terms: terms.iter().map(|t| t.to_string()).collect(),
        path_scope: None,
        path_scope_error: None,
        path_scope_exact: false,
    }
}

fn n1_options(root: &std::path::Path, limit: usize) -> SearchOptions {
    SearchOptions {
        root: root.to_path_buf(),
        limit,
        file_filter: None,
        count_only: false,
        use_rerank: false,
        ..SearchOptions::default()
    }
}

// ---------------------------------------------------------------------------
// 1-2. Per-intent channel-weight tables (exact decimal literals)
// ---------------------------------------------------------------------------

#[test]
fn default_weights_symbol_table_exact() {
    // Contract from intent.rs: symbol queries lean on def (2.0), keep caller /
    // anchor at 1.0, halve graph/embed/import, mute pattern to the 0.25 rail.
    let w = default_weights(QueryIntent::Symbol);
    assert_eq!(w.lexical, 0.8);
    assert_eq!(w.def, 2.0);
    assert_eq!(w.caller, 1.0);
    assert_eq!(w.graph, 0.7);
    assert_eq!(w.anchor, 1.0);
    assert_eq!(w.embed, 0.7);
    assert_eq!(w.pattern, 0.25);
    assert_eq!(w.import, 0.8);
}

#[test]
fn default_weights_conceptual_and_flat_intents_exact() {
    // Conceptual: embed leads (1.45), def close (1.35); graph/pattern muted.
    let w = default_weights(QueryIntent::Conceptual);
    assert_eq!(w.lexical, 0.75);
    assert_eq!(w.def, 1.35);
    assert_eq!(w.caller, 0.45);
    assert_eq!(w.graph, 0.25);
    assert_eq!(w.anchor, 0.7);
    assert_eq!(w.embed, 1.45);
    assert_eq!(w.pattern, 0.25);
    assert_eq!(w.import, 0.5);
    // Literal + Structural are the identity: every channel 1.0.
    for intent in [QueryIntent::Literal, QueryIntent::Structural] {
        let w = default_weights(intent);
        for v in [
            w.lexical,
            w.def,
            w.caller,
            w.graph,
            w.anchor,
            w.embed,
            w.pattern,
            w.import,
        ] {
            assert_eq!(v, 1.0, "flat intent {intent:?} must be all-ones");
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Env weight-spec parsing: override, clamp, and every ignore rule
// ---------------------------------------------------------------------------

#[test]
fn weights_for_env_override_clamps_and_filters() {
    // SOLE test touching ASGREP_INTENT_WEIGHTS (parallel-safe: no other test
    // in this binary reads it). Sequence: passthrough -> override -> restore.
    let key = "ASGREP_INTENT_WEIGHTS";
    std::env::remove_var(key);
    // No env: weights_for is the identity over default_weights.
    for intent in [
        QueryIntent::Literal,
        QueryIntent::Symbol,
        QueryIntent::Structural,
        QueryIntent::Conceptual,
    ] {
        assert_eq!(weights_for(intent), default_weights(intent));
    }

    // Spec grammar: "class:k=v,k=v;class:...". Class must equal the intent
    // string; pairs need k=v with finite f64; values clamp to [0.25, 2.0].
    std::env::set_var(
        key,
        "symbol:lexical=1.5,def=9.9,caller=-3.0,graph=NaN,embed=inf,bogus=1.5,malformed;\
         literal:lexical=0.25;\
         nospechere",
    );
    let s = weights_for(QueryIntent::Symbol);
    assert_eq!(s.lexical, 1.5); // clean override
    assert_eq!(s.def, 2.0); // 9.9 clamps to the 2.0 rail
    assert_eq!(s.caller, 0.25); // -3.0 clamps to the 0.25 rail
    assert_eq!(s.graph, 0.7); // NaN is non-finite -> ignored, default kept
    assert_eq!(s.embed, 0.7); // inf is non-finite -> ignored
    assert_eq!(s.anchor, 1.0); // unknown channel + malformed pair ignored
    assert_eq!(s.pattern, 0.25);
    assert_eq!(s.import, 0.8);
    // A class spec only fires for its own intent.
    let l = weights_for(QueryIntent::Literal);
    assert_eq!(l.lexical, 0.25);
    assert_eq!(l.def, 1.0);
    assert_eq!(
        weights_for(QueryIntent::Conceptual),
        default_weights(QueryIntent::Conceptual)
    );
    std::env::remove_var(key);
}

// ---------------------------------------------------------------------------
// 4. route_hits: Asgrep ceiling = rrf(0,60) * 200 = (1/61)*200
// ---------------------------------------------------------------------------

#[test]
fn route_hits_asgrep_ceiling_exact() {
    // e2hc.14(a): ONE OR-query -> single rank -> ceiling is rrf(0)*200, not
    // terms*. NOTE the ceiling is written with the SAME operations as
    // production ((1/61)*200): (1/61)*200 and 200/61 can differ by 1 ulp, so
    // a 200/61 literal would be a wrong oracle here.
    let ceiling = (1.0f64 / 61.0) * 200.0;
    let parsed = ParsedQuery::parse("foo"); // terms ["foo"], no spelling
    assert_eq!(parsed.terms, vec!["foo".to_string()]);

    let mut hits = vec![
        n1_hit(HitKind::Asgrep, "a.rs", 1, ceiling),
        n1_hit(HitKind::Asgrep, "b.rs", 2, ceiling / 2.0),
        n1_hit(HitKind::Asgrep, "c.rs", 3, ceiling * 2.0),
        n1_hit(HitKind::Asgrep, "d.rs", 4, 0.0),
    ];
    route_hits(&parsed, &mut hits);
    assert_eq!(hits[0].score, 1.0); // x/x == 1 exactly (x finite, nonzero)
    assert_eq!(hits[1].score, 0.5); // (c/2)/c: halving is exact, quotient 0.5 exact
    assert_eq!(hits[2].score, 1.0); // 2c/c == 2 -> clamped to 1.0
    assert_eq!(hits[3].score, 0.0);
}

// ---------------------------------------------------------------------------
// 5. route_hits: Def/Caller ceilings = 2*5*denom + base, empty-terms zeroing
// ---------------------------------------------------------------------------

#[test]
fn route_hits_def_caller_ceilings_and_empty_terms() {
    // Single term ["foo"], symbol "foo": matched=1, no identifier spelling ->
    // denom=1 -> Def ceiling 2*5*1+3 = 13, Caller 2*5*1+1.5 = 11.5.
    let parsed = ParsedQuery::parse("foo");
    let mut def = n1_hit(HitKind::Def, "a.rs", 1, 13.0);
    def.symbol = Some("foo".to_string());
    let mut caller = n1_hit(HitKind::Caller, "b.rs", 2, 11.5);
    caller.callee = Some("foo".to_string());
    // Multi-term ["foo","bar"] vs "foo bar": both terms substring-match ->
    // matched=2 -> ceiling 2*5*2+3 = 23. Score 11.5 -> exactly 0.5.
    let multi = n1_parsed("foo bar", &["foo", "bar"]);
    let mut def2 = n1_hit(HitKind::Def, "c.rs", 3, 11.5);
    def2.symbol = Some("foo bar".to_string());
    // Identifier-spelling branch: raw "Foo bar" has uppercase -> spelling
    // Some("Foo") -> denom = max(terms.len()=2, matched=1) = 2 -> ceiling 23.
    // (Without the spelling branch the ceiling would be 13 and 11.5/13 != 0.5,
    // so this kills the branch mutant.)
    let spelled = n1_parsed("Foo bar", &["foo", "bar"]);
    assert_eq!(spelled.identifier_spelling(), Some("Foo"));
    let mut def3 = n1_hit(HitKind::Def, "d.rs", 4, 11.5);
    def3.symbol = Some("foo".to_string());

    let mut pair = [def, caller];
    route_hits(&parsed, &mut pair);
    assert_eq!(pair[0].score, 1.0); // 13/13
    assert_eq!(pair[1].score, 1.0); // 11.5/11.5
    let mut m = [def2];
    route_hits(&multi, &mut m);
    assert_eq!(m[0].score, 0.5); // 11.5/23
    let mut s = [def3];
    route_hits(&spelled, &mut s);
    assert_eq!(s[0].score, 0.5); // 11.5/23 via the spelling denom

    // Empty terms: text channels (asgrep/def/caller/graph/anchor) zero out;
    // non-text channels still normalize (embed 2.0/4 = 0.5).
    let empty = ParsedQuery::parse("");
    assert!(empty.terms.is_empty());
    let mut z = [
        n1_hit(HitKind::Asgrep, "a.rs", 1, 99.0),
        n1_hit(HitKind::Def, "b.rs", 2, 99.0),
        n1_hit(HitKind::Graph, "c.rs", 3, 99.0),
        n1_hit(HitKind::Embed, "d.rs", 4, 2.0),
    ];
    z[1].symbol = Some("foo".to_string());
    route_hits(&empty, &mut z);
    assert_eq!(z[0].score, 0.0);
    assert_eq!(z[1].score, 0.0);
    assert_eq!(z[2].score, 0.0);
    assert_eq!(z[3].score, 0.5);
}

// ---------------------------------------------------------------------------
// 6. route_hits: fixed ceilings Graph 5 / Anchor 6 / Embed 4 / Pattern 7 / Import 2
// ---------------------------------------------------------------------------

#[test]
fn route_hits_fixed_channel_ceilings() {
    let parsed = ParsedQuery::parse("foo");
    // (kind, ceiling); half-ceiling normalizes to exactly 0.5 in binary.
    let table = [
        (HitKind::Graph, 5.0),
        (HitKind::Anchor, 6.0),
        (HitKind::Embed, 4.0),
        (HitKind::Pattern, 7.0),
        (HitKind::Import, 2.0),
    ];
    for (kind, ceiling) in table {
        let mut full = [n1_hit(kind, "a.rs", 1, ceiling)];
        route_hits(&parsed, &mut full);
        assert_eq!(full[0].score, 1.0, "{kind:?} ceiling must map to 1.0");
        let mut half = [n1_hit(kind, "a.rs", 1, ceiling / 2.0)];
        route_hits(&parsed, &mut half);
        assert_eq!(half[0].score, 0.5, "{kind:?} half-ceiling must map to 0.5");
    }
}

// ---------------------------------------------------------------------------
// 7-8. apply_weighted_rrf: fused sums, canonical member, rank order, drop rules
// ---------------------------------------------------------------------------

#[test]
fn apply_weighted_rrf_single_and_merge_exact() {
    let weights = ChannelWeights::default(); // all 1.0 -> weight is identity
    // Single hit: the only channel rank is 0 -> fused = 1*1/(60+0+1) = 1/61.
    let mut single = vec![n1_hit(HitKind::Asgrep, "a.rs", 1, 3.0)];
    apply_weighted_rrf(&mut single, &weights);
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].score, 1.0 / 61.0);

    // Same (file, line), two kinds: each channel's rank is 0, fused is the
    // SUM 1/61 + 1/61 (written as the literal sum: 2/61 can differ by 1 ulp
    // because doubling crosses a binade boundary vs. one division).
    // Canonical member: Def priority 0 beats Caller priority 1.
    let mut merged = vec![
        n1_hit(HitKind::Caller, "m.rs", 7, 8.0),
        n1_hit(HitKind::Def, "m.rs", 7, 5.0),
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
}

#[test]
fn apply_weighted_rrf_rank_order_and_zero_drop() {
    let weights = ChannelWeights::default();
    // One channel, two result keys: within-channel rank = score-desc order,
    // so a.rs (9.0) is rank 0 -> 1/61, b.rs (8.0) is rank 1 -> 1/62.
    // Emission order is sorted (file, line): [a.rs, b.rs].
    let mut hits = vec![
        n1_hit(HitKind::Def, "b.rs", 1, 8.0),
        n1_hit(HitKind::Def, "a.rs", 1, 9.0),
    ];
    apply_weighted_rrf(&mut hits, &weights);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].file, "a.rs");
    assert_eq!(hits[0].score, 1.0 / 61.0);
    assert_eq!(hits[1].file, "b.rs");
    assert_eq!(hits[1].score, 1.0 / 62.0);

    // Non-positive or non-finite scores never enter a channel or the member
    // map, so they vanish from the fused output entirely.
    let mut bad = vec![
        n1_hit(HitKind::Def, "a.rs", 1, 0.0),
        n1_hit(HitKind::Def, "b.rs", 2, -1.0),
        n1_hit(HitKind::Def, "c.rs", 3, f64::NAN),
    ];
    apply_weighted_rrf(&mut bad, &weights);
    assert!(bad.is_empty());
    // Empty input is a no-op, not a panic.
    let mut empty: Vec<SearchHit> = vec![];
    apply_weighted_rrf(&mut empty, &weights);
    assert!(empty.is_empty());
}

// ---------------------------------------------------------------------------
// 9. learn_fusion_weights: hand-computed softplus loss + rail convergence
// ---------------------------------------------------------------------------

fn n1_pair_example() -> Vec<FusionExample> {
    // Worse candidate FIRST: pins the (better, worse) relevance swap.
    // better ranks {lexical: 0} -> s = 1/61; worse {lexical: 1} -> 1/62.
    // delta = (1/61 - 1/62)*100 = (1/3782)*100 = 50/1891 > 0
    // loss = ln(1 + e^-delta) (single pair -> mean over 1).
    let worse = FusionCandidate {
        id: "worse".to_string(),
        relevance: 0.0,
        ranks: ChannelRanks {
            lexical: Some(1),
            ..ChannelRanks::default()
        },
    };
    let better = FusionCandidate {
        id: "better".to_string(),
        relevance: 1.0,
        ranks: ChannelRanks {
            lexical: Some(0),
            ..ChannelRanks::default()
        },
    };
    vec![FusionExample {
        query: "q".to_string(),
        candidates: vec![worse, better],
    }]
}

#[test]
fn learn_fusion_weights_single_pair_exact() {
    // Independent oracle (python3 math.log1p(math.exp(-50/1891))):
    // loss_before = 0.680014050821341.
    // Epsilon 1e-12: production computes delta as (1/61 - 1/62)*100 (a few
    // ulps from 50/1891) and exp/log1p are libm (<=1 ulp each); 1e-12 swamps
    // that while still killing formula mutants (missing *100 shifts loss by
    // ~0.013, wrong branch by orders more).
    let examples = n1_pair_example();
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
}

// ---------------------------------------------------------------------------
// 10. Sensitivity: empty-table zeros + live gradient/curvature/stiff ladder
// ---------------------------------------------------------------------------

#[test]
fn sensitivity_empty_and_absent_channels() {
    // Empty examples: every loss is 0 -> central differences are (0-0)/2h =
    // 0, curvature max(0,0) = 0, churn 0 -> stiff false (max_curvature = 0).
    // One row per channel in FusionChannel::ALL order.
    let rows = analyze_weight_sensitivity(&[], &ChannelWeights::default(), 0.1);
    assert_eq!(rows.len(), 8);
    for (row, channel) in rows.iter().zip(FusionChannel::ALL) {
        assert_eq!(row.channel, channel);
        assert_eq!(row.gradient, 0.0);
        assert_eq!(row.curvature, 0.0);
        assert_eq!(row.rank_churn, 0.0);
        assert!(!row.stiff);
    }
    // Non-finite step sanitizes to 0.1 -> identical table.
    let rows_nan = analyze_weight_sensitivity(&[], &ChannelWeights::default(), f64::NAN);
    assert_eq!(rows_nan, rows);

    // Live pair: raising the lexical weight widens delta and lowers the
    // softplus loss -> gradient < 0; softplus is strictly convex ->
    // curvature ~1.7e-4 > 0 (second difference >> ulp(0.68)); the pair order
    // never flips -> churn 0; lexical owns max curvature -> stiff.
    let live = analyze_weight_sensitivity(&n1_pair_example(), &ChannelWeights::default(), 0.1);
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
}

// ---------------------------------------------------------------------------
// 11. intent_weight_spec: exact 6-decimal rendering
// ---------------------------------------------------------------------------

#[test]
fn intent_weight_spec_format_exact() {
    let model = LearnedFusionModel {
        weights: ChannelWeights {
            lexical: 1.0,
            def: 2.0,
            caller: 0.25,
            graph: 0.7,
            anchor: 1.0,
            embed: 0.7,
            pattern: 0.25,
            import: 0.8,
        },
        loss_before: 0.0,
        loss_after: 0.0,
        sensitivity: vec![],
    };
    assert_eq!(
        model.intent_weight_spec("symbol"),
        "symbol:lexical=1.000000,def=2.000000,caller=0.250000,graph=0.700000,\
         anchor=1.000000,embed=0.700000,pattern=0.250000,import=0.800000"
    );
}

// ---------------------------------------------------------------------------
// 12. ANN thresholds: default, override, env, sufficiency truth table
// ---------------------------------------------------------------------------

#[test]
fn ann_threshold_boundary_table() {
    // SOLE test touching ASGREP_ANN_THRESHOLD (parallel-safe).
    let key = "ASGREP_ANN_THRESHOLD";
    std::env::remove_var(key);
    assert_eq!(DEFAULT_ANN_THRESHOLD, 2000);
    assert_eq!(DEFAULT_ADAPTIVE_PROBE_PERCENT, 90);
    assert_eq!(ann_threshold(None), 2000);
    assert_eq!(ann_threshold(Some(5)), 5); // explicit override wins
    // should_use_ann is `count >= threshold` (inclusive lower edge).
    assert!(!should_use_ann(1999, None));
    assert!(should_use_ann(2000, None));
    assert!(!should_use_ann(9, Some(10)));
    assert!(should_use_ann(10, Some(10)));
    // Env supplies the default; override still wins; garbage falls back.
    std::env::set_var(key, "50");
    assert_eq!(ann_threshold(None), 50);
    assert_eq!(ann_threshold(Some(7)), 7);
    std::env::set_var(key, "bogus");
    assert_eq!(ann_threshold(None), 2000);
    std::env::remove_var(key);
    // ann_result_is_sufficient: found >= min(limit, total).
    assert!(ann_result_is_sufficient(5, 10, 5)); // 5 >= min(5,10)=5
    assert!(!ann_result_is_sufficient(4, 10, 5)); // 4 < 5
    assert!(ann_result_is_sufficient(10, 3, 5)); // 10 >= min(5,3)=3
    assert!(!ann_result_is_sufficient(2, 3, 5)); // 2 < 3
    assert!(ann_result_is_sufficient(0, 0, 0)); // 0 >= 0
    assert!(!ann_result_is_sufficient(0, 5, 3)); // empty never sufficient
}

// ---------------------------------------------------------------------------
// 13. search_flat on the brute-force path: cosine ranking + similarity gate
// ---------------------------------------------------------------------------

#[test]
fn search_flat_brute_force_cosine_oracle() {
    // Empty build -> no centroids -> search_flat takes brute_force_flat:
    // cosine(query, row) gated by exceeds_threshold(sim, 0.08), top-k sorted.
    let index = SemanticAnnIndex::build_from_flat(&[], 2);
    assert_eq!(index.centroid_count(), 0);
    assert!(index.candidate_indices(&[1.0, 0.0], None).is_empty());

    // Rows: [1,0] -> cos 1.0; [0,1] -> cos 0.0 (gated: 0 is not > 0.08+1ulp);
    // [1,1] -> 1/sqrt(2) ~= 0.70710678.
    let flat = [1.0f32, 0.0, 0.0, 1.0, 1.0, 1.0];
    let got = index.search_flat(&flat, 2, &[1.0, 0.0], 10);
    assert_eq!(got.len(), 2, "orthogonal row must be gated out: {got:?}");
    assert_eq!(got[0], (0, 1.0)); // 1/(1*1) exactly
    assert_eq!(got[1].0, 2);
    // Epsilon 1e-6: production casts an f64 1/sqrt(2) to f32 (possible 1-ulp
    // double-rounding vs f32-native); 1e-6 still kills gate/order mutants.
    assert!(
        (got[1].1 - 0.70710677f32).abs() < 1e-6,
        "cos = {}, want 1/sqrt(2)",
        got[1].1
    );
    // Limit truncates after ranking; limit 0 and empty corpora are empty.
    assert_eq!(index.search_flat(&flat, 2, &[1.0, 0.0], 1), vec![(0, 1.0)]);
    assert!(index.search_flat(&flat, 2, &[1.0, 0.0], 0).is_empty());
    assert!(index.search_flat(&[], 2, &[1.0, 0.0], 5).is_empty());
    // Query normalization pin: [3,0]/|..| = [1,0] exactly -> bit-identical rows.
    assert_eq!(
        index.search_flat(&flat, 2, &[3.0, 0.0], 10),
        index.search_flat(&flat, 2, &[1.0, 0.0], 10)
    );
}

// ---------------------------------------------------------------------------
// 14. finish_response margins/confidence + margin_is_decisive boundary
// ---------------------------------------------------------------------------

#[test]
fn finish_margins_confidence_and_decisive_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let parsed = ParsedQuery::parse("foo"); // single term -> score ordering
    // Exact-signal ladder: 0.75 -> margin 0.75-0.5 = 0.25; the 0.5 tie ->
    // both 0.0 (tie rule); 0.125 -> 0.125-0.0625 = 0.0625; last -> 0.0.
    // (All operands exact in binary, so all margins below are bit-exact.)
    let mut x1 = n1_hit(HitKind::Asgrep, "f.rs", 1, 0.125);
    x1.contributors = vec![
        HitKind::Def,
        HitKind::Caller,
        HitKind::Graph,
        HitKind::Anchor,
        HitKind::Embed,
    ];
    let mut y1 = n1_hit(HitKind::Asgrep, "g.rs", 1, 0.0625);
    y1.contributors = vec![HitKind::Def, HitKind::Caller, HitKind::Graph, HitKind::Anchor];
    let mut d1 = n1_hit(HitKind::Def, "d.rs", 1, 1.0);
    d1.contributors = vec![HitKind::Caller, HitKind::Embed];
    let hits = vec![
        n1_hit(HitKind::Asgrep, "a.rs", 1, 0.75),
        n1_hit(HitKind::Asgrep, "b.rs", 2, 0.5),
        n1_hit(HitKind::Asgrep, "c.rs", 3, 0.5),
        d1,
        n1_hit(HitKind::Caller, "e.rs", 4, 0.5),
        x1,
        y1,
    ];
    let response = finish_response(&parsed, &n1_options(dir.path(), 10), hits, false);
    assert_eq!(response.hits.len(), 7);
    let by_file = |f: &str| response.hits.iter().find(|h| h.file == f).unwrap();

    assert_eq!(by_file("a.rs").margin, 0.25);
    assert_eq!(by_file("b.rs").margin, 0.0);
    assert_eq!(by_file("c.rs").margin, 0.0);
    assert_eq!(by_file("f.rs").margin, 0.0625);
    assert_eq!(by_file("g.rs").margin, 0.0);
    // Structural ladder (Def 1.0, Caller 0.5): margins 0.5 and 0.0.
    assert_eq!(by_file("d.rs").margin, 0.5);
    assert_eq!(by_file("e.rs").margin, 0.0);

    // Confidence = base(strongest signal) + min(n_contrib-1,3)*0.08, <= 0.99.
    // Bases: Exact 0.75, Structural 0.60, Semantic 0.35.
    assert_eq!(by_file("a.rs").confidence, 0.75); // Exact, no contributors
    assert_eq!(by_file("e.rs").confidence, 0.60); // Structural, none
    // d.rs: strongest Structural, 2 contributors -> 0.60 + 1*0.08 ~= 0.68.
    // Epsilon 1e-12: 0.6 and 0.08 are inexact in binary; the sum is within
    // 1 ulp of the 0.68 literal either way.
    assert!((by_file("d.rs").confidence - 0.68).abs() < 1e-12);
    // f.rs: Exact base + min(5-1,3)=3 steps: 0.75 + 3*0.08 == 0.99 exactly
    // in IEEE double (verified), then clamped to exactly 0.99.
    assert_eq!(by_file("f.rs").confidence, 0.99);
    // g.rs: min(4-1,3)=3, same agreement -> saturation pin, bit-identical.
    assert_eq!(by_file("g.rs").confidence, 0.99);

    // margin_is_decisive: score > 0 AND margin >= 0.10 * score.
    let mut h = n1_hit(HitKind::Def, "a.rs", 1, 1.0);
    h.margin = 0.1;
    assert!(margin_is_decisive(&h)); // 0.1 >= 0.10*1.0, inclusive edge
    h.margin = 0.0999999999;
    assert!(!margin_is_decisive(&h)); // just below the 10% line
    h.score = 2.0;
    h.margin = 0.2;
    assert!(margin_is_decisive(&h)); // ratio scales: 0.2 >= 0.2
    h.score = 0.0;
    h.margin = 0.0;
    assert!(!margin_is_decisive(&h)); // score > 0 gate
    h.score = -1.0;
    h.margin = 5.0;
    assert!(!margin_is_decisive(&h)); // negative scores never decisive
}
