//! N4 end-to-end scoring drills, kept standalone: FULL score -> rank -> fuse pipelines.
//!
//! Each drill runs the whole pipeline — raw producer scores (`score_def`, hand
//! raws) -> `route_hits` normalization (per-hit ceilings, clamp) ->
//! `apply_weighted_rrf` (within-channel ranks + weighted RRF sums) — on
//! hand-built adversarial corpora, asserting EXACT final orderings and EXACT
//! fused scores. One drill extends through `finish_response` to the
//! user-visible ranking. Each test carries INTENT + KILLS + ABSORBS.
//!
//! Every expectation is HAND-DERIVED (rational arithmetic in the comment above
//! it). Nothing here snapshots production output. Bit-equality oracles reuse
//! the op-order discipline proven in N1/N2: single terms as `w * (1.0/61.0)`,
//! multi-term sums in `FusionChannel::ALL` order starting from `0.0`
//! (`0.0 + x == x` exactly), ceilings written with production's operations.
//!
//! No test touches env vars, so this binary is parallel-safe.

use ast_sgrep_core::fusion::apply_weighted_rrf;
use ast_sgrep_core::intent::{route_hits, ChannelWeights};
use ast_sgrep_core::rank::score_def;
use ast_sgrep_core::search::{finish_response, HitKind, SearchHit};
use ast_sgrep_core::ParsedQuery;
use ast_sgrep_testkit::{
    crossover_corpus, finish_options, fused_key, hit_files_in_order, mk_hit, rank_fused,
    route_fuse_pipeline, tie_corpus,
};

/// INTENT: two-channel breadth beats a 500-raw single spike clamped to 1.0;
/// exact fused scores + order.
/// KILLS: clamp/fuse-mutant.
/// ABSORBS: none (KEEP standalone: `breadth_beats_clamped_single_spike`).
#[test]
fn breadth_beats_clamped_single_spike() {
    // Query "foo": Asgrep ceiling c = (1/61)*200 ~= 3.2787; Def ceiling 13
    // (symbol "foo", matched 1, no spelling); Graph ceiling 5.
    // Raws: P = {asgrep 3.0, def 13.0}, Q = {asgrep 2.0, def 6.5},
    // R = {graph 500.0}. R's raw dwarfs every other raw (500 >> 13).
    // Route: P -> {3/c ~= 0.915, 1.0}, Q -> {2/c ~= 0.61, 0.5},
    // R -> min(500/5, 1) = 1.0 (clamp erases the 100x spike).
    // Ranks: asgrep P=0 Q=1; def P=0 Q=1; graph R=0 (sole hit).
    // Fused (unit weights): P = 1/61+1/61 ~= 0.03279,
    // Q = 1/62+1/62 ~= 0.03226, R = 1/61 ~= 0.01639.
    // Final: P > Q > R — the 500-raw spike finishes LAST.
    let parsed = ParsedQuery::parse("foo");
    let mut p_def = mk_hit(HitKind::Def, "p.rs", 1, 13.0);
    p_def.symbol = Some("foo".to_string());
    let mut q_def = mk_hit(HitKind::Def, "q.rs", 1, 6.5);
    q_def.symbol = Some("foo".to_string());
    let fused = route_fuse_pipeline(
        &parsed,
        vec![
            mk_hit(HitKind::Asgrep, "p.rs", 1, 3.0),
            p_def,
            mk_hit(HitKind::Asgrep, "q.rs", 1, 2.0),
            q_def,
            mk_hit(HitKind::Graph, "r.rs", 1, 500.0),
        ],
        &ChannelWeights::default(),
    );
    assert_eq!(fused.len(), 3);
    let by_file = |f: &str| fused.iter().find(|h| h.file == f).unwrap();
    assert_eq!(by_file("p.rs").score, 1.0 / 61.0 + 1.0 / 61.0);
    assert_eq!(by_file("q.rs").score, 1.0 / 62.0 + 1.0 / 62.0);
    assert_eq!(by_file("r.rs").score, 1.0 / 61.0);
    let ranked = rank_fused(fused);
    assert_eq!(hit_files_in_order(&ranked), vec!["p.rs", "q.rs", "r.rs"]);
}

