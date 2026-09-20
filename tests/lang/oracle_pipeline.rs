//! Consolidated oracle suite: end-to-end pipelines through public APIs on
//! real source fixtures (pattern → match → extract → score → rank).
//!
//! Replaces `oracle_foundry_pass4.rs`. Expectations are hand-computed; errors
//! assert discriminants (`is_err`/`is_ok`/emptiness), never message text.

use ast_sgrep_embed::{dot_similarity, top_by_similarity, SemanticLocalEmbedding};
use ast_sgrep_lang::{
    detect_language, match_pattern, required_pattern_literal, Language, ParserRegistry,
};
use ast_sgrep_testkit::fold_rank_scored;
use std::collections::BTreeSet;
use std::path::Path;

/// INTENT: a Rust decl pattern yields 2 hits whose excerpts score finite,
/// rank in the independent-fold order, and reproduce the full
/// extract→match→score→rank pipeline bit-identically across 3 runs.
///
/// KILLS: pipeline-wiring (match→embed→rank), ranker-vs-fold divergence,
/// cross-run nondeterminism in extract/match/rank.
///
/// ABSORBS: pass4::e2e_rust_decl_pattern_to_ranked_excerpts,
/// pass4::e2e_pipeline_deterministic_under_repetition (BEHAVIOR-ONLY
/// repetition leg: the only full-pipeline determinism proof).
#[test]
fn e2e_decl_pattern_to_ranked_excerpts_deterministically() {
    let source =
        "fn greet() {\n    println!(\"hi\");\n}\nfn process_request() {\n    greet();\n}\n";
    let hits = match_pattern(Language::Rust, source, "fn $NAME($$$)").expect("match ok");
    // Hand: two fn decls in the fixture, one hit each.
    assert_eq!(hits.len(), 2);
    for hit in &hits {
        assert!(hit.excerpt.contains("fn "), "excerpt={:?}", hit.excerpt);
    }
    // Score each excerpt against the query embedding, then rank.
    let emb = SemanticLocalEmbedding;
    let query = emb.embed_text("greet");
    let scored: Vec<(usize, f32)> = hits
        .iter()
        .enumerate()
        .map(|(i, h)| (i, dot_similarity(&query, &emb.embed_text(&h.excerpt))))
        .collect();
    assert!(scored.iter().all(|(_, s)| s.is_finite()));
    let ranked = top_by_similarity(scored.clone(), 10, None);
    assert_eq!(ranked.len(), 2);
    // The ranker must agree with the independent fold oracle on this input
    // (fold half is a BEHAVIOR-ONLY differential; the 2-hit hand count above
    // carries the wiring kill).
    let folded: Vec<(usize, f32)> = hits
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let v = emb.embed_text(&h.excerpt);
            let sum: f64 = query
                .iter()
                .zip(v.iter())
                .map(|(x, y)| f64::from(*x) * f64::from(*y))
                .sum();
            (i, sum as f32)
        })
        .collect();
    let expected_order: Vec<usize> = fold_rank_scored(&folded).iter().map(|(i, _)| *i).collect();
    let actual_order: Vec<usize> = ranked.iter().map(|(i, _)| *i).collect();
    assert_eq!(actual_order, expected_order);

    // Repetition leg: the full pipeline is bit-identical across 3 runs.
    let rep_source = "fn greet() {}\nfn process_request() { greet(); }\n";
    let registry = ParserRegistry::new();
    let rep_query = emb.embed_text("greet");
    let mut runs = Vec::new();
    for _ in 0..3 {
        let extracted = registry
            .parse(Language::Rust, rep_source)
            .expect("parse ok");
        let hits = match_pattern(Language::Rust, rep_source, "fn $NAME($$$)").expect("match ok");
        let scored: Vec<(usize, f32)> = hits
            .iter()
            .enumerate()
            .map(|(i, h)| (i, dot_similarity(&rep_query, &emb.embed_text(&h.excerpt))))
            .collect();
        let ranked = top_by_similarity(scored, 10, None);
        runs.push((extracted, hits, ranked));
    }
    assert_eq!(runs[0].0, runs[1].0);
    assert_eq!(runs[0].0, runs[2].0);
    assert_eq!(runs[0].1, runs[1].1);
    assert_eq!(runs[0].1, runs[2].1);
    assert_eq!(runs[0].2, runs[1].2);
    assert_eq!(runs[0].2, runs[2].2);
    assert!(!runs[0].1.is_empty());
}

