//! E4 end-to-end error drills for `ast-sgrep-lang`.
//!
//! Where E1 inventories each failure site, E2 pins stage-to-stage
//! propagation, and E3 asserts cross-entry relations, E4 runs FULL
//! pipelines on hostile inputs end to end:
//!
//! ```text
//! detect -> parse/extract -> match -> score -> rank
//! ```
//!
//! Score/rank live downstream of this crate, so the drills run a small
//! deterministic in-test score+rank lane over the match hits and assert the
//! ranked output is sound: same multiset as the hits (nothing invented,
//! nothing dropped), non-increasing scores with a total tie-break, spans
//! pointing inside the source, and identical output across repetitions.
//!
//! One drill per input class (garbage, empty, unicode/BOM/NUL,
//! unsupported-language, hostile-pattern, fallback-loud, depth-breach) plus
//! one mixed hostile-corpus drill. Each drill asserts the documented
//! end-to-end outcome for its class: completion without panic, `Ok` on all
//! user-influenced input, and no silently-wrong ranked output.
//!
//! Assertions are discriminants only (`is_some` / `is_ok` / emptiness /
//! bools / equality of data values) — never message text.

use ast_sgrep_lang::{
    detect_language, match_pattern, needs_ast_grep_fallback, Language, ParserRegistry,
    PatternMatch,
};
use std::path::Path;

// --- In-test score+rank lane (deterministic downstream stand-in). ---

fn score_hit(hit: &PatternMatch) -> u64 {
    hit.excerpt.len() as u64 * 2
        + hit.captures.len() as u64
        + hit.byte_end.saturating_sub(hit.byte_start) as u64
}

fn rank_hits(mut hits: Vec<PatternMatch>) -> Vec<PatternMatch> {
    hits.sort_by(|a, b| {
        score_hit(b)
            .cmp(&score_hit(a))
            .then(a.byte_start.cmp(&b.byte_start))
            .then(a.byte_end.cmp(&b.byte_end))
            .then(a.excerpt.cmp(&b.excerpt))
    });
    hits
}

/// Ranked output must be exactly the hits, ordered: same length, same
/// multiset of hit keys, scores non-increasing, deterministic tie-break.
fn assert_rank_sound(ranked: &[PatternMatch], hits: &[PatternMatch]) {
    assert_eq!(ranked.len(), hits.len());
    let mut ranked_keys: Vec<(usize, usize, &str)> = ranked
        .iter()
        .map(|h| (h.byte_start, h.byte_end, h.excerpt.as_str()))
        .collect();
    let mut hit_keys: Vec<(usize, usize, &str)> = hits
        .iter()
        .map(|h| (h.byte_start, h.byte_end, h.excerpt.as_str()))
        .collect();
    ranked_keys.sort();
    hit_keys.sort();
    assert_eq!(ranked_keys, hit_keys);
    for pair in ranked.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let order = score_hit(b)
            .cmp(&score_hit(a))
            .then(a.byte_start.cmp(&b.byte_start))
            .then(a.byte_end.cmp(&b.byte_end))
            .then(a.excerpt.cmp(&b.excerpt));
        assert!(order != std::cmp::Ordering::Greater);
    }
}

/// Every ranked span must point inside the source it was matched against.
fn assert_spans_in_source(ranked: &[PatternMatch], source: &str) {
    for hit in ranked {
        assert!(hit.byte_start <= hit.byte_end);
        assert!(hit.byte_end <= source.len());
        assert!(source.get(hit.byte_start..hit.byte_end).is_some());
    }
}

// --- Full-pipeline runner: detect -> parse -> match -> score -> rank. ---
// Returns None exactly when detection yields None (short-circuit: no
// parse, no match, no ranked output — the documented unsupported outcome).

struct PipelineOutcome {
    depth_truncated: bool,
    rows_empty: bool,
    ranked: Vec<PatternMatch>,
}