/// INTENT: same corpus ties at unit weights; lex-heavy -> A first, def-heavy
/// -> B first; exact tilted scores.
/// KILLS: weight-application-mutant.
/// ABSORBS: none (KEEP standalone: `weight_tilt_flips_fused_order`).
#[test]
fn weight_tilt_flips_fused_order() {
    let parsed = ParsedQuery::parse("foo");
    // Unit weights: A = 1/61+1/62, B = 1/62+1/61 — the same two-term
    // multiset, and IEEE `+` is commutative, so the fused scores are
    // BIT-identical; the tie breaks by file: [a, b].
    let fused = route_fuse_pipeline(&parsed, crossover_corpus(), &ChannelWeights::default());
    assert_eq!(fused.len(), 2);
    let by_file = |fused: &[SearchHit], f: &str| fused.iter().find(|h| h.file == f).unwrap().score;
    assert_eq!(by_file(&fused, "a.rs"), 1.0 / 61.0 + 1.0 / 62.0);
    assert_eq!(by_file(&fused, "b.rs"), 1.0 / 62.0 + 1.0 / 61.0);
    assert_eq!(by_file(&fused, "a.rs"), by_file(&fused, "b.rs"));
    assert_eq!(hit_files_in_order(&rank_fused(fused)), vec!["a.rs", "b.rs"]);

    // Lex-heavy (lex 2.0, def 0.25): A - B = (2-0.25)*(1/61-1/62)
    // = 1.75/3782 ~= 4.63e-4 > 0, strict A first.
    let lex_heavy = ChannelWeights {
        lexical: 2.0,
        def: 0.25,
        ..ChannelWeights::default()
    };
    let fused = route_fuse_pipeline(&parsed, crossover_corpus(), &lex_heavy);
    assert_eq!(
        by_file(&fused, "a.rs"),
        2.0 * (1.0 / 61.0) + 0.25 * (1.0 / 62.0)
    );
    assert_eq!(
        by_file(&fused, "b.rs"),
        2.0 * (1.0 / 62.0) + 0.25 * (1.0 / 61.0)
    );
    assert!(by_file(&fused, "a.rs") > by_file(&fused, "b.rs"));
    assert_eq!(hit_files_in_order(&rank_fused(fused)), vec!["a.rs", "b.rs"]);

    // Def-heavy (lex 0.25, def 2.0): mirror image, B wins by the same
    // 1.75/3782 delta — the tilt flips the order the other way.
    let def_heavy = ChannelWeights {
        lexical: 0.25,
        def: 2.0,
        ..ChannelWeights::default()
    };
    let fused = route_fuse_pipeline(&parsed, crossover_corpus(), &def_heavy);
    assert_eq!(
        by_file(&fused, "a.rs"),
        0.25 * (1.0 / 61.0) + 2.0 * (1.0 / 62.0)
    );
    assert_eq!(
        by_file(&fused, "b.rs"),
        0.25 * (1.0 / 62.0) + 2.0 * (1.0 / 61.0)
    );
    assert!(by_file(&fused, "b.rs") > by_file(&fused, "a.rs"));
    assert_eq!(hit_files_in_order(&rank_fused(fused)), vec!["b.rs", "a.rs"]);
}

