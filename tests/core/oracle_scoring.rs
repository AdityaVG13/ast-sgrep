//! Core scoring/fusion oracles (consolidated).
//!
//! Consolidates the CAT=scoring and CAT=fusion facets of
//! `oracle_foundry_pass{2,3}` (RRF, fusion, symbol ladders, signal/dedup,
//! intent classify/weights/routing) into 4 intent-grouped tests. Expectations
//! are hand-computed; failures assert discriminants, never message text.

use ast_sgrep_core::fusion::{weighted_rrf_score, ChannelRanks, FusionChannel};
use ast_sgrep_core::intent::{
    classify, default_weights, route_hits, weights_for, ChannelWeights, QueryIntent,
};
use ast_sgrep_core::rank::{
    best_symbol_score, best_symbol_score_normalized, coverage_symbol_score,
    coverage_symbol_score_normalized, fuse_rrf, normalize_query_terms, rrf_score,
    score_caller, score_caller_normalized, score_def, score_def_normalized, score_lexical_rrf,
    score_symbol, LEXICAL_RRF_SCALE, RRF_K,
};
use ast_sgrep_core::search::{dedup_hits, SpanHitInput};
use ast_sgrep_core::{
    format_hit_line, hit_why, CriticNote, HitKind, HitSignal, ParsedQuery, SearchHit,
};
use ast_sgrep_testkit::{hit_key_bits, mk_hit, sorted_contributors, unit_channel_weights};

/// INTENT: RRF is exactly 1/(k+r+1) (value table, strict decrease, and the
/// cross-multiplication identity); fusion sums RRF terms (empty→0); lexical
/// applies the 200x scale; weighted RRF skips absent channels and clamps
/// huge/NaN weights.
/// KILLS: `+1`-drop, rank-ignored, k-ignored, `+2`/const-return, sum→max,
/// first-only, empty-nonzero, scale-drop, None-as-rank-0, clamp-removal, and
/// NaN-passthrough mutants.
/// ABSORBS: rrf_score_distinguishes_adjacent_ranks,
/// independent_rrf_rational_oracle_agrees, fuse_rrf_sums_terms_and_zeroes_on_empty,
/// score_lexical_rrf_applies_scale,
/// weighted_rrf_ignores_absent_channels_and_clamps_weights.
#[test]
fn rrf_identity_fusion_and_weight_clamps() {
    // Facet 1: exact RRF values plus strict decrease over ranks.
    // Kills: `+ 1.0` dropped (1/60 vs 1/61), rank ignored, k ignored.
    assert_eq!(rrf_score(0, 60.0), 1.0 / 61.0);
    assert_eq!(rrf_score(1, 60.0), 1.0 / 62.0);
    assert_eq!(rrf_score(0, 0.0), 1.0);
    assert!(rrf_score(0, 0.0) > rrf_score(0, 60.0));
    let ranks: Vec<f64> = (0..5).map(|r| rrf_score(r, 60.0)).collect();
    for pair in ranks.windows(2) {
        assert!(pair[0] > pair[1], "RRF must strictly decrease: {ranks:?}");
    }
    // Facet 2: independent rational oracle — cross-multiplication identity
    // score*(k+r+1)==1 over ranks×k (deliberately not the formula restated).
    for rank in 0..5usize {
        for k in [0.0, 60.0] {
            let score = rrf_score(rank, k);
            let identity = score * (k + rank as f64 + 1.0);
            assert!((identity - 1.0).abs() < 1e-12, "rank={rank} k={k}");
        }
    }
    // Facet 3: fusion sums RRF terms, empty→0.0.
    assert_eq!(fuse_rrf(&[], 60.0), 0.0);
    assert_eq!(fuse_rrf(&[0], 60.0), 1.0 / 61.0);
    assert_eq!(fuse_rrf(&[0, 1], 60.0), 1.0 / 61.0 + 1.0 / 62.0);
    // Facet 4: lexical RRF applies the 200x scale (200/61), empty→0.
    let got = score_lexical_rrf(&[0]);
    assert!((got - 200.0 / 61.0).abs() < 1e-12, "got {got}");
    assert_eq!(score_lexical_rrf(&[]), 0.0);
    // Facet 5: weighted RRF skips None, sums channels, clamps huge→2x, NaN→1x.
    assert_eq!(FusionChannel::ALL.len(), 8);
    let weights = unit_channel_weights();
    assert_eq!(ChannelRanks::default().get(FusionChannel::Lexical), None);
    assert_eq!(weighted_rrf_score(&ChannelRanks::default(), &weights), 0.0);
    let one = ChannelRanks {
        lexical: Some(0),
        ..ChannelRanks::default()
    };
    assert_eq!(weighted_rrf_score(&one, &weights), 1.0 / 61.0);
    let two = ChannelRanks {
        lexical: Some(0),
        definition: Some(0),
        ..ChannelRanks::default()
    };
    assert_eq!(weighted_rrf_score(&two, &weights), 2.0 / 61.0);
    let huge = ChannelWeights {
        lexical: 100.0,
        ..unit_channel_weights()
    };
    assert_eq!(weighted_rrf_score(&one, &huge), 2.0 / 61.0);
    let nan = ChannelWeights {
        lexical: f64::NAN,
        ..unit_channel_weights()
    };
    assert_eq!(weighted_rrf_score(&one, &nan), 1.0 / 61.0);
}