fn run_pipeline(
    registry: &ParserRegistry,
    path: &str,
    content: &str,
    pattern: &str,
) -> Option<PipelineOutcome> {
    let lang = detect_language(Path::new(path), Some(content))?;
    let extraction = registry.parse(lang, content).ok()?;
    let hits = match_pattern(lang, content, pattern).ok()?;
    let ranked = rank_hits(hits);
    Some(PipelineOutcome {
        depth_truncated: extraction.depth_truncated,
        rows_empty: extraction.symbols.is_empty()
            && extraction.calls.is_empty()
            && extraction.imports.is_empty(),
        ranked,
    })
}

// E4-LANG-01: garbage source end to end. Detect succeeds (known
// extension), parse stays Ok with empty rows and a clear flag, match stays
// Ok, rank is empty — never a panic, never invented output.
#[test]
fn garbage_source_end_to_end_rank_empty() {
    let registry = ParserRegistry::new();
    let garbages = ["{{{{ !!!", "}}}[[[", "@@@###$$$", "\x00\x01\x02\x03", "\u{fffd}{{{"];
    for lang in Language::all() {
        let ext = lang.as_str();
        // Resolve one real extension per language via the table.
        let table_ext = Language::SOURCE_EXTENSIONS
            .iter()
            .find(|(_, l)| *l == *lang)
            .map(|(e, _)| *e)
            .unwrap_or(ext);
        let path = format!("n.{table_ext}");
        for garbage in garbages {
            let out = run_pipeline(&registry, &path, garbage, "foo($$$)");
            assert!(out.is_some(), "{lang} {garbage:?}");
            let out = out.unwrap();
            assert!(out.rows_empty, "{lang} {garbage:?}");
            assert!(!out.depth_truncated, "{lang} {garbage:?}");
            assert!(out.ranked.is_empty(), "{lang} {garbage:?}");
        }
    }
}

// E4-LANG-02: empty / whitespace-only source end to end. Same documented
// outcome as garbage: detected, Ok, empty rows, empty rank.
#[test]
fn empty_source_end_to_end_rank_empty() {
    let registry = ParserRegistry::new();
    for lang in Language::all() {
        let table_ext = Language::SOURCE_EXTENSIONS
            .iter()
            .find(|(_, l)| *l == *lang)
            .map(|(e, _)| *e)
            .unwrap_or(lang.as_str());
        let path = format!("n.{table_ext}");
        for source in ["", "   \n\t  "] {
            let out = run_pipeline(&registry, &path, source, "foo($$$)");
            assert!(out.is_some(), "{lang} {source:?}");
            let out = out.unwrap();
            assert!(out.rows_empty, "{lang} {source:?}");
            assert!(!out.depth_truncated, "{lang} {source:?}");
            assert!(out.ranked.is_empty(), "{lang} {source:?}");
        }
    }
}

// E4-LANG-03: unicode / BOM / NUL source end to end. Documented outcome is
// totality, not emptiness: detect Ok, parse Ok, match Ok, and the ranked
// output is sound (spans in-source) and deterministic across repetitions.
#[test]
fn unicode_bom_nul_source_end_to_end_sound_and_stable() {
    let registry = ParserRegistry::new();
    let sources = [
        "💥💥💥",
        "µµµ µµµ",
        "\u{feff}",
        "\x00\x00",
        "fn föö() { bår(); }",
        "💥 fn foo() {} 💥",
    ];
    for source in sources {
        let first = run_pipeline(&registry, "n.rs", source, "foo");
        assert!(first.is_some(), "{source:?}");
        let first = first.unwrap();
        assert_spans_in_source(&first.ranked, source);
        // Determinism: the whole pipeline repeated yields identical rank.
        let second = run_pipeline(&registry, "n.rs", source, "foo").unwrap();
        assert_eq!(first.ranked, second.ranked, "{source:?}");
        assert_eq!(first.rows_empty, second.rows_empty);
        assert_eq!(first.depth_truncated, second.depth_truncated);
    }
    // Same totality contract in a second grammar.
    for source in sources {
        let out = run_pipeline(&registry, "n.py", source, "foo");
        assert!(out.is_some(), "{source:?}");
        let out = out.unwrap();
        assert_spans_in_source(&out.ranked, source);
        assert_eq!(out.ranked, run_pipeline(&registry, "n.py", source, "foo").unwrap().ranked);
    }
}