/// INTENT: routing CREATES ties from 100x-different raws; keys break them;
/// exact fused scores; reversal bit-identical.
/// KILLS: clamp/tiebreak-mutant.
/// ABSORBS: none (KEEP standalone: `routing_clamp_creates_ties_with_key_breaks`;
/// the reversal half overlaps the N3 permutation/tie cluster but the
/// clamp-creates-ties intent is unique).
#[test]
fn routing_clamp_creates_ties_with_key_breaks() {
    // Fused (unit): a = 1/61+1/61 (def0+caller0), b = 1/62+1/62,
    // c = 1/63, d = 1/64. Order a > b > c > d is total: every adjacent
    // gap is >= 1/63-1/64 ~= 2.5e-4, ~1e11 ulps at this magnitude.
    let parsed = ParsedQuery::parse("foo");
    let fused = route_fuse_pipeline(&parsed, tie_corpus(), &ChannelWeights::default());
    assert_eq!(fused.len(), 4);
    let by_file = |f: &str| fused.iter().find(|h| h.file == f).unwrap();
    assert_eq!(by_file("a.rs").score, 1.0 / 61.0 + 1.0 / 61.0);
    assert_eq!(by_file("b.rs").score, 1.0 / 62.0 + 1.0 / 62.0);
    assert_eq!(by_file("c.rs").score, 1.0 / 63.0);
    assert_eq!(by_file("d.rs").score, 1.0 / 64.0);
    assert_eq!(by_file("a.rs").kind, HitKind::Def);
    assert_eq!(
        by_file("a.rs").contributors,
        vec![HitKind::Def, HitKind::Caller]
    );
    let ranked = rank_fused(fused);
    assert_eq!(
        hit_files_in_order(&ranked),
        vec!["a.rs", "b.rs", "c.rs", "d.rs"]
    );

    // The tie order comes from keys, not input order: reversing the input
    // reproduces the fused stream bit-for-bit (emission is sorted by key).
    let forward: Vec<_> = route_fuse_pipeline(&parsed, tie_corpus(), &ChannelWeights::default())
        .iter()
        .map(fused_key)
        .collect();
    let mut reversed = tie_corpus();
    reversed.reverse();
    let backward: Vec<_> = route_fuse_pipeline(&parsed, reversed, &ChannelWeights::default())
        .iter()
        .map(fused_key)
        .collect();
    assert_eq!(forward, backward);
}

/// INTENT: 8 channels x 8 sanitize rails fuse to exact rail x 1/61 with
/// rail-cohort ranking.
/// KILLS: sanitize-rail-mutant.
/// ABSORBS: none (KEEP standalone: `hostile_weights_fuse_on_rails`).
#[test]
fn hostile_weights_fuse_on_rails() {
    // Eight channels, one hit each, every raw exactly at its ceiling so each
    // hit routes to 1.0 and takes rank 0 in its channel. Weights hit every
    // sanitize rail: 0.0/-5.0/-0.0/5e-324 -> 0.25, NaN/+inf -> 1.0,
    // 1e308/2.5 -> 2.0. Fused = rail * 1/61 per key:
    // caller = import = 2/61; graph = anchor = 1/61;
    // lexical = def = embed = pattern = 0.25/61.
    let parsed = ParsedQuery::parse("foo");
    let ceiling = (1.0f64 / 61.0) * 200.0; // Asgrep ceiling, production op order
    let mut def = mk_hit(HitKind::Def, "b_def.rs", 1, 13.0);
    def.symbol = Some("foo".to_string());
    let mut caller = mk_hit(HitKind::Caller, "c_caller.rs", 1, 11.5);
    caller.callee = Some("foo".to_string());
    let hits = vec![
        mk_hit(HitKind::Asgrep, "a_lex.rs", 1, ceiling),
        def,
        caller,
        mk_hit(HitKind::Graph, "d_graph.rs", 1, 5.0),
        mk_hit(HitKind::Anchor, "e_anchor.rs", 1, 6.0),
        mk_hit(HitKind::Embed, "f_embed.rs", 1, 4.0),
        mk_hit(HitKind::Pattern, "g_pattern.rs", 1, 7.0),
        mk_hit(HitKind::Import, "h_import.rs", 1, 2.0),
    ];
    let hostile = ChannelWeights {
        lexical: 0.0,
        def: -5.0,
        caller: 1e308,
        graph: f64::NAN,
        anchor: f64::INFINITY,
        embed: -0.0,
        pattern: 5e-324,
        import: 2.5,
    };
    let fused = route_fuse_pipeline(&parsed, hits, &hostile);
    assert_eq!(fused.len(), 8);
    let by_file = |f: &str| fused.iter().find(|h| h.file == f).unwrap().score;
    assert_eq!(by_file("c_caller.rs"), 2.0 * (1.0 / 61.0));
    assert_eq!(by_file("h_import.rs"), 2.0 * (1.0 / 61.0));
    assert_eq!(by_file("d_graph.rs"), 1.0 / 61.0);
    assert_eq!(by_file("e_anchor.rs"), 1.0 / 61.0);
    for f in ["a_lex.rs", "b_def.rs", "f_embed.rs", "g_pattern.rs"] {
        assert_eq!(
            by_file(f),
            0.25 * (1.0 / 61.0),
            "{f} must fuse on the 0.25 rail"
        );
    }
    // Score-desc ranking with file tiebreaks inside each rail cohort.
    assert_eq!(
        hit_files_in_order(&rank_fused(fused)),
        vec![
            "c_caller.rs",
            "h_import.rs",
            "d_graph.rs",
            "e_anchor.rs",
            "a_lex.rs",
            "b_def.rs",
            "f_embed.rs",
            "g_pattern.rs",
        ]
    );
}