/// INTENT: registry extraction and pattern matching agree across all three
/// cross-views: match hits cover extracted symbol spans on the same line,
/// `$NAME` captures equal the hand set and the registry names, and call-site
/// rows agree with the literal lane on caller/callee/line.
///
/// KILLS: extract/match span-divergence, prefilter-mistable, capture-drop,
/// extract/match name-divergence, call-extract wiring, line-divergence.
///
/// ABSORBS: pass4::e2e_extract_symbols_drive_prefilter_and_match,
/// pass4::e2e_captures_agree_with_extracted_symbols,
/// pass4::e2e_call_site_line_agrees_with_literal_match.
#[test]
fn e2e_extraction_and_matching_agree_on_spans_names_lines() {
    // Span view: one symbol (alpha, line 1); the prefilter literal is the
    // symbol name and a match hit covers the extracted span on the same line.
    let source = "fn alpha() {}\n";
    let registry = ParserRegistry::new();
    let extracted = registry.parse(Language::Rust, source).expect("parse ok");
    assert_eq!(extracted.symbols.len(), 1);
    let sym = &extracted.symbols[0];
    assert_eq!(sym.name, "alpha");
    assert_eq!((sym.line_start, sym.line_end), (1, 1));
    assert_eq!(
        required_pattern_literal("fn alpha($$$)"),
        Some("alpha".to_string())
    );
    assert!(source.contains(required_pattern_literal("fn alpha($$$)").unwrap().as_str()));
    let hits = match_pattern(Language::Rust, source, "fn alpha($$$)").expect("match ok");
    assert!(!hits.is_empty());
    assert!(
        hits.iter().any(|h| {
            h.byte_start <= sym.byte_start
                && h.byte_end >= sym.byte_end
                && h.line_start == sym.line_start
        }),
        "no hit covers {sym:?}: {hits:?}"
    );
    // Name view: $NAME captures equal hand {alpha,beta} and registry names.
    let source = "fn alpha() {}\nfn beta() {}\n";
    let hits = match_pattern(Language::Rust, source, "fn $NAME($$$)").expect("match ok");
    assert_eq!(hits.len(), 2);
    let captured: BTreeSet<&str> = hits
        .iter()
        .filter_map(|h| h.captures.get("NAME").map(String::as_str))
        .collect();
    let hand: BTreeSet<&str> = BTreeSet::from(["alpha", "beta"]);
    assert_eq!(captured, hand);
    let extracted = registry.parse(Language::Rust, source).expect("parse ok");
    let symbols: BTreeSet<&str> = extracted.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(symbols, hand);
    assert_eq!(captured, symbols);
    // Line view: the callee() call sits on line 2 inside caller, and the
    // literal lane hits the same line for the same callee.
    let source = "fn callee() {}\nfn caller() { callee(); }\n";
    let extracted = registry.parse(Language::Rust, source).expect("parse ok");
    assert!(
        extracted
            .calls
            .iter()
            .any(|c| { c.caller == "caller" && c.callee == "callee" && c.line == 2 }),
        "calls={:?}",
        extracted.calls
    );
    let hits = match_pattern(Language::Rust, source, "callee").expect("match ok");
    assert!(hits.iter().any(|h| h.line_start == 2), "hits={hits:?}");
}

/// INTENT: the detect→parse→match pipeline works end-to-end on rs/py/go
/// fixtures through public APIs.
///
/// KILLS: per-language detect/parse/match wiring.
///
/// ABSORBS: pass4::e2e_multilanguage_detect_parse_match.
#[test]
fn e2e_multilanguage_detect_parse_match() {
    let cases: &[(&str, Language, &str)] = &[
        ("n.rs", Language::Rust, "fn greet() {}\n"),
        ("n.py", Language::Python, "def greet():\n    pass\n"),
        ("n.go", Language::Go, "package main\nfunc greet() {}\n"),
    ];
    let registry = ParserRegistry::new();
    for (file, lang, source) in cases {
        // Detect from the path alone, then parse and match through it.
        assert_eq!(
            detect_language(Path::new(file), None),
            Some(*lang),
            "{file}"
        );
        let extracted = registry.parse(*lang, source).expect("parse ok");
        assert!(
            extracted.symbols.iter().any(|s| s.name == "greet"),
            "{file}: {:?}",
            extracted.symbols
        );
        let hits = match_pattern(*lang, source, "greet").expect("match ok");
        assert!(!hits.is_empty(), "{file}: no hits");
        assert!(
            hits.iter().any(|h| h.excerpt.contains("greet")),
            "{file}: {hits:?}"
        );
    }
}