// E4-LANG-04: unsupported-language inputs short-circuit with no ranked
// output. Detection yields None, so no parse, no match, and no rank stage
// ever runs — never a silent default language, never fabricated hits.
#[test]
fn unsupported_language_short_circuits_with_no_ranked_output() {
    let registry = ParserRegistry::new();
    let cases = [
        ("n.fortran", "print(1)"),
        ("q.cobol", "fn foo() {}"),
        ("x.zzz", "{{{{ garbage"),
        ("Makefile", "all: foo"),
        ("n.fortran", ""),
    ];
    for (path, content) in cases {
        assert!(detect_language(Path::new(path), Some(content)).is_none());
        assert!(run_pipeline(&registry, path, content, "foo($$$)").is_none());
        assert!(run_pipeline(&registry, path, content, "").is_none());
    }
    // Positive control: a supported sibling of the same content runs.
    assert!(run_pipeline(&registry, "n.rs", "fn foo() {}", "foo($$$)").is_some());
}

// E4-LANG-05: hostile patterns on real source end to end. Match stays Ok
// for every hostile shape, and the ranked output is sound (exactly the
// hits, ordered) and deterministic — never Err, never dropped hits.
#[test]
fn hostile_pattern_on_real_source_stays_sound() {
    let registry = ParserRegistry::new();
    let patterns = [
        "$%%%", "(((", "}}}", "a..b", "$A.$B.$C($D)", "fn fn fn", "@@@", "???", "<$A>",
        "foo(",
    ];
    let files = [
        ("n.rs", "fn foo() { foo(1); }"),
        ("n.py", "def foo():\n    foo(1)\n"),
        ("n.js", "function foo(){foo(1);}"),
    ];
    for (path, source) in files {
        for pattern in patterns {
            let first = run_pipeline(&registry, path, source, pattern);
            assert!(first.is_some(), "{path} {pattern:?}");
            let first = first.unwrap();
            assert!(!first.depth_truncated);
            // Soundness against the raw match hits.
            let lang = detect_language(Path::new(path), Some(source)).unwrap();
            let hits = match_pattern(lang, source, pattern).unwrap();
            assert_rank_sound(&first.ranked, &hits);
            assert_spans_in_source(&first.ranked, source);
            let second = run_pipeline(&registry, path, source, pattern).unwrap();
            assert_eq!(first.ranked, second.ranked, "{path} {pattern:?}");
        }
    }
}

// E4-LANG-06: fallback-loud patterns end to end. Shapes the search ingress
// refuses stay match-closed on the native lane: Ok + empty rank, never
// fabricated hits, with a native-shape control answering on the same file.
#[test]
fn fallback_loud_pattern_end_to_end_match_closed() {
    let registry = ParserRegistry::new();
    let source = "fn f() { greet(x); }";
    for pattern in ["greet($Ü)", "if ($COND) { $A; $B }"] {
        assert!(needs_ast_grep_fallback(pattern), "{pattern:?}");
        let out = run_pipeline(&registry, "n.rs", source, pattern).unwrap();
        assert!(out.ranked.is_empty(), "{pattern:?}");
        assert!(!out.depth_truncated);
    }
    // Positive control: the same file answers a native shape.
    let out = run_pipeline(&registry, "n.rs", source, "greet($$$)").unwrap();
    assert!(!out.ranked.is_empty());
}