/// INTENT: symbol scoring is the exact=5/substring=2/absent=0 ladder with
/// case-fold and a 2-char substring floor; best=max vs coverage=sum split.
/// KILLS: branch-swap, floor-flip, one-sided-fold, max↔sum-swap mutants.
/// ABSORBS: score_symbol_exact_substring_absent_ladder,
/// best_and_coverage_scores_split_max_vs_sum.
#[test]
fn symbol_score_ladder_max_vs_sum() {
    // Facet 1: exact/substring/absent ladder with case-fold and 2-char floor.
    assert_eq!(score_symbol("foo", "foo"), 5.0);
    assert_eq!(score_symbol("Foo", "foo"), 5.0);
    assert_eq!(score_symbol("ab", "abc"), 2.0);
    assert_eq!(score_symbol("abc", "ab"), 2.0);
    assert_eq!(score_symbol("xyz", "abc"), 0.0);
    assert_eq!(score_symbol("a", "abc"), 0.0);
    assert_eq!(score_symbol("ab", "a"), 0.0);
    // Facet 2: best=max vs coverage=sum split (2.0 vs 4.0) plus empty→0.
    let terms = vec!["foo".to_string(), "bar".to_string()];
    assert_eq!(best_symbol_score(&terms, "foo bar"), 2.0);
    assert_eq!(coverage_symbol_score(&terms, "foo bar"), 4.0);
    assert_eq!(best_symbol_score(&[], "foo"), 0.0);
    assert_eq!(coverage_symbol_score(&[], "foo"), 0.0);
    assert_eq!(best_symbol_score(&["foo".to_string()], "foo"), 5.0);
}

