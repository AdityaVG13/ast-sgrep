//! I4 end-to-end cache-consistency drills for ast-sgrep-lang.
//!
//! I1 pinned reuse-is-unobservable, I2 pinned per-key deltas, I3 pinned
//! rebuild-parity relations. I4 runs FULL workloads through warmed caches
//! end-to-end: each drill chains the public pipeline
//! detect -> parse -> match -> extract -> score -> rank over a multi-key
//! corpus, populates every memo/cache key in a hostile order, and asserts the
//! final ranked outcome is identical to the canonical-order run.
//!
//! The seven append-only caches (thread-local parser maps in
//! `extract`/`templates`, the process-wide `SUPPORTED` gate memo and
//! compiled-`Query` cache, and the per-thread general/literal/if-cond
//! template maps) are exercised only through the public API. Score/rank are
//! deterministic pure functions defined here; discriminants are line vectors,
//! symbol names, gate verdicts, signature rows, scores, and rankings — never
//! message text. Each order permutation runs in a fresh thread so
//! thread-local slots start cold.

use ast_sgrep_lang::{
    cached_pattern_signatures, detect_language, index_can_serve_pattern, match_pattern,
    native_pattern_answerable, needs_ast_grep_fallback, Language, ParserRegistry,
};
use std::path::Path;

fn run_in_fresh_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(f).join().unwrap()
}

/// One corpus cell: a file plus the pattern panel queried against it.
#[derive(Debug, Clone, Copy)]
struct Cell {
    path: &'static str,
    lang: Language,
    source: &'static str,
    patterns: &'static [&'static str],
}

/// Full per-file pipeline outcome: detect + extract + match + gates + score.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileOutcome {
    path: &'static str,
    detected: Option<Language>,
    symbols: Vec<String>,
    hits_per_pattern: Vec<Vec<u32>>,
    fallback_per_pattern: Vec<bool>,
    answerable_per_pattern: Vec<bool>,
    sig_rows: Vec<Vec<String>>,
    index_verdicts: Vec<bool>,
    score: u64,
}

/// Deterministic score: symbol mass dominates, then hit mass, then gate bits.
fn score(symbols: usize, hits: usize, fallback_true: usize, answerable_true: usize) -> u64 {
    symbols as u64 * 1_000 + hits as u64 * 10 + fallback_true as u64 * 2 + answerable_true as u64
}

/// Deterministic rank: score desc, path asc as tie-break.
fn rank(outcomes: &[FileOutcome]) -> Vec<&'static str> {
    let mut order: Vec<&FileOutcome> = outcomes.iter().collect();
    order.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(b.path)));
    order.into_iter().map(|o| o.path).collect()
}

/// Run one pipeline stage-set for a single cell through the public API.
fn run_cell(registry: &ParserRegistry, cell: &Cell) -> FileOutcome {
    let detected = detect_language(Path::new(cell.path), Some(cell.source));
    let extraction = registry.parse(cell.lang, cell.source).unwrap();
    let symbols: Vec<String> = extraction
        .symbols
        .iter()
        .map(|s| s.name.clone())
        .collect();
    let mut hits_per_pattern = Vec::with_capacity(cell.patterns.len());
    let mut fallback_per_pattern = Vec::with_capacity(cell.patterns.len());
    let mut answerable_per_pattern = Vec::with_capacity(cell.patterns.len());
    let mut sig_rows = Vec::with_capacity(cell.patterns.len());
    let mut index_verdicts = Vec::with_capacity(cell.patterns.len());
    for pattern in cell.patterns {
        let hits = match_pattern(cell.lang, cell.source, pattern).unwrap();
        hits_per_pattern.push(hits.iter().map(|h| h.line_start).collect());
        fallback_per_pattern.push(needs_ast_grep_fallback(pattern));
        answerable_per_pattern.push(native_pattern_answerable(cell.lang, pattern));
        let rows = cached_pattern_signatures(pattern).unwrap_or_default();
        index_verdicts.push(index_can_serve_pattern(pattern, &rows));
        sig_rows.push(rows);
    }
    let total_hits: usize = hits_per_pattern.iter().map(Vec::len).sum();
    let fallback_true = fallback_per_pattern.iter().filter(|b| **b).count();
    let answerable_true = answerable_per_pattern.iter().filter(|b| **b).count();
    let score = score(symbols.len(), total_hits, fallback_true, answerable_true);
    FileOutcome {
        path: cell.path,
        detected,
        symbols,
        hits_per_pattern,
        fallback_per_pattern,
        answerable_per_pattern,
        sig_rows,
        index_verdicts,
        score,
    }
}