// E4-LANG-07: depth breach end to end. The cap propagates LOUD through the
// full pipeline: extraction stays Ok with `depth_truncated` set, match
// stays Ok, and the ranked output is sound and deterministic. The shallow
// control keeps the flag clear.
#[test]
fn depth_breach_end_to_end_loud_flag_with_sound_rank() {
    let registry = ParserRegistry::new();
    let deep_src = format!("fn f() {{ let x = {}1{}; }}", "(".repeat(300), ")".repeat(300));
    let out = run_pipeline(&registry, "n.rs", &deep_src, "f").unwrap();
    assert!(out.depth_truncated);
    let lang = detect_language(Path::new("n.rs"), Some(&deep_src)).unwrap();
    let hits = match_pattern(lang, &deep_src, "f").unwrap();
    assert_rank_sound(&out.ranked, &hits);
    assert_spans_in_source(&out.ranked, &deep_src);
    assert_eq!(out.ranked, run_pipeline(&registry, "n.rs", &deep_src, "f").unwrap().ranked);
    // Shallow control: same pipeline, flag clear.
    let shallow = run_pipeline(&registry, "n.rs", "fn f() { let x = (1 + (2 * 3)); }", "f").unwrap();
    assert!(!shallow.depth_truncated);
}

// E4-LANG-08: mixed hostile-corpus drill. One pipeline run per file over a
// corpus mixing every hostile class; the corpus-level rank (files ordered
// by hit count desc, path asc) is deterministic across full runs, and the
// union of ranked hits equals the union of raw hits (no invented or
// dropped output anywhere in the corpus).
#[test]
fn mixed_hostile_corpus_drill() {
    let registry = ParserRegistry::new();
    let deep_src = format!("fn f() {{ let x = {}1{}; }}", "(".repeat(300), ")".repeat(300));
    let corpus = [
        ("a.rs", "fn foo() { foo(1); foo(2); }"),
        ("b.py", "def foo():\n    foo(1)\n"),
        ("c.js", "function foo(){foo(1);}"),
        ("empty.rs", ""),
        ("garbage.rs", "{{{{ !!! @@@"),
        ("unicode.rs", "fn föö() { bår(); } 💥"),
        ("nul.rs", "fn foo() {}\x00\x00"),
        ("deep.rs", deep_src.as_str()),
        ("skip.fortran", "fn foo() { foo(1); }"),
        ("Makefile", "all: foo"),
    ];
    let run_corpus = |pattern: &str| {
        let mut per_file: Vec<(String, Vec<PatternMatch>)> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        for (path, content) in &corpus {
            match run_pipeline(&registry, path, content, pattern) {
                Some(out) => {
                    assert_spans_in_source(&out.ranked, content);
                    per_file.push((path.to_string(), out.ranked));
                }
                None => skipped.push(path.to_string()),
            }
        }
        // Corpus rank: hit count desc, path asc (total order).
        per_file.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
        (per_file, skipped)
    };
    for pattern in ["foo($$$)", "foo"] {
        let (ranked, skipped) = run_corpus(pattern);
        // Documented per-class outcomes inside the corpus.
        assert_eq!(skipped, vec!["skip.fortran".to_string(), "Makefile".to_string()]);
        assert_eq!(ranked.len(), corpus.len() - skipped.len());
        // Empty and garbage members rank empty; valid members answer.
        for (path, hits) in &ranked {
            match path.as_str() {
                "empty.rs" | "garbage.rs" => assert!(hits.is_empty(), "{pattern} {path}"),
                "a.rs" | "b.py" | "c.js" => assert!(!hits.is_empty(), "{pattern} {path}"),
                _ => {}
            }
        }
        // No invented/dropped hits: corpus union equals raw-hit union.
        let mut ranked_total = 0usize;
        for (path, hits) in &ranked {
            ranked_total += hits.len();
            let content = corpus.iter().find(|(p, _)| p == path).unwrap().1;
            let lang = detect_language(Path::new(path), Some(content)).unwrap();
            let raw = match_pattern(lang, content, pattern).unwrap();
            assert_rank_sound(hits, &raw);
        }
        let mut raw_total = 0usize;
        for (path, content) in &corpus {
            if let Some(lang) = detect_language(Path::new(path), Some(content)) {
                raw_total += match_pattern(lang, content, pattern).unwrap().len();
            }
        }
        assert_eq!(ranked_total, raw_total);
        // Whole-corpus determinism across full runs.
        let (ranked_again, skipped_again) = run_corpus(pattern);
        assert_eq!(skipped, skipped_again);
        assert_eq!(ranked, ranked_again);
    }
}