/// INTENT: zeroed/negative hits vanish, lone Embed fuses; empty-terms kills
/// all text channels.
/// KILLS: fuse-gate/empty-terms-mutant.
/// ABSORBS: none (KEEP standalone: `empty_and_zeroed_channels_vanish`).
#[test]
fn empty_and_zeroed_channels_vanish() {
    // Only Embed carries signal: raws 4.0/2.0 -> routed 1.0/0.5 -> ranks
    // 0/1 -> 1/61, 1/62. The Asgrep 0.0 and Def 0.0 hits route to exactly
    // 0.0 and the Graph -3.0 clamps to 0.0, so the fuse gate (> 0.0) drops
    // all three before ranking; the other channels are simply absent.
    let parsed = ParsedQuery::parse("foo");
    let mut zeroed_def = mk_hit(HitKind::Def, "z2.rs", 1, 0.0);
    zeroed_def.symbol = Some("foo".to_string());
    let fused = route_fuse_pipeline(
        &parsed,
        vec![
            mk_hit(HitKind::Embed, "e1.rs", 1, 4.0),
            mk_hit(HitKind::Embed, "e2.rs", 1, 2.0),
            mk_hit(HitKind::Asgrep, "z1.rs", 1, 0.0),
            zeroed_def,
            mk_hit(HitKind::Graph, "z3.rs", 1, -3.0),
        ],
        &ChannelWeights::default(),
    );
    assert_eq!(fused.len(), 2);
    let by_file = |fused: &[SearchHit], f: &str| fused.iter().find(|h| h.file == f).unwrap().score;
    assert_eq!(by_file(&fused, "e1.rs"), 1.0 / 61.0);
    assert_eq!(by_file(&fused, "e2.rs"), 1.0 / 62.0);
    assert_eq!(
        hit_files_in_order(&rank_fused(fused)),
        vec!["e1.rs", "e2.rs"]
    );

    // Empty-terms query: all text channels (asgrep/def/graph/...) route to
    // 0.0 and vanish in fusion; the Embed 2.0/4 = 0.5 survivor is the sole
    // hit in its channel -> rank 0 -> exactly 1/61.
    let empty = ParsedQuery::parse("");
    assert!(empty.terms.is_empty());
    let mut def = mk_hit(HitKind::Def, "t2.rs", 1, 99.0);
    def.symbol = Some("foo".to_string());
    let fused = route_fuse_pipeline(
        &empty,
        vec![
            mk_hit(HitKind::Asgrep, "t1.rs", 1, 99.0),
            def,
            mk_hit(HitKind::Graph, "t3.rs", 1, 99.0),
            mk_hit(HitKind::Embed, "k.rs", 1, 2.0),
        ],
        &ChannelWeights::default(),
    );
    assert_eq!(fused.len(), 1);
    assert_eq!(fused[0].file, "k.rs");
    assert_eq!(fused[0].score, 1.0 / 61.0);
    assert_eq!(fused[0].contributors, vec![HitKind::Embed]);
}

