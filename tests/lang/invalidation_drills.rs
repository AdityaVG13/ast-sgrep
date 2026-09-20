//! Lang cache-consistency drill suite: full workloads through warmed caches.
//!
//! Contract under test: the seven append-only caches are pure functions of
//! their keys, so hostile population orders, junk warm-up, and repetition
//! never change the final ranked outcome of the public detect -> parse ->
//! match -> gates -> score -> rank pipeline. Discriminants are line vectors,
//! symbol names, gate verdicts, signature rows, scores, and rankings — never
//! message text. Each order permutation runs in a fresh thread so
//! thread-local slots start cold.

use ast_sgrep_lang::{
    cached_pattern_signatures, detect_language, index_can_serve_pattern, match_pattern,
    native_pattern_answerable, needs_ast_grep_fallback, Language, ParserRegistry,
};
use ast_sgrep_testkit::run_in_fresh_thread;
use std::path::Path;

// WHY area-local: one corpus cell (file + pattern panel) for the drill
// workload harness; the drill harness is single-suite.
#[derive(Debug, Clone, Copy)]
struct Cell {
    path: &'static str,
    lang: Language,
    source: &'static str,
    patterns: &'static [&'static str],
}

// WHY area-local: full per-file pipeline outcome; the drill comparison unit.
// Single-suite drill harness.
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

// WHY area-local: deterministic in-test score (symbol mass, then hits, then
// gate bits). Single-suite drill harness.
fn score(symbols: usize, hits: usize, fallback_true: usize, answerable_true: usize) -> u64 {
    symbols as u64 * 1_000 + hits as u64 * 10 + fallback_true as u64 * 2 + answerable_true as u64
}

// WHY area-local: deterministic rank — score desc, path asc as tie-break.
// Single-suite drill harness.
fn rank(outcomes: &[FileOutcome]) -> Vec<&'static str> {
    let mut order: Vec<&FileOutcome> = outcomes.iter().collect();
    order.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(b.path)));
    order.into_iter().map(|o| o.path).collect()
}