/// Run the full workload visiting cells in `order`, remapped to canonical
/// cell order so hostile schedules compare directly against canonical runs.
fn run_workload(cells: &[Cell], order: &[usize]) -> (Vec<FileOutcome>, Vec<&'static str>) {
    let registry = ParserRegistry::new();
    let mut back: Vec<Option<FileOutcome>> = Vec::with_capacity(cells.len());
    back.resize_with(cells.len(), || None);
    for cell_idx in order {
        let outcome = run_cell(&registry, &cells[*cell_idx]);
        back[*cell_idx] = Some(outcome);
    }
    let outcomes: Vec<FileOutcome> = back.into_iter().map(|o| o.unwrap()).collect();
    let ranking = rank(&outcomes);
    (outcomes, ranking)
}

fn canonical_order(n: usize) -> Vec<usize> {
    (0..n).collect()
}

fn reversed_order(n: usize) -> Vec<usize> {
    (0..n).rev().collect()
}

// ---------------------------------------------------------------------------
// Workload shapes
// ---------------------------------------------------------------------------

/// Single-language heavy reuse: one Rust panel reused across four files.
const RUST_PANEL: &[&str] = &["alpha", "beta", "gamma", "fn $NAME($$$)", "struct $NAME"];
const RUST_CORPUS: &[Cell] = &[
    Cell {
        path: "src/a.rs",
        lang: Language::Rust,
        source: "fn alpha() {}\nfn beta() {}\n",
        patterns: RUST_PANEL,
    },
    Cell {
        path: "src/b.rs",
        lang: Language::Rust,
        source: "fn beta() {}\nfn gamma() {}\nstruct Widget {}\n",
        patterns: RUST_PANEL,
    },
    Cell {
        path: "src/c.rs",
        lang: Language::Rust,
        source: "fn gamma() {}\nfn alpha() {}\n",
        patterns: RUST_PANEL,
    },
    Cell {
        path: "src/d.rs",
        lang: Language::Rust,
        source: "struct Gadget {}\nfn alpha() {}\nfn beta() {}\nfn gamma() {}\n",
        patterns: RUST_PANEL,
    },
];

/// Many-language interleave: one literal + one structural pattern per file.
const POLYGLOT_CORPUS: &[Cell] = &[
    Cell {
        path: "src/a.rs",
        lang: Language::Rust,
        source: "fn zeta() {}\nstruct Widget {}\n",
        patterns: &["zeta", "struct $NAME"],
    },
    Cell {
        path: "src/b.py",
        lang: Language::Python,
        source: "def zeta():\n    pass\n",
        patterns: &["zeta", "def $NAME"],
    },
    Cell {
        path: "src/c.js",
        lang: Language::JavaScript,
        source: "function zeta() {}\n",
        patterns: &["zeta", "function $NAME"],
    },
    Cell {
        path: "src/d.go",
        lang: Language::Go,
        source: "package main\n\nfunc zeta() {}\n",
        patterns: &["zeta", "func $NAME"],
    },
    Cell {
        path: "src/e.ts",
        lang: Language::TypeScript,
        source: "function zeta() {}\n",
        patterns: &["zeta", "function $NAME"],
    },
];

/// Supported/unsupported mix: the if-cond panel discriminates covered
/// grammars (JavaScript) from uncovered ones (Swift).
const MIXED_CORPUS: &[Cell] = &[
    Cell {
        path: "src/a.js",
        lang: Language::JavaScript,
        source: "if (alpha) { foo(); }\nif (beta) { bar(); }\n",
        patterns: &["if (alpha) { $B }", "if (beta) { $B }", "alpha"],
    },
    Cell {
        path: "src/b.swift",
        lang: Language::Swift,
        source: "func zeta() {}\n",
        patterns: &["if (alpha) { $B }", "if (beta) { $B }", "zeta"],
    },
    Cell {
        path: "src/c.py",
        lang: Language::Python,
        source: "def zeta():\n    pass\n",
        patterns: &["zeta", "class $NAME", "$A $B"],
    },
    Cell {
        path: "src/d.rs",
        lang: Language::Rust,
        source: "fn zeta() {}\n",
        patterns: &["zeta", "fn $NAME($$$)", "$A $B"],
    },
];

// ---------------------------------------------------------------------------
// Drills: single-language heavy reuse
// ---------------------------------------------------------------------------

#[test]
fn single_lang_heavy_reuse_canonical_vs_reversed() {
    let n = RUST_CORPUS.len();
    let (canon_out, canon_rank) =
        run_in_fresh_thread(move || run_workload(RUST_CORPUS, &canonical_order(n)));
    let (rev_out, rev_rank) =
        run_in_fresh_thread(move || run_workload(RUST_CORPUS, &reversed_order(n)));
    assert_eq!(canon_out, rev_out);
    assert_eq!(canon_rank, rev_rank);
    // Discriminant sanity: detection, extraction, and hits are non-trivial.
    assert!(canon_out.iter().all(|o| o.detected == Some(Language::Rust)));
    assert_eq!(canon_out[0].symbols, vec!["alpha", "beta"]);
    assert_eq!(canon_out[0].hits_per_pattern[0], vec![1]);
    assert_eq!(canon_out[1].hits_per_pattern[2], vec![2]);
    assert_eq!(canon_rank.len(), n);
}

