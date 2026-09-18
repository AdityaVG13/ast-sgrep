//! N4 end-to-end scoring drills: FULL score -> rank -> fuse pipelines.
//!
//! N1 pins single-function exact values, N2 pins totality on hostile inputs,
//! N3 pins metamorphic relations. N4 runs the whole pipeline — raw producer
//! scores (`score_def`, hand raws) -> `route_hits` normalization (per-hit
//! ceilings, clamp) -> `apply_weighted_rrf` (within-channel ranks + weighted
//! RRF sums) — on hand-built adversarial corpora, asserting EXACT final
//! orderings and EXACT fused scores. One drill extends through
//! `finish_response` to the user-visible ranking.
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
use ast_sgrep_core::search::{
    finish_response, HitKind, SearchHit, SearchOptions, SpanHitInput,
};
use ast_sgrep_core::ParsedQuery;

fn n4_hit(kind: HitKind, file: &str, line: u32, score: f64) -> SearchHit {
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

fn n4_options(root: &std::path::Path, limit: usize) -> SearchOptions {
    SearchOptions {
        root: root.to_path_buf(),
        limit,
        file_filter: None,
        count_only: false,
        use_rerank: false,
        ..SearchOptions::default()
    }
}

/// Run the full score -> rank -> fuse pipeline: normalize raws against
/// per-hit ceilings, then rank within channels and fuse with `weights`.
fn n4_pipeline(parsed: &ParsedQuery, hits: Vec<SearchHit>, weights: &ChannelWeights) -> Vec<SearchHit> {
    let mut hits = hits;
    route_hits(parsed, &mut hits);
    apply_weighted_rrf(&mut hits, weights);
    hits
}

/// Final ranking order: fused score desc, ties by (file, line).
fn n4_ranked(mut fused: Vec<SearchHit>) -> Vec<SearchHit> {
    fused.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line_start.cmp(&b.line_start))
    });
    fused
}

fn n4_files(hits: &[SearchHit]) -> Vec<String> {
    hits.iter().map(|h| h.file.clone()).collect()
}

/// Full observable fused row (SearchHit has no PartialEq).
fn n4_key(hit: &SearchHit) -> (HitKind, String, u32, u32, u64, Vec<HitKind>) {
    (
        hit.kind,
        hit.file.clone(),
        hit.line_start,
        hit.line_end,
        hit.score.to_bits(),
        hit.contributors.clone(),
    )
}

// ---------------------------------------------------------------------------
// 1. Breadth beats a clamped single spike: the max raw finishes last
// ---------------------------------------------------------------------------

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
    let mut p_def = n4_hit(HitKind::Def, "p.rs", 1, 13.0);
    p_def.symbol = Some("foo".to_string());
    let mut q_def = n4_hit(HitKind::Def, "q.rs", 1, 6.5);
    q_def.symbol = Some("foo".to_string());
    let fused = n4_pipeline(
        &parsed,
        vec![
            n4_hit(HitKind::Asgrep, "p.rs", 1, 3.0),
            p_def,
            n4_hit(HitKind::Asgrep, "q.rs", 1, 2.0),
            q_def,
            n4_hit(HitKind::Graph, "r.rs", 1, 500.0),
        ],
        &ChannelWeights::default(),
    );
    assert_eq!(fused.len(), 3);
    let by_file = |f: &str| fused.iter().find(|h| h.file == f).unwrap();
    assert_eq!(by_file("p.rs").score, 1.0 / 61.0 + 1.0 / 61.0);
    assert_eq!(by_file("q.rs").score, 1.0 / 62.0 + 1.0 / 62.0);
    assert_eq!(by_file("r.rs").score, 1.0 / 61.0);
    let ranked = n4_ranked(fused);
    assert_eq!(n4_files(&ranked), vec!["p.rs", "q.rs", "r.rs"]);
}

// ---------------------------------------------------------------------------
// 2. One corpus, three weightings: tie, then a tilt flips the order both ways
// ---------------------------------------------------------------------------

