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
//! Score/rank live downstream of this crate, so the drills run the small
//! deterministic score+rank lane from the `ast-sgrep-testkit` pipeline kit
//! over the match hits and assert the ranked output is sound: same multiset as the
//! hits (nothing invented, nothing dropped), non-increasing scores with a
//! total tie-break, spans pointing inside the source, and identical output
//! across repetitions.
//!
//! One drill per input class (empty/garbage, unicode/BOM/NUL,
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

use ast_sgrep_testkit::{assert_rank_sound, assert_spans_in_source, run_pipeline};

/// E4-LANG-02 (+E4-LANG-01): empty / whitespace-only / garbage source end to
/// end. One parameterized drill over source classes: every class shares the
/// identical documented outcome — detected, Ok, empty rows, clear flag,
/// empty rank — never a panic, never invented output.
/// INTENT: empty/whitespace/garbage × every lang: detected, Ok, empty rows, flag clear, empty rank.
/// KILLS: empty-invented-output, garbage-invented-output, garbage-Err.
/// ABSORBS: garbage_source_end_to_end_rank_empty (folded as the garbage source-class leg; identical documented outcome).
#[test]
fn empty_and_garbage_source_end_to_end_rank_empty() {
    let registry = ParserRegistry::new();
    let classes: [(&str, &[&str]); 2] = [
        ("empty", &["", "   \n\t  "]),
        (
            "garbage",
            &["{{{{ !!!", "}}}[[[", "@@@###$$$", "\x00\x01\x02\x03", "\u{fffd}{{{"],
        ),
    ];
    for lang in Language::all() {
        let table_ext = Language::SOURCE_EXTENSIONS
            .iter()
            .find(|(_, l)| *l == *lang)
            .map(|(e, _)| *e)
            .unwrap_or(lang.as_str());
        let path = format!("n.{table_ext}");
        for (class, sources) in classes {
            for source in sources {
                let out = run_pipeline(&registry, &path, source, "foo($$$)");
                assert!(out.is_some(), "{lang} {class} {source:?}");
                let out = out.unwrap();
                assert!(out.rows_empty, "{lang} {class} {source:?}");
                assert!(!out.depth_truncated, "{lang} {class} {source:?}");
                assert!(out.ranked.is_empty(), "{lang} {class} {source:?}");
            }
        }
    }
}

// NOTE: `garbage_source_end_to_end_rank_empty` (former E4-LANG-01) was MERGED
// per the errorapi catalog into `empty_and_garbage_source_end_to_end_rank_empty`
// above: its header stated the identical documented outcome, so it survives
// as the garbage source-class leg of the one parameterized drill.

/// E4-LANG-03: unicode / BOM / NUL source end to end. Documented outcome is
/// totality, not emptiness: detect Ok, parse Ok, match Ok, and the ranked
/// output is sound (spans in-source) and deterministic across repetitions.
/// INTENT: unicode/BOM/NUL: total pipeline, spans in-source, identical repeat rank.
/// KILLS: span-out-of-source, nondeterministic-rank.
/// ABSORBS: none (drill pin; nothing merged).
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

/// E4-LANG-04: unsupported-language inputs short-circuit with no ranked
/// output. Detection yields None, so no parse, no match, and no rank stage
/// ever runs — never a silent default language, never fabricated hits.
/// INTENT: unsupported inputs short-circuit None: no parse/match/rank + control.
/// KILLS: silent-default-language, fabricated-hits.
/// ABSORBS: none (drill pin; nothing merged).
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

/// E4-LANG-05: hostile patterns on real source end to end. Match stays Ok
/// for every hostile shape, and the ranked output is sound (exactly the
/// hits, ordered) and deterministic — never Err, never dropped hits.
/// INTENT: hostile patterns on real files: match Ok, rank sound + deterministic.
/// KILLS: dropped-hits, unsound-rank.
/// ABSORBS: none (drill pin; nothing merged).
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

/// E4-LANG-06: fallback-loud patterns end to end. Shapes the search ingress
/// refuses stay match-closed on the native lane: Ok + empty rank, never
/// fabricated hits, with a native-shape control answering on the same file.
/// INTENT: fallback-loud patterns rank empty end-to-end + native control answers.
/// KILLS: fabricated-hits-on-loud-shape.
/// ABSORBS: none (drill pin; nothing merged).
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

/// E4-LANG-07: depth breach end to end. The cap propagates LOUD through the
/// full pipeline: extraction stays Ok with `depth_truncated` set, match
/// stays Ok, and the ranked output is sound and deterministic. The shallow
/// control keeps the flag clear.
/// INTENT: deep pipeline: loud flag + sound deterministic rank; shallow control clear.
/// KILLS: silent-breach, unsound-rank-on-deep.
/// ABSORBS: none (drill pin; nothing merged).
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

/// E4-LANG-08: mixed hostile-corpus drill. One pipeline run per file over a
/// corpus mixing every hostile class; the corpus-level rank (files ordered
/// by hit count desc, path asc) is deterministic across full runs, and the
/// union of ranked hits equals the union of raw hits (no invented or
/// dropped output anywhere in the corpus).
/// INTENT: corpus mixing all hostile classes: per-class outcomes, union equality, corpus-rank determinism.
/// KILLS: invented/dropped-corpus-hits, nondeterministic-corpus-rank.
/// ABSORBS: none (drill pin; nothing merged).
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