/// INTENT: genuine score_def raws X>Y invert to Y>X via per-hit ceilings,
/// cemented in fusion.
/// KILLS: ceiling-denom-mutant.
/// ABSORBS: none (KEEP standalone: `real_producer_scores_invert_through_routing`).
#[test]
fn real_producer_scores_invert_through_routing() {
    // Terms ["aa","bb","cc"] (tokenizer sorts+dedups; all len 2 so no
    // identifier-spelling branch: denom = matched count).
    // X symbol "xx aa yy bb zz cc": each term substring-scores 2.0 ->
    // coverage 6.0 -> score_def = 2*6+3 = 15.0; matched 3 -> ceiling 33.
    // Y symbol "aa": exact 5.0, others 0 -> coverage 5.0 -> score_def 13.0;
    // matched 1 -> ceiling 13.
    // Raw: X 15.0 > Y 13.0. Routed: X 15/33 = 5/11 ~= 0.4545 < Y 1.0.
    // The raw leader loses its rank IN ROUTING, before fusion runs.
    let parsed = ParsedQuery::parse("aa bb cc");
    assert_eq!(
        parsed.terms,
        vec!["aa".to_string(), "bb".to_string(), "cc".to_string()]
    );
    assert_eq!(parsed.identifier_spelling(), None);
    let raw_x = score_def(&parsed.terms, "xx aa yy bb zz cc");
    let raw_y = score_def(&parsed.terms, "aa");
    assert_eq!(raw_x, 15.0);
    assert_eq!(raw_y, 13.0);
    assert!(raw_x > raw_y, "X must lead on raw producer score");

    let mut x = mk_hit(HitKind::Def, "x_inv.rs", 1, raw_x);
    x.symbol = Some("xx aa yy bb zz cc".to_string());
    let mut y = mk_hit(HitKind::Def, "y_inv.rs", 1, raw_y);
    y.symbol = Some("aa".to_string());
    let mut routed = vec![x, y];
    route_hits(&parsed, &mut routed);
    assert_eq!(routed[0].score, 15.0 / 33.0);
    assert_eq!(routed[1].score, 1.0);
    assert!(
        routed[1].score > routed[0].score,
        "routing must invert the pair"
    );

    // Fusion cements the inversion: Y rank 0 -> 1/61, X rank 1 -> 1/62.
    let mut fused = routed;
    apply_weighted_rrf(&mut fused, &ChannelWeights::default());
    assert_eq!(fused.len(), 2);
    let by_file = |f: &str| fused.iter().find(|h| h.file == f).unwrap().score;
    assert_eq!(by_file("y_inv.rs"), 1.0 / 61.0);
    assert_eq!(by_file("x_inv.rs"), 1.0 / 62.0);
    assert_eq!(
        hit_files_in_order(&rank_fused(fused)),
        vec!["y_inv.rs", "x_inv.rs"]
    );
}

/// INTENT: 3-channel merge — canonical Def, contributor order, breadth beats
/// three channel winners.
/// KILLS: merge/canonical-mutant.
/// ABSORBS: none (KEEP standalone: `three_way_merge_canonical_and_breadth_win`).
#[test]
fn three_way_merge_canonical_and_breadth_win() {
    // M = ("m.rs", 7) in three channels: asgrep 3.0 (3/c ~= 0.915),
    // def 13.0 (1.0), caller 11.5 (1.0). One competitor per channel:
    // L asgrep 3.2 (3.2/c ~= 0.976 > M), D def 13.0 (tie 1.0, file a < m),
    // C caller 11.5 (tie 1.0, file a < m). M takes rank 1 in all three
    // channels; each competitor is rank 0 alone in its channel.
    // Fused: M = 1/62+1/62+1/62 ~= 0.04839 (channel order lex, def,
    // caller); D = C = L = 1/61 ~= 0.01639. M wins by breadth despite
    // losing every channel head-to-head. Canonical kind: Def priority 0
    // beats Caller 1 and Asgrep 6; contributors sort by channel index.
    let parsed = ParsedQuery::parse("foo");
    let mut m_def = mk_hit(HitKind::Def, "m.rs", 7, 13.0);
    m_def.symbol = Some("foo".to_string());
    let mut m_caller = mk_hit(HitKind::Caller, "m.rs", 7, 11.5);
    m_caller.callee = Some("foo".to_string());
    let mut d_def = mk_hit(HitKind::Def, "a.rs", 1, 13.0);
    d_def.symbol = Some("foo".to_string());
    let mut c_caller = mk_hit(HitKind::Caller, "a.rs", 2, 11.5);
    c_caller.callee = Some("foo".to_string());
    let fused = route_fuse_pipeline(
        &parsed,
        vec![
            mk_hit(HitKind::Asgrep, "m.rs", 7, 3.0),
            m_def,
            m_caller,
            d_def,
            c_caller,
            mk_hit(HitKind::Asgrep, "a.rs", 3, 3.2),
        ],
        &ChannelWeights::default(),
    );
    assert_eq!(fused.len(), 4);
    let by_key = |f: &str, line: u32| {
        fused
            .iter()
            .find(|h| h.file == f && h.line_start == line)
            .unwrap()
    };
    let m = by_key("m.rs", 7);
    assert_eq!(m.score, 1.0 / 62.0 + 1.0 / 62.0 + 1.0 / 62.0);
    assert_eq!(m.kind, HitKind::Def);
    assert_eq!(
        m.contributors,
        vec![HitKind::Asgrep, HitKind::Def, HitKind::Caller]
    );
    assert_eq!(by_key("a.rs", 1).score, 1.0 / 61.0);
    assert_eq!(by_key("a.rs", 2).score, 1.0 / 61.0);
    assert_eq!(by_key("a.rs", 3).score, 1.0 / 61.0);
    let ranked = rank_fused(fused);
    assert_eq!(ranked[0].file, "m.rs");
    assert_eq!(
        ranked
            .iter()
            .map(|h| (h.file.as_str(), h.line_start))
            .collect::<Vec<_>>(),
        vec![("m.rs", 7), ("a.rs", 1), ("a.rs", 2), ("a.rs", 3)]
    );
}