fn n4_crossover_corpus() -> Vec<SearchHit> {
    // Asgrep raws A=3.0 > B=2.0; Def raws A=6.5 (routed 0.5) < B=13.0
    // (routed 1.0, both symbol "foo", ceiling 13). Opposite within-channel
    // orders: A = (lex 0, def 1), B = (lex 1, def 0).
    let mut a_def = n4_hit(HitKind::Def, "a.rs", 1, 6.5);
    a_def.symbol = Some("foo".to_string());
    let mut b_def = n4_hit(HitKind::Def, "b.rs", 1, 13.0);
    b_def.symbol = Some("foo".to_string());
    vec![
        n4_hit(HitKind::Asgrep, "a.rs", 1, 3.0),
        a_def,
        n4_hit(HitKind::Asgrep, "b.rs", 1, 2.0),
        b_def,
    ]
}

#[test]
fn weight_tilt_flips_fused_order() {
    let parsed = ParsedQuery::parse("foo");
    // Unit weights: A = 1/61+1/62, B = 1/62+1/61 — the same two-term
    // multiset, and IEEE `+` is commutative, so the fused scores are
    // BIT-identical; the tie breaks by file: [a, b].
    let fused = n4_pipeline(&parsed, n4_crossover_corpus(), &ChannelWeights::default());
    assert_eq!(fused.len(), 2);
    let by_file = |fused: &[SearchHit], f: &str| fused.iter().find(|h| h.file == f).unwrap().score;
    assert_eq!(by_file(&fused, "a.rs"), 1.0 / 61.0 + 1.0 / 62.0);
    assert_eq!(by_file(&fused, "b.rs"), 1.0 / 62.0 + 1.0 / 61.0);
    assert_eq!(by_file(&fused, "a.rs"), by_file(&fused, "b.rs"));
    assert_eq!(n4_files(&n4_ranked(fused)), vec!["a.rs", "b.rs"]);

    // Lex-heavy (lex 2.0, def 0.25): A - B = (2-0.25)*(1/61-1/62)
    // = 1.75/3782 ~= 4.63e-4 > 0, strict A first.
    let lex_heavy = ChannelWeights {
        lexical: 2.0,
        def: 0.25,
        ..ChannelWeights::default()
    };
    let fused = n4_pipeline(&parsed, n4_crossover_corpus(), &lex_heavy);
    assert_eq!(
        by_file(&fused, "a.rs"),
        2.0 * (1.0 / 61.0) + 0.25 * (1.0 / 62.0)
    );
    assert_eq!(
        by_file(&fused, "b.rs"),
        2.0 * (1.0 / 62.0) + 0.25 * (1.0 / 61.0)
    );
    assert!(by_file(&fused, "a.rs") > by_file(&fused, "b.rs"));
    assert_eq!(n4_files(&n4_ranked(fused)), vec!["a.rs", "b.rs"]);

    // Def-heavy (lex 0.25, def 2.0): mirror image, B wins by the same
    // 1.75/3782 delta — the tilt flips the order the other way.
    let def_heavy = ChannelWeights {
        lexical: 0.25,
        def: 2.0,
        ..ChannelWeights::default()
    };
    let fused = n4_pipeline(&parsed, n4_crossover_corpus(), &def_heavy);
    assert_eq!(
        by_file(&fused, "a.rs"),
        0.25 * (1.0 / 61.0) + 2.0 * (1.0 / 62.0)
    );
    assert_eq!(
        by_file(&fused, "b.rs"),
        0.25 * (1.0 / 62.0) + 2.0 * (1.0 / 61.0)
    );
    assert!(by_file(&fused, "b.rs") > by_file(&fused, "a.rs"));
    assert_eq!(n4_files(&n4_ranked(fused)), vec!["b.rs", "a.rs"]);
}

// ---------------------------------------------------------------------------
// 3. Routing CREATES ties from 100x-different raws; keys break them
// ---------------------------------------------------------------------------

fn n4_tie_corpus() -> Vec<SearchHit> {
    // Def raws 13/26/130/1300 (symbol "foo", ceiling 13) all route to 1.0
    // (1.0, then 2/10/100 clamped); Caller raws 11.5/115 (callee "foo",
    // ceiling 11.5) both route to 1.0. Within-channel ranks fall back to
    // (file, line): def a=0 b=1 c=2 d=3; caller a=0 b=1.
    let mut defs = Vec::new();
    for (file, raw) in [("a.rs", 13.0), ("b.rs", 26.0), ("c.rs", 130.0), ("d.rs", 1300.0)] {
        let mut hit = n4_hit(HitKind::Def, file, 1, raw);
        hit.symbol = Some("foo".to_string());
        defs.push(hit);
    }
    let mut a_caller = n4_hit(HitKind::Caller, "a.rs", 1, 11.5);
    a_caller.callee = Some("foo".to_string());
    let mut b_caller = n4_hit(HitKind::Caller, "b.rs", 1, 115.0);
    b_caller.callee = Some("foo".to_string());
    defs.push(a_caller);
    defs.push(b_caller);
    defs
}