/// INTENT: signal provenance (kind→signal mapping, strict strength ladder),
/// confidence bases with capped agreement bonus, and order-free dedup merge
/// with why/format rendering.
/// KILLS: ladder-swap, confidence-const, merge-key, order-dependence, and
/// format mutants.
/// ABSORBS: signal_ladder_confidence_and_dedup_merge.
#[test]
fn signal_ladder_confidence_and_dedup_merge() {
    // Provenance mapping: exact text, structural kinds, semantic embed.
    assert_eq!(HitKind::Asgrep.signal(), HitSignal::Exact);
    assert_eq!(HitKind::Embed.signal(), HitSignal::Semantic);
    for kind in [
        HitKind::Def,
        HitKind::Caller,
        HitKind::Graph,
        HitKind::Anchor,
        HitKind::Import,
        HitKind::Pattern,
    ] {
        assert_eq!(kind.signal(), HitSignal::Structural, "{kind:?}");
    }
    // Strength ladder is strict and hand-pinned.
    assert_eq!(HitSignal::Semantic.rank(), 0);
    assert_eq!(HitSignal::Structural.rank(), 1);
    assert_eq!(HitSignal::Exact.rank(), 2);
    assert_eq!(HitSignal::ALL.len(), 3);
    assert_eq!(HitSignal::Exact.as_str(), "exact");
    assert_eq!(HitSignal::Structural.as_str(), "structural");
    assert_eq!(HitSignal::Semantic.as_str(), "semantic");
    assert_eq!(HitKind::Asgrep.as_str(), "asgrep");
    assert_eq!(HitKind::Def.as_str(), "def");
    assert_eq!(HitKind::Embed.as_str(), "embed");

    // Empty in, empty out.
    assert!(dedup_hits(vec![]).is_empty());
    // Singletons keep their base confidence: exact .75 / structural .60 / semantic .35.
    assert_eq!(dedup_hits(vec![mk_hit(HitKind::Asgrep, "a.rs", 1, 5.0)])[0].confidence, 0.75);
    assert_eq!(dedup_hits(vec![mk_hit(HitKind::Def, "a.rs", 1, 5.0)])[0].confidence, 0.60);
    assert_eq!(dedup_hits(vec![mk_hit(HitKind::Embed, "a.rs", 1, 5.0)])[0].confidence, 0.35);

    // Same location merges: best score wins regardless of input order.
    let low = mk_hit(HitKind::Asgrep, "a.rs", 1, 1.0);
    let high = mk_hit(HitKind::Def, "a.rs", 1, 5.0);
    let merged = dedup_hits(vec![low.clone(), high.clone()]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].score, 5.0);
    assert_eq!(merged[0].kind, HitKind::Def);
    let swapped = dedup_hits(vec![high, low]);
    assert_eq!(swapped.len(), 1);
    assert_eq!(swapped[0].score, 5.0);
    assert_eq!(sorted_contributors(&merged[0]), sorted_contributors(&swapped[0]));
    assert_eq!(sorted_contributors(&merged[0]), vec!["asgrep", "def"]);
    // Agreement bonus: one extra contributor adds .08 to the exact base.
    assert!((merged[0].confidence - 0.83).abs() < 1e-12);
    // Bonus caps at three extra contributors: 4- and 8-way merges agree at .99.
    let kinds = [
        HitKind::Asgrep,
        HitKind::Def,
        HitKind::Caller,
        HitKind::Graph,
        HitKind::Anchor,
        HitKind::Import,
        HitKind::Pattern,
        HitKind::Embed,
    ];
    let four = dedup_hits(
        kinds[..4].iter().map(|kind| mk_hit(*kind, "a.rs", 1, 2.0)).collect(),
    );
    let eight = dedup_hits(
        kinds.iter().map(|kind| mk_hit(*kind, "a.rs", 1, 2.0)).collect(),
    );
    assert_eq!(four.len(), 1);
    assert_eq!(eight.len(), 1);
    assert_eq!(four[0].confidence, eight[0].confidence);
    assert!((four[0].confidence - 0.99).abs() < 1e-12);

    // Identity: None is not Some("") — the pair must NOT merge.
    let mut with_none = mk_hit(HitKind::Asgrep, "a.rs", 1, 1.0);
    with_none.symbol = None;
    let mut with_empty = mk_hit(HitKind::Asgrep, "a.rs", 1, 1.0);
    with_empty.symbol = Some(String::new());
    assert_eq!(dedup_hits(vec![with_none, with_empty]).len(), 2);
    // Distinct lines never merge.
    assert_eq!(
        dedup_hits(vec![
            mk_hit(HitKind::Asgrep, "a.rs", 1, 1.0),
            mk_hit(HitKind::Asgrep, "a.rs", 2, 1.0),
        ])
        .len(),
        2
    );
    // Idempotence: dedup is a fixpoint on keys and contributor sets.
    let input = vec![
        mk_hit(HitKind::Asgrep, "a.rs", 1, 1.0),
        mk_hit(HitKind::Def, "a.rs", 1, 5.0),
        mk_hit(HitKind::Embed, "b.rs", 3, 2.0),
    ];
    let once = dedup_hits(input);
    let twice = dedup_hits(once.clone());
    assert_eq!(
        once.iter().map(hit_key_bits).collect::<Vec<_>>(),
        twice.iter().map(hit_key_bits).collect::<Vec<_>>()
    );
    assert_eq!(
        once.iter().map(sorted_contributors).collect::<Vec<_>>(),
        twice.iter().map(sorted_contributors).collect::<Vec<_>>()
    );

    // hit_why renders contributor evidence in contributor order.
    let mut merged_why = mk_hit(HitKind::Asgrep, "a.rs", 1, 1.0);
    merged_why.contributors = vec![HitKind::Asgrep, HitKind::Def];
    assert_eq!(hit_why(&merged_why), vec!["exact_text", "exact_symbol"]);
    let mut caller = mk_hit(HitKind::Caller, "f.rs", 7, 1.0);
    caller.caller = Some("main".to_string());
    assert_eq!(hit_why(&caller), vec!["called_by:main"]);
    let bare_caller = mk_hit(HitKind::Caller, "f.rs", 7, 1.0);
    assert_eq!(hit_why(&bare_caller), vec!["caller_edge"]);
    let mut noted = mk_hit(HitKind::Asgrep, "a.rs", 1, 1.0);
    noted.critic = vec![CriticNote::ChannelAgreement];
    assert_eq!(hit_why(&noted), vec!["exact_text", "critic:channel_agreement"]);

    // format_hit_line: one hand-pinned prefix per kind.
    assert_eq!(
        format_hit_line(&mk_hit(HitKind::Asgrep, "f.rs", 1, 1.0))
            .starts_with("ASGREP: "),
        true
    );
    let mut def = mk_hit(HitKind::Def, "f.rs", 1, 1.0);
    def.line_end = 2;
    def.excerpt = "exc".to_string();
    assert_eq!(format_hit_line(&def), "DEF: f.rs: ? span=1..2 | exc");
    assert_eq!(format_hit_line(&caller), "CALLER: f.rs: main -> ?");
    let mut graph = mk_hit(HitKind::Graph, "f.rs", 7, 1.0);
    graph.caller = Some("a".to_string());
    graph.callee = Some("b".to_string());
    assert_eq!(format_hit_line(&graph), "GRAPH: f.rs: a calls b");
    assert!(format_hit_line(&mk_hit(HitKind::Anchor, "f.rs", 1, 1.0)).starts_with("ANCHOR: "));
    assert!(format_hit_line(&mk_hit(HitKind::Pattern, "f.rs", 1, 1.0)).starts_with("PATTERN: "));
    assert!(format_hit_line(&mk_hit(HitKind::Import, "f.rs", 1, 1.0)).starts_with("IMPORT: "));
    let mut embed = mk_hit(HitKind::Embed, "f.rs", 1, 1.0);
    embed.symbol = Some("sym".to_string());
    embed.excerpt = "e".to_string();
    assert_eq!(format_hit_line(&embed), "EMBED: f.rs:1-1: sym | e");
    // Long excerpts truncate to 120 chars plus "..." on excerpt-carrying rows.
    let mut long = mk_hit(HitKind::Def, "f.rs", 1, 1.0);
    long.excerpt = "a".repeat(200);
    let line = format_hit_line(&long);
    assert!(line.ends_with("..."));
    assert_eq!(line.rsplit('|').next().expect("tail").trim().len(), 123);
    let mut long_uni = mk_hit(HitKind::Def, "f.rs", 1, 1.0);
    long_uni.excerpt = "é".repeat(200);
    assert!(format_hit_line(&long_uni).ends_with("..."));
}

