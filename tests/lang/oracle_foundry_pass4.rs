//! Pass 4 (oracle-foundry, Mission 2): L4 end-to-end oracles for the lang
//! surface pass 1–3 did not cover as pipelines.
//!
//! Pass 1–3 own unit-level contracts: embed math, ranker thresholds,
//! tokenizer hand-sets, concept triggers, keyword roots, literal/trivia
//! lanes, language tables, classify invariance, signature serve tables,
//! literal differential, connector carves, and adversarial vectors. This
//! pass owns FULL pipelines through public APIs on real source fixtures:
//! pattern parse -> match -> extract -> score -> rank end-to-end,
//! multi-language detect -> parse -> match, determinism under repetition,
//! empty/garbage fail-closed, and threshold shaping end-to-end.
//!
//! Expectations are hand-computed; errors assert discriminants
//! (`is_err`/`is_ok`/emptiness), never message text.

use ast_sgrep_embed::{
    dot_similarity, top_by_similarity, SemanticLocalEmbedding,
};
use ast_sgrep_lang::{
    detect_language, match_pattern, required_pattern_literal, Language, ParserRegistry,
};
use std::collections::BTreeSet;
use std::path::Path;

fn embedder() -> SemanticLocalEmbedding {
    SemanticLocalEmbedding
}

/// Independent f64-fold oracle for excerpt ranking: a deliberately different
/// accumulation from the pipeline's `dot_similarity`, with the hand rule
/// (score desc, ties by ascending index) applied explicitly.
fn fold_rank(order: &[(usize, f32)]) -> Vec<(usize, f32)> {
    let mut ranked = order.to_vec();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    ranked
}

// ─── E2E: Rust decl pattern -> match -> embed -> score -> rank ────────────────

#[test]
fn e2e_rust_decl_pattern_to_ranked_excerpts() {
    let source = "fn greet() {\n    println!(\"hi\");\n}\nfn process_request() {\n    greet();\n}\n";
    let hits = match_pattern(Language::Rust, source, "fn $NAME($$$)").expect("match ok");
    // Hand: two fn decls in the fixture, one hit each.
    assert_eq!(hits.len(), 2);
    for hit in &hits {
        assert!(hit.excerpt.contains("fn "), "excerpt={:?}", hit.excerpt);
    }
    // Score each excerpt against the query embedding, then rank.
    let emb = embedder();
    let query = emb.embed_text("greet");
    let scored: Vec<(usize, f32)> = hits
        .iter()
        .enumerate()
        .map(|(i, h)| (i, dot_similarity(&query, &emb.embed_text(&h.excerpt))))
        .collect();
    assert!(scored.iter().all(|(_, s)| s.is_finite()));
    let ranked = top_by_similarity(scored.clone(), 10, None);
    assert_eq!(ranked.len(), 2);
    // The ranker must agree with the independent fold oracle on this input.
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
    let expected_order: Vec<usize> = fold_rank(&folded).iter().map(|(i, _)| *i).collect();
    let actual_order: Vec<usize> = ranked.iter().map(|(i, _)| *i).collect();
    assert_eq!(actual_order, expected_order);
}

// ─── E2E: extract symbols -> prefilter literal -> match covers symbol ────────

#[test]
fn e2e_extract_symbols_drive_prefilter_and_match() {
    let source = "fn alpha() {}\n";
    let registry = ParserRegistry::new();
    let extracted = registry.parse(Language::Rust, source).expect("parse ok");
    // Hand: exactly one symbol, named alpha, on line 1.
    assert_eq!(extracted.symbols.len(), 1);
    let sym = &extracted.symbols[0];
    assert_eq!(sym.name, "alpha");
    assert_eq!((sym.line_start, sym.line_end), (1, 1));
    // The prefilter literal for the concrete decl face is the symbol name.
    assert_eq!(
        required_pattern_literal("fn alpha($$$)"),
        Some("alpha".to_string())
    );
    assert!(source.contains(required_pattern_literal("fn alpha($$$)").unwrap().as_str()));
    // The match hit must cover the extracted symbol span on the same line.
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
}

// ─── E2E: multi-language detect -> parse -> match ────────────────────────────

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
        assert_eq!(detect_language(Path::new(file), None), Some(*lang), "{file}");
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

// ─── E2E: extracted symbol names scored and ranked ───────────────────────────