#[test]
fn routing_clamp_creates_ties_with_key_breaks() {
    // Fused (unit): a = 1/61+1/61 (def0+caller0), b = 1/62+1/62,
    // c = 1/63, d = 1/64. Order a > b > c > d is total: every adjacent
    // gap is >= 1/63-1/64 ~= 2.5e-4, ~1e11 ulps at this magnitude.
    let parsed = ParsedQuery::parse("foo");
    let fused = n4_pipeline(&parsed, n4_tie_corpus(), &ChannelWeights::default());
    assert_eq!(fused.len(), 4);
    let by_file = |f: &str| fused.iter().find(|h| h.file == f).unwrap();
    assert_eq!(by_file("a.rs").score, 1.0 / 61.0 + 1.0 / 61.0);
    assert_eq!(by_file("b.rs").score, 1.0 / 62.0 + 1.0 / 62.0);
    assert_eq!(by_file("c.rs").score, 1.0 / 63.0);
    assert_eq!(by_file("d.rs").score, 1.0 / 64.0);
    assert_eq!(by_file("a.rs").kind, HitKind::Def);
    assert_eq!(by_file("a.rs").contributors, vec![HitKind::Def, HitKind::Caller]);
    let ranked = n4_ranked(fused);
    assert_eq!(n4_files(&ranked), vec!["a.rs", "b.rs", "c.rs", "d.rs"]);

    // The tie order comes from keys, not input order: reversing the input
    // reproduces the fused stream bit-for-bit (emission is sorted by key).
    let forward: Vec<_> = n4_pipeline(&parsed, n4_tie_corpus(), &ChannelWeights::default())
        .iter()
        .map(n4_key)
        .collect();
    let mut reversed = n4_tie_corpus();
    reversed.reverse();
    let backward: Vec<_> = n4_pipeline(&parsed, reversed, &ChannelWeights::default())
        .iter()
        .map(n4_key)
        .collect();
    assert_eq!(forward, backward);
}