// WHY area-local: run one pipeline stage-set for a single cell through the
// public API. Single-suite drill harness.
fn run_cell(registry: &ParserRegistry, cell: &Cell) -> FileOutcome {
    let detected = detect_language(Path::new(cell.path), Some(cell.source));
    let extraction = registry.parse(cell.lang, cell.source).unwrap();
    let symbols: Vec<String> = extraction.symbols.iter().map(|s| s.name.clone()).collect();
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

// WHY area-local: run the full workload visiting cells in `order`, remapped
// to canonical cell order so hostile schedules compare directly against
// canonical runs. Single-suite drill harness.
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

// WHY area-local: canonical / reversed visit orders for schedule-parity
// legs. Single-suite drill harness.
fn canonical_order(n: usize) -> Vec<usize> {
    (0..n).collect()
}

// WHY area-local: see `canonical_order`. Single-suite drill harness.
fn reversed_order(n: usize) -> Vec<usize> {
    (0..n).rev().collect()
}

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

/// INTENT: junk-warmed threads agree with cold threads on a match panel, on
/// the single-lang workload, and on the unsupported-mix workload.
/// KILLS: warmed-vs-cold-divergence / warmed-vs-cold-workload-divergence /
/// warmed-mixed-workload-divergence.
/// ABSORBS: single_lang_heavy_reuse_warmed_vs_cold,
/// unsupported_mix_warmed_caches_match_cold.
#[test]
fn warmed_caches_match_fresh_thread_results() {
    // Leg 1 (anchor): junk-warmed thread agrees with a cold thread on a panel.
    let panel = ["delta", "epsilon", "fn $NAME($$$)", "struct $NAME"];
    let source = "fn delta() {}\nfn epsilon() {}\nstruct Widget {}\n";
    for junk in ["zeta", "eta", "theta", "class $NAME", "$X $Y $Z"] {
        let _ = match_pattern(Language::Rust, source, junk);
        let _ = needs_ast_grep_fallback(junk);
        let _ = cached_pattern_signatures(junk);
    }
    let warmed: Vec<Vec<u32>> = panel
        .iter()
        .map(|p| {
            match_pattern(Language::Rust, source, p)
                .unwrap()
                .iter()
                .map(|h| h.line_start)
                .collect()
        })
        .collect();
    let cold = run_in_fresh_thread(move || {
        panel
            .iter()
            .map(|p| {
                match_pattern(Language::Rust, source, p)
                    .unwrap()
                    .iter()
                    .map(|h| h.line_start)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(warmed, cold);
    assert_eq!(warmed[0], vec![1]);
    assert_eq!(warmed[1], vec![2]);

    // Leg 2 (single-lang warmed): junk-saturated thread workload equals cold.
    let junk_sources = [
        (Language::Go, "package main\n\nfunc junk() {}\n"),
        (Language::Python, "def junk():\n    pass\n"),
        (Language::JavaScript, "function junk() {\n  return 1;\n}\n"),
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
    let warmed_rust = run_workload(RUST_CORPUS, &canonical_order(n));
    let cold_rust = run_in_fresh_thread(move || run_workload(RUST_CORPUS, &canonical_order(n)));
    assert_eq!(warmed_rust, cold_rust);
    assert_eq!(
        warmed_rust.0[3].symbols,
        vec!["Gadget", "alpha", "beta", "gamma"]
    );

    // Leg 3 (unsupported-mix warmed): unsupported-first + junk warm-up, then
    // canonical, equals cold canonical.
    let m = MIXED_CORPUS.len();
    let unsupported_first = vec![1usize, 0, 3, 2];
    let _ = run_workload(MIXED_CORPUS, &unsupported_first);
    for pattern in ["$P + $Q", "yield $X", "while ($C) { $B }", "interface $N"] {
        let _ = needs_ast_grep_fallback(pattern);
        let _ = cached_pattern_signatures(pattern);
        let _ = native_pattern_answerable(Language::Swift, pattern);
    }
    let warmed_mixed = run_workload(MIXED_CORPUS, &canonical_order(m));
    let cold_mixed = run_in_fresh_thread(move || run_workload(MIXED_CORPUS, &canonical_order(m)));
    assert_eq!(warmed_mixed, cold_mixed);
    assert_eq!(warmed_mixed.0[3].symbols, vec!["zeta"]);
}

/// INTENT: full detect→rank single-lang workload identical under canonical vs
/// reversed file order.
/// KILLS: file-order-dependence.
/// ABSORBS: (standalone — no merges).
#[test]
fn single_lang_heavy_reuse_canonical_vs_reversed() {
    let n = RUST_CORPUS.len();
    let (canon_out, canon_rank) =
        run_in_fresh_thread(move || run_workload(RUST_CORPUS, &canonical_order(n)));
    let (rev_out, rev_rank) =
        run_in_fresh_thread(move || run_workload(RUST_CORPUS, &reversed_order(n)));
    assert_eq!(canon_out, rev_out);
    assert_eq!(canon_rank, rev_rank);
    assert!(canon_out.iter().all(|o| o.detected == Some(Language::Rust)));
    assert_eq!(canon_out[0].symbols, vec!["alpha", "beta"]);
    assert_eq!(canon_out[0].hits_per_pattern[0], vec![1]);
    assert_eq!(canon_out[1].hits_per_pattern[2], vec![2]);
    assert_eq!(canon_rank.len(), n);
}

/// INTENT: mixed covered/uncovered workload identical under supported-first,
/// unsupported-first, and canonical orders.
/// KILLS: unsupported-first-poisoning.
/// ABSORBS: (standalone — no merges).
#[test]
fn unsupported_mix_supported_first_vs_unsupported_first() {
    let n = MIXED_CORPUS.len();
    let supported_first = vec![0usize, 3, 2, 1];
    let unsupported_first = vec![1usize, 0, 2, 3];
    let a = run_in_fresh_thread(move || run_workload(MIXED_CORPUS, &supported_first));
    let b = run_in_fresh_thread(move || run_workload(MIXED_CORPUS, &unsupported_first));
    let canon = run_in_fresh_thread(move || run_workload(MIXED_CORPUS, &canonical_order(n)));
    assert_eq!(a, b);
    assert_eq!(a, canon);
    assert!(a.0[0].answerable_per_pattern[0]);
    assert!(!a.0[1].answerable_per_pattern[0]);
    assert_eq!(a.0[0].hits_per_pattern[0], vec![1]);
    assert!(a.0[2].fallback_per_pattern[2]);
}

/// INTENT: polyglot workload identical under canonical, hostile
/// lane-alternating + junk-poisoned, round-robin, reverse, and repeated
/// schedules; every repetition round pinned by hand oracles.
/// KILLS: hostile-schedule-/-junk-poisoning-divergence /
/// polyglot-schedule-dependence / polyglot-file-order-dependence /
/// repetition-drift.
/// ABSORBS: many_lang_batched_vs_round_robin,
/// many_lang_forward_vs_reverse_pipeline,
/// end_to_end_pipeline_repeat_is_stable (repetition leg WITH hand oracles;
/// the hand-oracle-less repetition-equality-only assert is dropped).
#[test]
fn adversarial_interleave_matches_canonical_pipeline() {
    // Leg 1 (anchor): hostile lane-alternating order with per-step junk
    // genus-poisoning equals the canonical run incl ranking.
    let n = POLYGLOT_CORPUS.len();
    let canonical = run_in_fresh_thread(move || run_workload(POLYGLOT_CORPUS, &canonical_order(n)));
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
        let order = [4usize, 2, 0, 3, 1];
        let mut back: Vec<Option<FileOutcome>> = Vec::from_iter((0..n).map(|_| None));
        for (step, cell_idx) in order.iter().enumerate() {
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

    // Leg 2 (round-robin): stride-2 schedule equals batched; literal cells
    // hit decl lines, languages detected.
    let batched = run_in_fresh_thread(move || run_workload(POLYGLOT_CORPUS, &canonical_order(n)));
    let rr_order = vec![0usize, 2, 4, 1, 3];
    let interleaved = run_in_fresh_thread(move || run_workload(POLYGLOT_CORPUS, &rr_order));
    assert_eq!(batched, interleaved);
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

    // Leg 3 (forward vs reverse): polyglot workload identical; structural
    // lane discriminates per language.
    let fwd = run_in_fresh_thread(move || run_workload(POLYGLOT_CORPUS, &canonical_order(n)));
    let rev = run_in_fresh_thread(move || run_workload(POLYGLOT_CORPUS, &reversed_order(n)));
    assert_eq!(fwd, rev);
    assert!(fwd.0.iter().any(|o| !o.hits_per_pattern[1].is_empty()));
    assert_eq!(fwd.1.len(), n);

    // Leg 4 (repetition with hand oracles): three rounds on one warmed
    // thread are identical AND hand-pinned every round (symbols, lines,
    // answerability delta, fallback verdict) — no oracle-less equality-only
    // assert remains.
    let m = MIXED_CORPUS.len();
    let first = run_workload(MIXED_CORPUS, &canonical_order(m));
    let assert_mixed_oracles = |outcomes: &[FileOutcome]| {
        assert_eq!(outcomes[3].symbols, vec!["zeta"]);
        assert_eq!(outcomes[0].hits_per_pattern[0], vec![1]);
        assert_eq!(outcomes[0].hits_per_pattern[1], vec![2]);
        assert!(outcomes[0].answerable_per_pattern[0]);
        assert!(!outcomes[1].answerable_per_pattern[0]);
        assert!(outcomes[2].fallback_per_pattern[2]);
        assert!(outcomes[3].fallback_per_pattern[2]);
    };
    assert_mixed_oracles(&first.0);
    for _ in 0..2 {
        let round = run_workload(MIXED_CORPUS, &canonical_order(m));
        assert_mixed_oracles(&round.0);
        assert_eq!(round, first);
    }
    let poly_first = run_workload(POLYGLOT_CORPUS, &canonical_order(n));
    let assert_poly_oracles = |outcomes: &[FileOutcome]| {
        let lit: Vec<Vec<u32>> = outcomes
            .iter()
            .map(|o| o.hits_per_pattern[0].clone())
            .collect();
        assert_eq!(lit, vec![vec![1], vec![1], vec![1], vec![3], vec![1]]);
        assert_eq!(outcomes[0].symbols, vec!["zeta", "Widget"]);
        assert_eq!(outcomes[3].symbols, vec!["zeta"]);
    };
    assert_poly_oracles(&poly_first.0);
    for _ in 0..2 {
        let round = run_workload(POLYGLOT_CORPUS, &canonical_order(n));
        assert_poly_oracles(&round.0);
        assert_eq!(round, poly_first);
    }
    assert!(!first.1.is_empty());
    assert!(!poly_first.1.is_empty());
}
