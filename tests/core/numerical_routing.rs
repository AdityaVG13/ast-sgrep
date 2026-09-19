//! Canonical numerical contracts: intent weights, hit routing, def/caller scoring.
//!
//! Consolidates the N1 (exactness) + N2 (totality) + N3 (metamorphic) legs of
//! `numerical_pass{1,2,3}.rs` for the routing layer into ONE contract test per
//! function, plus the two KEEP standalone tests that own their surface. Each
//! test carries INTENT + KILLS + ABSORBS.
//!
//! Every expectation is HAND-DERIVED (rational or float derivation in the
//! comment above it). Nothing here snapshots production output.

use ast_sgrep_core::fusion::LearnedFusionModel;
use ast_sgrep_core::intent::{default_weights, route_hits, weights_for, ChannelWeights, QueryIntent};
use ast_sgrep_core::query::ParsedQuery;
use ast_sgrep_core::rank::{score_caller, score_def};
use ast_sgrep_core::search::{HitKind, SearchHit};
use ast_sgrep_testkit::{mk_hit, parsed_query};

/// INTENT: `default_weights` per-intent channel-weight table literals.
/// KILLS: table-literal-mutant.
/// ABSORBS: `default_weights_symbol_table_exact` (N1) +
/// `default_weights_conceptual_and_flat_intents_exact` (N1).
#[test]
fn default_weights_contract() {
    // Symbol: lean on def (2.0), keep caller/anchor at 1.0, halve
    // graph/embed/import, mute pattern to the 0.25 rail.
    let w = default_weights(QueryIntent::Symbol);
    assert_eq!(w.lexical, 0.8);
    assert_eq!(w.def, 2.0);
    assert_eq!(w.caller, 1.0);
    assert_eq!(w.graph, 0.7);
    assert_eq!(w.anchor, 1.0);
    assert_eq!(w.embed, 0.7);
    assert_eq!(w.pattern, 0.25);
    assert_eq!(w.import, 0.8);
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

/// INTENT: env weight-spec grammar — override, [0.25,2.0] clamp, all ignore
/// rules; sole toucher of ASGREP_INTENT_WEIGHTS.
/// KILLS: parse/clamp-branch-mutant.
/// ABSORBS: none (KEEP standalone: `weights_for_env_override_clamps_and_filters`).
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

/// INTENT: `route_hits` ceilings (Asgrep/Def/Caller/fixed) + empty-terms
/// zeroing + hostile-score propagation + fixed-ceiling monotonicity.
/// KILLS: ceiling-formula-mutant, denom-branch-mutant, ceiling-constant-mutant,
/// clamp/NaN-zeroing-mutant, divide/clamp-mutant.
/// ABSORBS: `route_hits_asgrep_ceiling_exact` (N1) +
/// `route_hits_def_caller_ceilings_and_empty_terms` (N1) +
/// `route_hits_fixed_channel_ceilings` (N1) +
/// `route_hits_hostile_scores_propagate_or_clamp` (N2) +
/// `route_hits_monotone_within_fixed_ceiling_channel` (N3).
#[test]
fn route_hits_contract() {
    // --- Exactness: Asgrep ceiling = rrf(0,60) * 200 = (1/61)*200.
    // ONE OR-query -> single rank -> ceiling is rrf(0)*200, not terms*. NOTE
    // the ceiling is written with the SAME operations as production
    // ((1/61)*200): (1/61)*200 and 200/61 can differ by 1 ulp, so a 200/61
    // literal would be a wrong oracle here.
    let ceiling = (1.0f64 / 61.0) * 200.0;
    let parsed = ParsedQuery::parse("foo"); // terms ["foo"], no spelling
    assert_eq!(parsed.terms, vec!["foo".to_string()]);
    let mut hits = vec![
        mk_hit(HitKind::Asgrep, "a.rs", 1, ceiling),
        mk_hit(HitKind::Asgrep, "b.rs", 2, ceiling / 2.0),
        mk_hit(HitKind::Asgrep, "c.rs", 3, ceiling * 2.0),
        mk_hit(HitKind::Asgrep, "d.rs", 4, 0.0),
    ];
    route_hits(&parsed, &mut hits);
    assert_eq!(hits[0].score, 1.0); // x/x == 1 exactly (x finite, nonzero)
    assert_eq!(hits[1].score, 0.5); // (c/2)/c: halving is exact, quotient 0.5 exact
    assert_eq!(hits[2].score, 1.0); // 2c/c == 2 -> clamped to 1.0
    assert_eq!(hits[3].score, 0.0);

    // --- Exactness: Def/Caller ceilings = 2*5*denom + base, empty-terms zeroing.
    // Single term ["foo"], symbol "foo": matched=1, no identifier spelling ->
    // denom=1 -> Def ceiling 2*5*1+3 = 13, Caller 2*5*1+1.5 = 11.5.
    let parsed = ParsedQuery::parse("foo");
    let mut def = mk_hit(HitKind::Def, "a.rs", 1, 13.0);
    def.symbol = Some("foo".to_string());
    let mut caller = mk_hit(HitKind::Caller, "b.rs", 2, 11.5);
    caller.callee = Some("foo".to_string());
    // Multi-term ["foo","bar"] vs "foo bar": both terms substring-match ->
    // matched=2 -> ceiling 2*5*2+3 = 23. Score 11.5 -> exactly 0.5.
    let multi = parsed_query("foo bar", &["foo", "bar"]);
    let mut def2 = mk_hit(HitKind::Def, "c.rs", 3, 11.5);
    def2.symbol = Some("foo bar".to_string());
    // Identifier-spelling branch: raw "Foo bar" has uppercase -> spelling
    // Some("Foo") -> denom = max(terms.len()=2, matched=1) = 2 -> ceiling 23.
    // (Without the spelling branch the ceiling would be 13 and 11.5/13 != 0.5,
    // so this kills the branch mutant.)
    let spelled = parsed_query("Foo bar", &["foo", "bar"]);
    assert_eq!(spelled.identifier_spelling(), Some("Foo"));
    let mut def3 = mk_hit(HitKind::Def, "d.rs", 4, 11.5);
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
        mk_hit(HitKind::Asgrep, "a.rs", 1, 99.0),
        mk_hit(HitKind::Def, "b.rs", 2, 99.0),
        mk_hit(HitKind::Graph, "c.rs", 3, 99.0),
        mk_hit(HitKind::Embed, "d.rs", 4, 2.0),
    ];
    z[1].symbol = Some("foo".to_string());
    route_hits(&empty, &mut z);
    assert_eq!(z[0].score, 0.0);
    assert_eq!(z[1].score, 0.0);
    assert_eq!(z[2].score, 0.0);
    assert_eq!(z[3].score, 0.5);

    // --- Exactness: fixed ceilings Graph 5 / Anchor 6 / Embed 4 / Pattern 7 / Import 2.
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
        let mut full = [mk_hit(kind, "a.rs", 1, ceiling)];
        route_hits(&parsed, &mut full);
        assert_eq!(full[0].score, 1.0, "{kind:?} ceiling must map to 1.0");
        let mut half = [mk_hit(kind, "a.rs", 1, ceiling / 2.0)];
        route_hits(&parsed, &mut half);
        assert_eq!(half[0].score, 0.5, "{kind:?} half-ceiling must map to 0.5");
    }

    // --- Totality: f64::clamp is comparison-based: NaN fails both comparisons
    // and passes through UNCHANGED. route_hits therefore propagates NaN (no
    // panic, no silent zeroing); infinities and extremes hit the [0,1] rails.
    let parsed = ParsedQuery::parse("foo");
    let mut hits = vec![
        mk_hit(HitKind::Embed, "nan.rs", 1, f64::NAN),
        mk_hit(HitKind::Embed, "pinf.rs", 2, f64::INFINITY),
        mk_hit(HitKind::Embed, "ninf.rs", 3, f64::NEG_INFINITY),
        mk_hit(HitKind::Embed, "big.rs", 4, 1e308),
        mk_hit(HitKind::Embed, "negbig.rs", 5, -1e308),
        mk_hit(HitKind::Embed, "neg.rs", 6, -5.0),
        mk_hit(HitKind::Embed, "tiny.rs", 7, 1e-308),
        mk_hit(HitKind::Embed, "sub.rs", 8, 5e-324),
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
    let mut def = [mk_hit(HitKind::Def, "d.rs", 1, f64::NAN)];
    route_hits(&parsed, &mut def);
    assert!(def[0].score.is_nan());
    let mut asg = [mk_hit(HitKind::Asgrep, "a.rs", 1, f64::INFINITY)];
    route_hits(&parsed, &mut asg);
    assert_eq!(asg[0].score, 1.0);

    // --- Metamorphic: Embed divides by the fixed ceiling 4.0 and clamps to
    // [0, 1]: x/4 is strictly increasing on finite positives and clamping only
    // collapses values (never inverts), so ascending inputs stay
    // non-decreasing. (This relation holds ONLY for fixed-ceiling channels:
    // Def/Caller ceilings vary per hit via the symbol match and can
    // legitimately reorder.)
    let parsed = ParsedQuery::parse("foo");
    let mut hits: Vec<SearchHit> = [0.3, 1.1, 2.7, 4.0, 9.0]
        .iter()
        .enumerate()
        .map(|(i, s)| mk_hit(HitKind::Embed, "m.rs", i as u32, *s))
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

/// INTENT: `score_def`/`score_caller` extreme-term-count exactness + zero-guard
/// + relevance-ladder relations (producer-score contract).
/// KILLS: coverage/base-guard-mutant, ladder/scale-mutant.
/// ABSORBS: `score_def_caller_extreme_term_counts` (N2) +
/// `def_caller_relevance_ladder_relations` (N3).
#[test]
fn score_def_caller_contract() {
    // --- Totality: 100k exact-match terms: coverage = 100000 * 5.0. Every
    // partial sum is an integer < 2^53, hence exact; final 500000.0 exact.
    // score_def adds the 2x+base scaling exactly: 2*500000+3 = 1000003.0
    // (and +1.5 for caller).
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

    // --- Metamorphic: same coverage feeds both: def = 2c + 3, caller = 2c +
    // 1.5, so def is strictly above caller exactly when coverage is positive,
    // and the two agree exactly when nothing matches (no base awarded either
    // way).
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

/// INTENT: `intent_weight_spec` 6-decimal rendering.
/// KILLS: format/order-mutant.
/// ABSORBS: none (KEEP standalone: `intent_weight_spec_format_exact`).
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