// ---------------------------------------------------------------------------
// 4. Hostile weights fuse on the sanitize rails with a hand-computed order
// ---------------------------------------------------------------------------

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
    let mut def = n4_hit(HitKind::Def, "b_def.rs", 1, 13.0);
    def.symbol = Some("foo".to_string());
    let mut caller = n4_hit(HitKind::Caller, "c_caller.rs", 1, 11.5);
    caller.callee = Some("foo".to_string());
    let hits = vec![
        n4_hit(HitKind::Asgrep, "a_lex.rs", 1, ceiling),
        def,
        caller,
        n4_hit(HitKind::Graph, "d_graph.rs", 1, 5.0),
        n4_hit(HitKind::Anchor, "e_anchor.rs", 1, 6.0),
        n4_hit(HitKind::Embed, "f_embed.rs", 1, 4.0),
        n4_hit(HitKind::Pattern, "g_pattern.rs", 1, 7.0),
        n4_hit(HitKind::Import, "h_import.rs", 1, 2.0),
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
    let fused = n4_pipeline(&parsed, hits, &hostile);
    assert_eq!(fused.len(), 8);
    let by_file = |f: &str| fused.iter().find(|h| h.file == f).unwrap().score;
    assert_eq!(by_file("c_caller.rs"), 2.0 * (1.0 / 61.0));
    assert_eq!(by_file("h_import.rs"), 2.0 * (1.0 / 61.0));
    assert_eq!(by_file("d_graph.rs"), 1.0 / 61.0);
    assert_eq!(by_file("e_anchor.rs"), 1.0 / 61.0);
    for f in ["a_lex.rs", "b_def.rs", "f_embed.rs", "g_pattern.rs"] {
        assert_eq!(by_file(f), 0.25 * (1.0 / 61.0), "{f} must fuse on the 0.25 rail");
    }
    // Score-desc ranking with file tiebreaks inside each rail cohort.
    assert_eq!(
        n4_files(&n4_ranked(fused)),
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

// ---------------------------------------------------------------------------
// 5. Empty / zeroed channels vanish; the surviving channel fuses alone
// ---------------------------------------------------------------------------

#[test]
fn empty_and_zeroed_channels_vanish() {
    // Only Embed carries signal: raws 4.0/2.0 -> routed 1.0/0.5 -> ranks
    // 0/1 -> 1/61, 1/62. The Asgrep 0.0 and Def 0.0 hits route to exactly
    // 0.0 and the Graph -3.0 clamps to 0.0, so the fuse gate (> 0.0) drops
    // all three before ranking; the other channels are simply absent.
    let parsed = ParsedQuery::parse("foo");
    let mut zeroed_def = n4_hit(HitKind::Def, "z2.rs", 1, 0.0);
    zeroed_def.symbol = Some("foo".to_string());
    let fused = n4_pipeline(
        &parsed,
        vec![
            n4_hit(HitKind::Embed, "e1.rs", 1, 4.0),
            n4_hit(HitKind::Embed, "e2.rs", 1, 2.0),
            n4_hit(HitKind::Asgrep, "z1.rs", 1, 0.0),
            zeroed_def,
            n4_hit(HitKind::Graph, "z3.rs", 1, -3.0),
        ],
        &ChannelWeights::default(),
    );
    assert_eq!(fused.len(), 2);
    let by_file = |fused: &[SearchHit], f: &str| fused.iter().find(|h| h.file == f).unwrap().score;
    assert_eq!(by_file(&fused, "e1.rs"), 1.0 / 61.0);
    assert_eq!(by_file(&fused, "e2.rs"), 1.0 / 62.0);
    assert_eq!(n4_files(&n4_ranked(fused)), vec!["e1.rs", "e2.rs"]);

    // Empty-terms query: all text channels (asgrep/def/graph/...) route to
    // 0.0 and vanish in fusion; the Embed 2.0/4 = 0.5 survivor is the sole
    // hit in its channel -> rank 0 -> exactly 1/61.
    let empty = ParsedQuery::parse("");
    assert!(empty.terms.is_empty());
    let mut def = n4_hit(HitKind::Def, "t2.rs", 1, 99.0);
    def.symbol = Some("foo".to_string());
    let fused = n4_pipeline(
        &empty,
        vec![
            n4_hit(HitKind::Asgrep, "t1.rs", 1, 99.0),
            def,
            n4_hit(HitKind::Graph, "t3.rs", 1, 99.0),
            n4_hit(HitKind::Embed, "k.rs", 1, 2.0),
        ],
        &ChannelWeights::default(),
    );
    assert_eq!(fused.len(), 1);
    assert_eq!(fused[0].file, "k.rs");
    assert_eq!(fused[0].score, 1.0 / 61.0);
    assert_eq!(fused[0].contributors, vec![HitKind::Embed]);
}

// ---------------------------------------------------------------------------
// 6. Genuine producer scores INVERT through per-hit ceilings
// ---------------------------------------------------------------------------

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
    assert_eq!(parsed.terms, vec!["aa".to_string(), "bb".to_string(), "cc".to_string()]);
    assert_eq!(parsed.identifier_spelling(), None);
    let raw_x = score_def(&parsed.terms, "xx aa yy bb zz cc");
    let raw_y = score_def(&parsed.terms, "aa");
    assert_eq!(raw_x, 15.0);
    assert_eq!(raw_y, 13.0);
    assert!(raw_x > raw_y, "X must lead on raw producer score");

    let mut x = n4_hit(HitKind::Def, "x_inv.rs", 1, raw_x);
    x.symbol = Some("xx aa yy bb zz cc".to_string());
    let mut y = n4_hit(HitKind::Def, "y_inv.rs", 1, raw_y);
    y.symbol = Some("aa".to_string());
    let mut routed = vec![x, y];
    route_hits(&parsed, &mut routed);
    assert_eq!(routed[0].score, 15.0 / 33.0);
    assert_eq!(routed[1].score, 1.0);
    assert!(routed[1].score > routed[0].score, "routing must invert the pair");

    // Fusion cements the inversion: Y rank 0 -> 1/61, X rank 1 -> 1/62.
    let mut fused = routed;
    apply_weighted_rrf(&mut fused, &ChannelWeights::default());
    assert_eq!(fused.len(), 2);
    let by_file = |f: &str| fused.iter().find(|h| h.file == f).unwrap().score;
    assert_eq!(by_file("y_inv.rs"), 1.0 / 61.0);
    assert_eq!(by_file("x_inv.rs"), 1.0 / 62.0);
    assert_eq!(n4_files(&n4_ranked(fused)), vec!["y_inv.rs", "x_inv.rs"]);
}

// ---------------------------------------------------------------------------
// 7. Three-way same-line merge: canonical kind, contributors, breadth win
// ---------------------------------------------------------------------------

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
    let mut m_def = n4_hit(HitKind::Def, "m.rs", 7, 13.0);
    m_def.symbol = Some("foo".to_string());
    let mut m_caller = n4_hit(HitKind::Caller, "m.rs", 7, 11.5);
    m_caller.callee = Some("foo".to_string());
    let mut d_def = n4_hit(HitKind::Def, "a.rs", 1, 13.0);
    d_def.symbol = Some("foo".to_string());
    let mut c_caller = n4_hit(HitKind::Caller, "a.rs", 2, 11.5);
    c_caller.callee = Some("foo".to_string());
    let fused = n4_pipeline(
        &parsed,
        vec![
            n4_hit(HitKind::Asgrep, "m.rs", 7, 3.0),
            m_def,
            m_caller,
            d_def,
            c_caller,
            n4_hit(HitKind::Asgrep, "a.rs", 3, 3.2),
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
    let ranked = n4_ranked(fused);
    assert_eq!(ranked[0].file, "m.rs");
    assert_eq!(
        ranked.iter().map(|h| (h.file.as_str(), h.line_start)).collect::<Vec<_>>(),
        vec![("m.rs", 7), ("a.rs", 1), ("a.rs", 2), ("a.rs", 3)]
    );
}

// ---------------------------------------------------------------------------
// 8. Capstone: route -> fuse -> finish preserves scores and ranks by score
// ---------------------------------------------------------------------------

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
    let mut b_def = n4_hit(HitKind::Def, "b_struct.rs", 1, 13.0);
    b_def.symbol = Some("foo".to_string());
    let mut b_caller = n4_hit(HitKind::Caller, "b_struct.rs", 1, 11.5);
    b_caller.callee = Some("foo".to_string());
    let fused = n4_pipeline(
        &parsed,
        vec![
            n4_hit(HitKind::Asgrep, "a_exact.rs", 1, 3.0),
            b_def,
            b_caller,
            n4_hit(HitKind::Embed, "c_sem.rs", 1, 4.0),
        ],
        &ChannelWeights::default(),
    );
    assert_eq!(fused.len(), 3);
    let response = finish_response(&parsed, &n4_options(dir.path(), 10), fused, false);
    assert_eq!(response.hits.len(), 3);
    assert_eq!(
        n4_files(&response.hits),
        vec!["b_struct.rs", "a_exact.rs", "c_sem.rs"]
    );
    let by_file = |f: &str| response.hits.iter().find(|h| h.file == f).unwrap();
    assert_eq!(by_file("b_struct.rs").score, 1.0 / 61.0 + 1.0 / 61.0);
    assert_eq!(by_file("a_exact.rs").score, 1.0 / 61.0);
    assert_eq!(by_file("c_sem.rs").score, 1.0 / 61.0);
    for hit in &response.hits {
        assert_eq!(hit.margin, 0.0, "{} must be alone in its signal group", hit.file);
    }
    // Confidence: Exact base 0.75 / Semantic base 0.35 with one contributor
    // are bit-exact; the 2-contributor Structural row needs epsilon (0.60
    // and 0.08 are inexact in binary; N1 precedent).
    assert_eq!(by_file("a_exact.rs").confidence, 0.75);
    assert_eq!(by_file("c_sem.rs").confidence, 0.35);
    assert!((by_file("b_struct.rs").confidence - 0.68).abs() < 1e-12);
}