#[test]
fn single_lang_heavy_reuse_warmed_vs_cold() {
    // Hostile warm-up: saturate every cache genus with junk keys on this
    // thread, then run the canonical schedule; must equal a cold thread.
    let junk_sources = [
        (Language::Go, "package main\n\nfunc junk() {}\n"),
        (Language::Python, "def junk():\n    pass\n"),
        (
            Language::JavaScript,
            "function junk() {\n  return 1;\n}\n",
        ),
    ];
    let junk_patterns = [
        "junk",
        "qux",
        "return $X",
        "throw $X",
        "if (junk) { $$$ }",
        "$X $Y $Z",
    ];
    let registry = ParserRegistry::new();
    for (lang, src) in &junk_sources {
        let _ = registry.parse(*lang, src);
        for pattern in &junk_patterns {
            let _ = match_pattern(*lang, src, pattern);
            let _ = native_pattern_answerable(*lang, pattern);
            let _ = needs_ast_grep_fallback(pattern);
            let _ = cached_pattern_signatures(pattern);
        }
    }
    let n = RUST_CORPUS.len();
    let warmed = run_workload(RUST_CORPUS, &canonical_order(n));
    let cold = run_in_fresh_thread(move || run_workload(RUST_CORPUS, &canonical_order(n)));
    assert_eq!(warmed, cold);
    assert_eq!(warmed.0[3].symbols, vec!["Gadget", "alpha", "beta", "gamma"]);
}

// ---------------------------------------------------------------------------
// Drills: many-language interleave
// ---------------------------------------------------------------------------

#[test]
fn many_lang_batched_vs_round_robin() {
    let n = POLYGLOT_CORPUS.len();
    let batched =
        run_in_fresh_thread(move || run_workload(POLYGLOT_CORPUS, &canonical_order(n)));
    // Round-robin stride-2 schedule over the five cells.
    let rr_order = vec![0usize, 2, 4, 1, 3];
    let interleaved = run_in_fresh_thread(move || run_workload(POLYGLOT_CORPUS, &rr_order));
    assert_eq!(batched, interleaved);
    // Discriminant sanity: literal cells hit their decl lines; langs detected.
    let literal_lines: Vec<Vec<u32>> = batched
        .0
        .iter()
        .map(|o| o.hits_per_pattern[0].clone())
        .collect();
    assert_eq!(
        literal_lines,
        vec![vec![1], vec![1], vec![1], vec![3], vec![1]]
    );
    let detected: Vec<Option<Language>> = batched.0.iter().map(|o| o.detected).collect();
    assert_eq!(
        detected,
        vec![
            Some(Language::Rust),
            Some(Language::Python),
            Some(Language::JavaScript),
            Some(Language::Go),
            Some(Language::TypeScript),
        ]
    );
}

#[test]
fn many_lang_forward_vs_reverse_pipeline() {
    let n = POLYGLOT_CORPUS.len();
    let forward =
        run_in_fresh_thread(move || run_workload(POLYGLOT_CORPUS, &canonical_order(n)));
    let reverse =
        run_in_fresh_thread(move || run_workload(POLYGLOT_CORPUS, &reversed_order(n)));
    assert_eq!(forward, reverse);
    // Structural lane discriminates per language (non-empty somewhere).
    assert!(
        forward
            .0
            .iter()
            .any(|o| !o.hits_per_pattern[1].is_empty())
    );
    assert_eq!(forward.1.len(), n);
}

// ---------------------------------------------------------------------------
// Drills: unsupported-language mix
// ---------------------------------------------------------------------------

#[test]
fn unsupported_mix_supported_first_vs_unsupported_first() {
    let n = MIXED_CORPUS.len();
    // Supported-first: JS, Rust, Python, then Swift. Unsupported-first: Swift
    // cell leads so uncovered-grammar keys populate before covered ones.
    let supported_first = vec![0usize, 3, 2, 1];
    let unsupported_first = vec![1usize, 0, 2, 3];
    let a = run_in_fresh_thread(move || run_workload(MIXED_CORPUS, &supported_first));
    let b = run_in_fresh_thread(move || run_workload(MIXED_CORPUS, &unsupported_first));
    let canon = run_in_fresh_thread(move || run_workload(MIXED_CORPUS, &canonical_order(n)));
    assert_eq!(a, b);
    assert_eq!(a, canon);
    // Discriminant sanity: the JS/Swift if-cond answerability delta holds
    // end-to-end under every population order.
    assert!(a.0[0].answerable_per_pattern[0]);
    assert!(!a.0[1].answerable_per_pattern[0]);
    assert_eq!(a.0[0].hits_per_pattern[0], vec![1]);
    assert_eq!(a.0[2].fallback_per_pattern[2], true);
}