/// INTENT: query intent classification (prefix routes + hybrid heuristics),
/// pinned per-intent channel weights, routing normalization/clamp/contraction,
/// and raw-vs-normalized scoring-path agreement.
/// KILLS: misroute, weight-drift, route-clamp, path-split, and
/// normalization mutants.
/// ABSORBS: intent_classify_weights_routing_and_scoring.
#[test]
fn intent_classify_weights_routing_and_scoring() {
    // Mode prefixes route to fixed intents.
    assert_eq!(classify(&ParsedQuery::parse("defs:foo")), QueryIntent::Symbol);
    assert_eq!(classify(&ParsedQuery::parse("callers:Bar")), QueryIntent::Symbol);
    assert_eq!(classify(&ParsedQuery::parse("imports:os")), QueryIntent::Symbol);
    assert_eq!(classify(&ParsedQuery::parse("pattern:$A")), QueryIntent::Structural);
    assert_eq!(classify(&ParsedQuery::parse("literal:Foo")), QueryIntent::Literal);
    assert_eq!(classify(&ParsedQuery::parse("word:Foo")), QueryIntent::Literal);
    assert_eq!(classify(&ParsedQuery::parse("regex:A+")), QueryIntent::Literal);
    // Hybrid heuristics: quoted → literal; markers → structural; idents → symbol.
    assert_eq!(classify(&ParsedQuery::parse("\"exact phrase\"")), QueryIntent::Literal);
    assert_eq!(classify(&ParsedQuery::parse("foo {")), QueryIntent::Structural);
    assert_eq!(classify(&ParsedQuery::parse("a => b")), QueryIntent::Structural);
    assert_eq!(classify(&ParsedQuery::parse("foo_bar")), QueryIntent::Symbol);
    assert_eq!(classify(&ParsedQuery::parse("Foo")), QueryIntent::Symbol);
    assert_eq!(classify(&ParsedQuery::parse("café_au_lait")), QueryIntent::Symbol);
    // Longer prose and empty queries fall through to conceptual.
    assert_eq!(classify(&ParsedQuery::parse("hello world foo")), QueryIntent::Conceptual);
    assert_eq!(classify(&ParsedQuery::parse("héllo wörld foo bar")), QueryIntent::Conceptual);
    assert_eq!(classify(&ParsedQuery::parse("")), QueryIntent::Conceptual);
    assert_eq!(classify(&ParsedQuery::parse("   ")), QueryIntent::Conceptual);
    assert_eq!(QueryIntent::Literal.as_str(), "literal");
    assert_eq!(QueryIntent::Symbol.as_str(), "symbol");
    assert_eq!(QueryIntent::Structural.as_str(), "structural");
    assert_eq!(QueryIntent::Conceptual.as_str(), "conceptual");
    // Determinism under repetition.
    for raw in ["defs:foo", "foo {", "\"q\"", "hello world foo", ""] {
        assert_eq!(classify(&ParsedQuery::parse(raw)), classify(&ParsedQuery::parse(raw)));
    }

    // Weight tables are hand-pinned; literal/structural use the uniform default.
    assert_eq!(
        default_weights(QueryIntent::Symbol),
        ChannelWeights {
            lexical: 0.8,
            def: 2.0,
            caller: 1.0,
            graph: 0.7,
            anchor: 1.0,
            embed: 0.7,
            pattern: 0.25,
            import: 0.8,
        }
    );
    assert_eq!(
        default_weights(QueryIntent::Conceptual),
        ChannelWeights {
            lexical: 0.75,
            def: 1.35,
            caller: 0.45,
            graph: 0.25,
            anchor: 0.7,
            embed: 1.45,
            pattern: 0.25,
            import: 0.5,
        }
    );
    assert_eq!(default_weights(QueryIntent::Literal), ChannelWeights::default());
    assert_eq!(default_weights(QueryIntent::Structural), ChannelWeights::default());
    // Without an override spec, weights_for is exactly the default table.
    std::env::remove_var("ASGREP_INTENT_WEIGHTS");
    for intent in [
        QueryIntent::Literal,
        QueryIntent::Symbol,
        QueryIntent::Structural,
        QueryIntent::Conceptual,
    ] {
        assert_eq!(weights_for(intent), default_weights(intent), "{intent:?}");
    }

    // Routing normalizes by the channel ceiling: an at-ceiling hit maps to 1.0.
    let ceiling = rrf_score(0, RRF_K) * LEXICAL_RRF_SCALE;
    let parsed = ParsedQuery::parse("needle");
    let mut at_ceiling = vec![mk_hit(HitKind::Asgrep, "a.rs", 1, ceiling)];
    route_hits(&parsed, &mut at_ceiling);
    assert_eq!(at_ceiling[0].score, 1.0);
    // Oversize scores clamp to 1.0 rather than escaping the unit range.
    let mut huge = vec![mk_hit(HitKind::Asgrep, "a.rs", 1, 1e18)];
    route_hits(&parsed, &mut huge);
    assert_eq!(huge[0].score, 1.0);
    // Empty-term queries zero TEXT channels but still scale non-text ones.
    let empty = ParsedQuery::parse("");
    let mut text = vec![mk_hit(HitKind::Asgrep, "a.rs", 1, 5.0)];
    route_hits(&empty, &mut text);
    assert_eq!(text[0].score, 0.0);
    let mut structural_text = vec![mk_hit(HitKind::Def, "a.rs", 1, 5.0)];
    route_hits(&empty, &mut structural_text);
    assert_eq!(structural_text[0].score, 0.0);
    let mut semantic = vec![mk_hit(HitKind::Embed, "a.rs", 1, 2.0)];
    route_hits(&empty, &mut semantic);
    assert_eq!(semantic[0].score, 0.5);
    // Differential: a def hit scored by the scoring path normalizes to exactly 1.0.
    let defs = ParsedQuery::parse("defs:foo");
    let def_score = score_def(&defs.terms, "foo");
    assert_eq!(def_score, 13.0);
    let mut def_hit = vec![SearchHit::span(SpanHitInput {
        kind: HitKind::Def,
        file: "a.rs".to_string(),
        line_start: 1,
        line_end: 2,
        score: def_score,
        excerpt: "fn foo()".to_string(),
        symbol: Some("foo".to_string()),
        language: None,
        byte_span: None,
    })];
    route_hits(&defs, &mut def_hit);
    assert_eq!(def_hit[0].score, 1.0);
    // Routing is a contraction: a second pass never raises any score.
    for mut hits in [
        vec![mk_hit(HitKind::Asgrep, "a.rs", 1, ceiling)],
        vec![mk_hit(HitKind::Asgrep, "a.rs", 1, 5.0)],
        vec![mk_hit(HitKind::Embed, "a.rs", 1, 2.0)],
    ] {
        route_hits(&parsed, &mut hits);
        let first = hits[0].score;
        route_hits(&parsed, &mut hits);
        assert!(hits[0].score <= first, "second pass must not raise");
    }
    // Routing is deterministic for identical inputs.
    let mut left = vec![mk_hit(HitKind::Asgrep, "a.rs", 1, 7.0)];
    let mut right = vec![mk_hit(HitKind::Asgrep, "a.rs", 1, 7.0)];
    route_hits(&parsed, &mut left);
    route_hits(&parsed, &mut right);
    assert_eq!(left[0].score, right[0].score);

    // Scoring tables: exact def/caller scale from coverage; zero stays zero.
    assert_eq!(score_def(&["foo".to_string()], "foo"), 13.0);
    assert_eq!(score_caller(&["foo".to_string()], "foo"), 11.5);
    assert_eq!(score_def(&[], "foo"), 0.0);
    assert_eq!(score_def(&["xyz".to_string()], "foo"), 0.0);
    assert_eq!(score_caller(&[], "x"), 0.0);
    assert_eq!(score_symbol("é", "é"), 5.0);
    assert_eq!(score_symbol("É", "é"), 5.0);
    assert_eq!(score_symbol("é", "éx"), 0.0);
    // Normalization is idempotent and lowercases across scripts.
    let raw = ["Foo".to_string(), "BAR_baz".to_string(), "É".to_string()];
    let normalized = normalize_query_terms(&raw);
    assert_eq!(normalized, vec!["foo", "bar_baz", "é"]);
    assert_eq!(normalize_query_terms(&normalized), normalized);
    // Differential: raw and pre-normalized scoring paths agree exactly.
    let term_sets: Vec<Vec<String>> = vec![
        vec![],
        vec!["foo".to_string()],
        vec!["Foo".to_string()],
        vec!["a".to_string()],
        vec!["École".to_string()],
        vec!["foo_bar".to_string()],
        vec!["foo".to_string(), "bar".to_string()],
    ];
    for terms in &term_sets {
        let pre = normalize_query_terms(terms);
        for symbol in ["foo", "Foo", "foobar", "xyz", "école", "a", "foo_bar"] {
            assert_eq!(score_def(terms, symbol), score_def_normalized(&pre, symbol));
            assert_eq!(score_caller(terms, symbol), score_caller_normalized(&pre, symbol));
            assert_eq!(best_symbol_score(terms, symbol), best_symbol_score_normalized(&pre, symbol));
            assert_eq!(
                coverage_symbol_score(terms, symbol),
                coverage_symbol_score_normalized(&pre, symbol)
            );
            // Best (max) can never exceed coverage (sum of non-negative parts).
            assert!(best_symbol_score(terms, symbol) <= coverage_symbol_score(terms, symbol));
        }
    }
    // Coverage is monotone non-decreasing as matching terms accumulate.
    let mut previous = 0.0;
    for end in 1..=4 {
        let terms: Vec<String> = ["foo", "bar", "xyz", "foo"][..end]
            .iter()
            .map(|term| term.to_string())
            .collect();
        let coverage = coverage_symbol_score(&terms, "foo bar");
        assert!(coverage >= previous, "monotone at {end}");
        previous = coverage;
    }
}