/// INTENT: on the shared alpha/beta fixture the exact-name query self-scores
/// 1.0 and ranks first, limit-1 keeps exactly the head, the head score as an
/// exclusive threshold keeps nothing, and a just-below-head threshold
/// partitions kept/dropped exactly.
///
/// KILLS: scoring/rank-inversion, limit-head-loss, exclusive-threshold
/// breach, partition leak/loss.
///
/// ABSORBS: pass4::e2e_symbol_names_scored_and_ranked,
/// pass4::e2e_threshold_shapes_ranked_pipeline.
///
/// DEDUP: the alpha/beta scoring setup was built twice (once per absorbed
/// test); it is built ONCE here and both assert families run on it.
#[test]
fn e2e_alpha_beta_scored_ranked_and_threshold_shaped() {
    let source = "fn alpha() {}\nfn beta() {}\n";
    let registry = ParserRegistry::new();
    let extracted = registry.parse(Language::Rust, source).expect("parse ok");
    let names: Vec<&str> = extracted.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "beta"]);
    let emb = SemanticLocalEmbedding;
    let query = emb.embed_text("alpha");
    let scored: Vec<(usize, f32)> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (i, dot_similarity(&query, &emb.embed_text(n))))
        .collect();
    // Hand: the exact match is the query embedding itself, so its
    // normalized self-similarity is 1.0 and it ranks first.
    let ranked = top_by_similarity(scored.clone(), 10, None);
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0].0, 0);
    assert!((ranked[0].1 - 1.0).abs() < 1e-5, "got {ranked:?}");
    assert!(ranked[1].1 < ranked[0].1, "got {ranked:?}");
    // Limit 1 keeps exactly the head of the full ranking.
    let head = ranked[0];
    assert_eq!(top_by_similarity(scored.clone(), 1, None), vec![head]);
    // The threshold is exclusive: the head score itself keeps nothing.
    assert!(top_by_similarity(scored.clone(), 10, Some(head.1)).is_empty());
    // Just below the head: the head survives, and the kept/dropped sets
    // partition exactly on the threshold (near-BEHAVIOR-ONLY restatement of
    // `>`, kept as the pipeline-level partition proof).
    let below = head.1 - 1e-3;
    let shaped = top_by_similarity(scored.clone(), 10, Some(below));
    assert!(shaped.contains(&head), "got {shaped:?}");
    for (i, s) in &scored {
        if *s > below {
            assert!(shaped.iter().any(|(j, _)| j == i), "lost {i}:{s}");
        } else {
            assert!(!shaped.iter().any(|(j, _)| j == i), "leaked {i}:{s}");
        }
    }
}

/// INTENT: empty sources parse/match/rank to honest empty (no truncation
/// flag, no ghost rows) on rs+py, and garbage sources/patterns stay total-Ok
/// and silent without ever matching everything.
///
/// KILLS: empty-panic, ghost-rows, truncation-flag-breach, garbage-panic/Err,
/// match-everything-breach.
///
/// ABSORBS: pass4::e2e_empty_source_fail_closed,
/// pass4::e2e_garbage_input_fail_closed.
#[test]
fn e2e_empty_and_garbage_inputs_fail_closed() {
    let registry = ParserRegistry::new();
    for lang in [Language::Rust, Language::Python] {
        let extracted = registry.parse(lang, "").expect("empty parse ok");
        assert!(extracted.symbols.is_empty());
        assert!(extracted.calls.is_empty());
        assert!(extracted.imports.is_empty());
        assert!(!extracted.depth_truncated);
        let hits = match_pattern(lang, "", "greet").expect("empty match ok");
        assert!(hits.is_empty());
        // Ranking over zero extracted rows is the honest empty.
        let ranked = top_by_similarity(Vec::new(), 10, None);
        assert!(ranked.is_empty());
    }
    let garbage = "{{{{(((\n\x00\x01\x02(((";
    let real = "fn greet() {}\n";
    // Garbage source: the lenient parser stays total (Ok), never panics.
    for lang in [Language::Rust, Language::Python] {
        assert!(registry.parse(lang, garbage).is_ok());
        assert!(match_pattern(lang, garbage, "greet").is_ok());
    }
    // Garbage pattern: Ok and silent on real source, never an error, never
    // a match-everything breach.
    let hits = match_pattern(Language::Rust, real, "\x00\x01").expect("garbage pattern ok");
    assert!(hits.is_empty());
    let ws = match_pattern(Language::Rust, garbage, "   ").expect("whitespace ok");
    assert!(ws.is_empty());
}