#[test]
fn e2e_symbol_names_scored_and_ranked() {
    let source = "fn alpha() {}\nfn beta() {}\n";
    let registry = ParserRegistry::new();
    let extracted = registry.parse(Language::Rust, source).expect("parse ok");
    let names: Vec<&str> = extracted.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "beta"]);
    // Score every extracted name against the query "alpha" and rank.
    let emb = embedder();
    let query = emb.embed_text("alpha");
    let scored: Vec<(usize, f32)> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (i, dot_similarity(&query, &emb.embed_text(n))))
        .collect();
    let ranked = top_by_similarity(scored, 10, None);
    assert_eq!(ranked.len(), 2);
    // Hand: the exact match is the query embedding itself, so its
    // normalized self-similarity is 1.0 and it ranks first.
    assert_eq!(ranked[0].0, 0);
    assert!((ranked[0].1 - 1.0).abs() < 1e-5, "got {ranked:?}");
    assert!(ranked[1].1 < ranked[0].1, "got {ranked:?}");
}

// ─── E2E: threshold and limit shape the ranked pipeline ──────────────────────

#[test]
fn e2e_threshold_shapes_ranked_pipeline() {
    let source = "fn alpha() {}\nfn beta() {}\n";
    let registry = ParserRegistry::new();
    let extracted = registry.parse(Language::Rust, source).expect("parse ok");
    let emb = embedder();
    let query = emb.embed_text("alpha");
    let scored: Vec<(usize, f32)> = extracted
        .symbols
        .iter()
        .enumerate()
        .map(|(i, s)| (i, dot_similarity(&query, &emb.embed_text(&s.name))))
        .collect();
    let full = top_by_similarity(scored.clone(), 10, None);
    assert_eq!(full.len(), 2);
    let head = full[0];
    // Limit 1 keeps exactly the head of the full ranking.
    assert_eq!(top_by_similarity(scored.clone(), 1, None), vec![head]);
    // The threshold is exclusive: the head score itself keeps nothing.
    assert!(top_by_similarity(scored.clone(), 10, Some(head.1)).is_empty());
    // Just below the head: the head survives, and the kept/dropped sets
    // partition exactly on the threshold.
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

// ─── E2E: the full pipeline is deterministic under repetition ────────────────

#[test]
fn e2e_pipeline_deterministic_under_repetition() {
    let source = "fn greet() {}\nfn process_request() { greet(); }\n";
    let registry = ParserRegistry::new();
    let emb = embedder();
    let query = emb.embed_text("greet");
    let mut runs = Vec::new();
    for _ in 0..3 {
        let extracted = registry.parse(Language::Rust, source).expect("parse ok");
        let hits = match_pattern(Language::Rust, source, "fn $NAME($$$)").expect("match ok");
        let scored: Vec<(usize, f32)> = hits
            .iter()
            .enumerate()
            .map(|(i, h)| (i, dot_similarity(&query, &emb.embed_text(&h.excerpt))))
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

// ─── E2E: empty source fails closed across languages ─────────────────────────

#[test]
fn e2e_empty_source_fail_closed() {
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
}

// ─── E2E: garbage source and garbage pattern fail closed ─────────────────────

#[test]
fn e2e_garbage_input_fail_closed() {
    let garbage = "{{{{(((\n\x00\x01\x02(((";
    let real = "fn greet() {}\n";
    let registry = ParserRegistry::new();
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

// ─── E2E: match captures agree with extracted symbols ────────────────────────

#[test]
fn e2e_captures_agree_with_extracted_symbols() {
    let source = "fn alpha() {}\nfn beta() {}\n";
    let hits = match_pattern(Language::Rust, source, "fn $NAME($$$)").expect("match ok");
    assert_eq!(hits.len(), 2);
    let captured: BTreeSet<&str> = hits
        .iter()
        .filter_map(|h| h.captures.get("NAME").map(String::as_str))
        .collect();
    let hand: BTreeSet<&str> = BTreeSet::from(["alpha", "beta"]);
    assert_eq!(captured, hand);
    // The registry extraction must name the same two symbols.
    let registry = ParserRegistry::new();
    let extracted = registry.parse(Language::Rust, source).expect("parse ok");
    let symbols: BTreeSet<&str> = extracted.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(symbols, hand);
    assert_eq!(captured, symbols);
}

// ─── E2E: extracted call-site line agrees with the literal match ─────────────

#[test]
fn e2e_call_site_line_agrees_with_literal_match() {
    let source = "fn callee() {}\nfn caller() { callee(); }\n";
    let registry = ParserRegistry::new();
    let extracted = registry.parse(Language::Rust, source).expect("parse ok");
    // Hand: the callee() call sits on line 2 inside caller.
    assert!(
        extracted.calls.iter().any(|c| {
            c.caller == "caller" && c.callee == "callee" && c.line == 2
        }),
        "calls={:?}",
        extracted.calls
    );
    // The literal lane must hit the same line for the same callee.
    let hits = match_pattern(Language::Rust, source, "callee").expect("match ok");
    assert!(hits.iter().any(|h| h.line_start == 2), "hits={hits:?}");
}