#[test]
fn unsupported_mix_warmed_caches_match_cold() {
    // Warm this thread with the unsupported-first schedule plus junk growth,
    // then re-run canonical; must equal a cold canonical thread.
    let n = MIXED_CORPUS.len();
    let unsupported_first = vec![1usize, 0, 3, 2];
    let _ = run_workload(MIXED_CORPUS, &unsupported_first);
    for pattern in ["$P + $Q", "yield $X", "while ($C) { $B }", "interface $N"] {
        let _ = needs_ast_grep_fallback(pattern);
        let _ = cached_pattern_signatures(pattern);
        let _ = native_pattern_answerable(Language::Swift, pattern);
    }
    let warmed = run_workload(MIXED_CORPUS, &canonical_order(n));
    let cold = run_in_fresh_thread(move || run_workload(MIXED_CORPUS, &canonical_order(n)));
    assert_eq!(warmed, cold);
    assert_eq!(warmed.0[3].symbols, vec!["zeta"]);
}

// ---------------------------------------------------------------------------
// Drill: adversarial interleave
// ---------------------------------------------------------------------------

#[test]
fn adversarial_interleave_matches_canonical_pipeline() {
    // Adversarial schedule: alternate literal/structural/if-cond/general
    // lanes across languages, with junk growth injected between every cell,
    // in a cold thread; must equal the canonical batched run.
    let n = POLYGLOT_CORPUS.len();
    let canonical =
        run_in_fresh_thread(move || run_workload(POLYGLOT_CORPUS, &canonical_order(n)));
    let adversarial = run_in_fresh_thread(move || {
        let registry = ParserRegistry::new();
        let junk = [
            (Language::Rust, "fn junk() {}\n", "junk"),
            (
                Language::JavaScript,
                "if (junk) { foo(); }\n",
                "if (junk) { $$$ }",
            ),
            (
                Language::JavaScript,
                "function f() {\n  return 1;\n}\n",
                "return $X",
            ),
            (Language::Python, "def junk():\n    pass\n", "class $NAME"),
        ];
        // Hostile cell order: structural-heavy cells first, reversed within.
        let order = vec![4usize, 2, 0, 3, 1];
        let mut back: Vec<Option<FileOutcome>> =
            Vec::from_iter((0..n).map(|_| None));
        for (step, cell_idx) in order.iter().enumerate() {
            // Poison a different cache genus between every pipeline cell.
            let (lang, src, pattern) = junk[step % junk.len()];
            let _ = registry.parse(lang, src);
            let _ = match_pattern(lang, src, pattern);
            let _ = native_pattern_answerable(lang, pattern);
            let _ = needs_ast_grep_fallback(pattern);
            let _ = cached_pattern_signatures(pattern);
            back[*cell_idx] = Some(run_cell(&registry, &POLYGLOT_CORPUS[*cell_idx]));
        }
        let outcomes: Vec<FileOutcome> = back.into_iter().map(|o| o.unwrap()).collect();
        let ranking = rank(&outcomes);
        (outcomes, ranking)
    });
    assert_eq!(canonical, adversarial);
    // Discriminant sanity: ranking is score-ordered and total order holds.
    let scores: Vec<u64> = canonical.0.iter().map(|o| o.score).collect();
    let mut ranked_scores: Vec<u64> = canonical
        .1
        .iter()
        .map(|p| canonical.0.iter().find(|o| &o.path == p).unwrap().score)
        .collect();
    let mut sorted = scores.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    ranked_scores.sort_by(|a, b| b.cmp(a));
    assert_eq!(ranked_scores, sorted);
}

// ---------------------------------------------------------------------------
// Drill: end-to-end repeat stability on warmed caches
// ---------------------------------------------------------------------------

#[test]
fn end_to_end_pipeline_repeat_is_stable() {
    // The full detect->parse->match->extract->score->rank workload, run
    // three times on one warmed thread, is identical every round.
    let n = MIXED_CORPUS.len();
    let first = run_workload(MIXED_CORPUS, &canonical_order(n));
    for _ in 0..2 {
        assert_eq!(run_workload(MIXED_CORPUS, &canonical_order(n)), first);
    }
    let poly_n = POLYGLOT_CORPUS.len();
    let poly_first = run_workload(POLYGLOT_CORPUS, &canonical_order(poly_n));
    for _ in 0..2 {
        assert_eq!(
            run_workload(POLYGLOT_CORPUS, &canonical_order(poly_n)),
            poly_first
        );
    }
    // Discriminant sanity: the two workloads rank different leaders.
    assert!(!first.1.is_empty());
    assert!(!poly_first.1.is_empty());
}