/// INTENT: capstone route -> fuse -> finish — scores preserved, [b,a,c] order,
/// margins 0, confidences.
/// KILLS: finish-rewrite/sort-mutant.
/// ABSORBS: none (KEEP standalone: `full_pipeline_through_finish_order_scores_margins`).
#[test]
fn full_pipeline_through_finish_order_scores_margins() {
    // A: asgrep-only (3.0 -> 3/c, sole channel hit -> rank 0 -> 1/61),
    // canonical Asgrep (Exact). B: def 13.0 + caller 11.5 (both 1.0, rank
    // 0 in each -> 1/61+1/61), canonical Def (Structural). C: embed-only
    // (4.0 -> 1.0 -> 1/61), canonical Embed (Semantic).
    // Single-term query: finish sorts by score desc (coverage is 0 for all
    // — no excerpt contains "foo"), tie A/C broken by file: [b, a, c].
    // Finish never rewrites scores; each hit is alone in its signal group
    // so every margin is exactly 0.0. Limit 10 >> 3 hits: no prune, no
    // per-file cap, best-definition already retained, def-head no-op.
    let dir = tempfile::tempdir().unwrap();
    let parsed = ParsedQuery::parse("foo");
    let mut b_def = mk_hit(HitKind::Def, "b_struct.rs", 1, 13.0);
    b_def.symbol = Some("foo".to_string());
    let mut b_caller = mk_hit(HitKind::Caller, "b_struct.rs", 1, 11.5);
    b_caller.callee = Some("foo".to_string());
    let fused = route_fuse_pipeline(
        &parsed,
        vec![
            mk_hit(HitKind::Asgrep, "a_exact.rs", 1, 3.0),
            b_def,
            b_caller,
            mk_hit(HitKind::Embed, "c_sem.rs", 1, 4.0),
        ],
        &ChannelWeights::default(),
    );
    assert_eq!(fused.len(), 3);
    let response = finish_response(&parsed, &finish_options(dir.path(), 10), fused, false);
    assert_eq!(response.hits.len(), 3);
    assert_eq!(
        hit_files_in_order(&response.hits),
        vec!["b_struct.rs", "a_exact.rs", "c_sem.rs"]
    );
    let by_file = |f: &str| response.hits.iter().find(|h| h.file == f).unwrap();
    assert_eq!(by_file("b_struct.rs").score, 1.0 / 61.0 + 1.0 / 61.0);
    assert_eq!(by_file("a_exact.rs").score, 1.0 / 61.0);
    assert_eq!(by_file("c_sem.rs").score, 1.0 / 61.0);
    for hit in &response.hits {
        assert_eq!(
            hit.margin, 0.0,
            "{} must be alone in its signal group",
            hit.file
        );
    }
    // Confidence: Exact base 0.75 / Semantic base 0.35 with one contributor
    // are bit-exact; the 2-contributor Structural row needs epsilon (0.60
    // and 0.08 are inexact in binary; N1 precedent).
    assert_eq!(by_file("a_exact.rs").confidence, 0.75);
    assert_eq!(by_file("c_sem.rs").confidence, 0.35);
    assert!((by_file("b_struct.rs").confidence - 0.68).abs() < 1e-12);
}
