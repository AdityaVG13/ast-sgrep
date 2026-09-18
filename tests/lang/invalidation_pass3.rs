//! I3 rebuild-parity metamorphic tests for ast-sgrep-lang memo caches.
//!
//! I1 pinned reuse-is-unobservable and I2 pinned per-key deltas. I3 asserts
//! RELATIONS over the same seven append-only caches (thread-local parser maps
//! in `extract`/`templates`, the process-wide `SUPPORTED` gate memo and
//! compiled-`Query` cache, and the per-thread general/literal/if-cond template
//! maps), through the public API only:
//!
//! - use-order independence (different population orders -> identical results);
//! - repeat-use stability (using a cached key N times == once);
//! - cross-key isolation under interleaving (round-robin == batched);
//! - cache-state equivalence (fresh thread vs warmed caches agree).
//!
//! All assertions are discriminant relations (equality of result vectors,
//! lines, verdicts) — never message text. Each order permutation runs in a
//! fresh thread so thread-local cache slots start cold.

use ast_sgrep_lang::{
    cached_pattern_signatures, index_can_serve_pattern, match_literal_pattern, match_pattern,
    native_pattern_answerable, needs_ast_grep_fallback, Language, ParserRegistry,
};

fn lines(hits: &[ast_sgrep_lang::PatternMatch]) -> Vec<u32> {
    hits.iter().map(|h| h.line_start).collect::<Vec<_>>()
}

fn symbol_names(result: &ast_sgrep_lang::ExtractionResult) -> Vec<String> {
    result
        .symbols
        .iter()
        .map(|s| s.name.clone())
        .collect::<Vec<_>>()
}

fn run_in_fresh_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(f).join().unwrap()
}

const RUST_FNS: &str = "fn alpha() {}\nfn beta() {}\nfn gamma() {}\n";

#[test]
fn panel_order_permutations_agree_across_fresh_threads() {
    // Same pattern panel, three population orders, each in a cold thread:
    // per-key results must be identical regardless of warm-up order.
    let panel = ["alpha", "beta", "gamma", "fn $NAME($$$)"];
    let query = |order: &[&str]| {
        order
            .iter()
            .map(|p| lines(&match_pattern(Language::Rust, RUST_FNS, p).unwrap()))
            .collect::<Vec<_>>()
    };
    let forward = run_in_fresh_thread({
        let panel = panel;
        move || query(&panel)
    });
    let reversed = run_in_fresh_thread({
        let mut rev = panel;
        rev.reverse();
        move || {
            let got = query(&rev);
            let mut back = got.clone();
            back.reverse();
            back
        }
    });
    let rotated = run_in_fresh_thread({
        let mut rot = panel;
        rot.rotate_left(2);
        move || {
            let got = query(&rot);
            let mut back = vec![Vec::new(); got.len()];
            for (i, slot) in got.into_iter().enumerate() {
                back[(i + 2) % panel.len()] = slot;
            }
            back
        }
    });
    assert_eq!(forward, reversed);
    assert_eq!(forward, rotated);
    // Discriminant sanity: the panel actually discriminates per key.
    assert_eq!(forward[0], vec![1]);
    assert_eq!(forward[1], vec![2]);
    assert_eq!(forward[2], vec![3]);
    assert!(!forward[3].is_empty());
}

#[test]
fn warmed_caches_match_fresh_thread_results() {
    // Cache-state equivalence: warm THIS thread's slots with unrelated growth
    // keys, then compare against a cold thread on the same panel.
    let panel = ["delta", "epsilon", "fn $NAME($$$)", "struct $NAME"];
    let source = "fn delta() {}\nfn epsilon() {}\nstruct Widget {}\n";
    for junk in ["zeta", "eta", "theta", "class $NAME", "$X $Y $Z"] {
        let _ = match_pattern(Language::Rust, source, junk);
        let _ = needs_ast_grep_fallback(junk);
        let _ = cached_pattern_signatures(junk);
    }
    let warmed: Vec<Vec<u32>> = panel
        .iter()
        .map(|p| lines(&match_pattern(Language::Rust, source, p).unwrap()))
        .collect();
    let cold = run_in_fresh_thread(move || {
        panel
            .iter()
            .map(|p| lines(&match_pattern(Language::Rust, source, p).unwrap()))
            .collect::<Vec<_>>()
    });
    assert_eq!(warmed, cold);
    assert_eq!(warmed[0], vec![1]);
    assert_eq!(warmed[1], vec![2]);
}

#[test]
fn match_pattern_repeat_n_equals_once() {
    // Repeat-use stability: 25 uses of a cached general-lane key == 1 use.
    let source = "function f() {\n  return 1;\n  return 2;\n}\n";
    let first = match_pattern(Language::JavaScript, source, "return $X").unwrap();
    assert_eq!(lines(&first), vec![2, 3]);
    for _ in 0..25 {
        assert_eq!(
            match_pattern(Language::JavaScript, source, "return $X").unwrap(),
            first
        );
    }
    // Same relation on a literal-lane key in another language.
    let lit_first = match_pattern(Language::Rust, RUST_FNS, "beta").unwrap();
    assert_eq!(lines(&lit_first), vec![2]);
    for _ in 0..25 {
        assert_eq!(
            match_pattern(Language::Rust, RUST_FNS, "beta").unwrap(),
            lit_first
        );
    }
}

#[test]
fn match_literal_repeat_n_equals_once() {
    // Repeat-use stability on the literal-lane cache: 25 uses == 1 use.
    let source = "fn alpha() {}\n";
    let first = match_literal_pattern(Language::Rust, source, "alpha").unwrap();
    assert_eq!(lines(&first), vec![1]);
    for _ in 0..25 {
        assert_eq!(
            match_literal_pattern(Language::Rust, source, "alpha").unwrap(),
            first
        );
    }
    // Empty-hit keys are cached too: repetition must stay empty, not flip.
    let empty_first = match_literal_pattern(Language::Python, "def beta():\n    pass\n", "alpha")
        .unwrap();
    assert!(empty_first.is_empty());
    for _ in 0..25 {
        assert!(
            match_literal_pattern(Language::Python, "def beta():\n    pass\n", "alpha")
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn fallback_gate_repeat_n_equals_once_panel() {
    // Repeat-use stability on the process-wide SUPPORTED gate memo: every
    // round over the panel must reproduce the first round exactly.
    let panel = [
        "class $NAME",
        "$A $B",
        "foo($$$)",
        "process_request",
        "if ($COND) { $A; $B }",
    ];
    let first: Vec<bool> = panel.iter().map(|p| needs_ast_grep_fallback(p)).collect();
    assert_eq!(first, vec![false, true, false, false, true]);
    for _ in 0..25 {
        let round: Vec<bool> = panel.iter().map(|p| needs_ast_grep_fallback(p)).collect();
        assert_eq!(round, first);
    }
}

#[test]
fn signature_helpers_repeat_n_equals_once() {
    // Repeat-use stability on the signature memo: rows and index verdicts
    // identical across 25 rounds.
    let panel = ["Gadget", "fn beta", "bar($$$)", "fn $NAME"];
    let first_rows: Vec<Vec<String>> = panel
        .iter()
        .map(|p| cached_pattern_signatures(p).unwrap())
        .collect();
    let first_verdicts: Vec<bool> = panel
        .iter()
        .zip(first_rows.iter())
        .map(|(p, rows)| index_can_serve_pattern(p, rows))
        .collect();
    assert_eq!(first_verdicts, vec![true, true, true, false]);
    for _ in 0..25 {
        let rows: Vec<Vec<String>> = panel
            .iter()
            .map(|p| cached_pattern_signatures(p).unwrap())
            .collect();
        assert_eq!(rows, first_rows);
        let verdicts: Vec<bool> = panel
            .iter()
            .zip(rows.iter())
            .map(|(p, rows)| index_can_serve_pattern(p, rows))
            .collect();
        assert_eq!(verdicts, first_verdicts);
    }
}

#[test]
fn round_robin_interleave_matches_batched_schedule() {
    // Cross-key isolation under interleaving: batched per-language schedule
    // vs round-robin schedule across the (language, pattern) matrix, each in
    // a cold thread, must yield identical per-cell results.
    let cells = [
        (Language::Rust, "fn zeta() {}\n", "zeta"),
        (Language::Python, "def zeta():\n    pass\n", "zeta"),
        (Language::JavaScript, "function zeta() {}\n", "zeta"),
        (Language::Rust, "fn zeta() {}\n", "fn $NAME($$$)"),
        (Language::Python, "def zeta():\n    pass\n", "def $NAME"),
        (
            Language::JavaScript,
            "function zeta() {}\n",
            "function $NAME",
        ),
    ];
    let batched = run_in_fresh_thread(move || {
        cells
            .iter()
            .map(|(l, src, p)| lines(&match_pattern(*l, src, p).unwrap()))
            .collect::<Vec<_>>()
    });
    // Round-robin order: 0, 3, 1, 4, 2, 5 remapped back to cell order.
    let order = [0usize, 3, 1, 4, 2, 5];
    let interleaved = run_in_fresh_thread(move || {
        let mut back = vec![Vec::new(); cells.len()];
        for (rank, cell) in order.iter().enumerate() {
            let _ = rank;
            let (l, src, p) = cells[*cell];
            back[*cell] = lines(&match_pattern(l, src, p).unwrap());
        }
        back
    });
    assert_eq!(batched, interleaved);
    // Discriminant sanity: literal cells hit line 1 in every language.
    assert_eq!(batched[0], vec![1]);
    assert_eq!(batched[1], vec![1]);
    assert_eq!(batched[2], vec![1]);
}

#[test]
fn parser_round_robin_matches_batched() {
    // Parser-map order independence: batched per-language parses vs a
    // round-robin interleave over two registries must agree per cell.
    let cells = [
        (Language::Rust, "fn alpha() {}\n"),
        (Language::Python, "def beta():\n    pass\n"),
        (Language::JavaScript, "function gamma() {}\n"),
        (Language::Go, "package main\n\nfunc delta() {}\n"),
    ];
    let registry_a = ParserRegistry::new();
    let batched: Vec<Vec<String>> = cells
        .iter()
        .map(|(l, src)| symbol_names(&registry_a.parse(*l, src).unwrap()))
        .collect();
    let registry_b = ParserRegistry::new();
    let order = [2usize, 0, 3, 1];
    let mut interleaved = vec![Vec::new(); cells.len()];
    for cell in order {
        let (l, src) = cells[cell];
        interleaved[cell] = symbol_names(&registry_b.parse(l, src).unwrap());
    }
    assert_eq!(batched, interleaved);
    assert_eq!(batched[0], vec!["alpha"]);
    assert_eq!(batched[1], vec!["beta"]);
    assert_eq!(batched[2], vec!["gamma"]);
    assert_eq!(batched[3], vec!["delta"]);
}

#[test]
fn answerability_panel_permutation_invariant() {
    // Use-order independence on the answerability consults: three fixed
    // permutations of the (language, pattern) panel agree per cell.
    let panel = [
        (Language::Rust, "fn $NAME($$$)"),
        (Language::Python, "def $NAME"),
        (Language::JavaScript, "if (alpha) { $B }"),
        (Language::Swift, "if (alpha) { $B }"),
        (Language::Rust, "struct $NAME"),
        (Language::Python, "class $NAME"),
    ];
    fn probe(panel: [(Language, &str); 6], order: &[usize]) -> Vec<bool> {
        let mut back = vec![false; panel.len()];
        for cell in order {
            let (l, p) = panel[*cell];
            back[*cell] = native_pattern_answerable(l, p);
        }
        back
    }
    let forward: Vec<usize> = (0..panel.len()).collect();
    let reverse: Vec<usize> = (0..panel.len()).rev().collect();
    let shuffled = vec![3usize, 0, 5, 1, 4, 2];
    let a = run_in_fresh_thread(move || probe(panel, &forward));
    let b = run_in_fresh_thread(move || probe(panel, &reverse));
    let c = run_in_fresh_thread(move || probe(panel, &shuffled));
    assert_eq!(a, b);
    assert_eq!(a, c);
    // Discriminant sanity: the JS/Swift if-cond delta from I2 still holds.
    assert!(a[2]);
    assert!(!a[3]);
}

#[test]
fn pairwise_commutativity_in_fresh_threads() {
    // A-then-B in a cold thread == B-then-A in a cold thread, per key, for
    // key pairs sharing template machinery.
    let pairs: Vec<(Language, &str, &str, &str)> = vec![
        (
            Language::JavaScript,
            "function f() {\n  return 1;\n  return 2;\n}\n",
            "return $X",
            "throw $X",
        ),
        (
            Language::JavaScript,
            "if (alpha) { foo(); }\nif (beta) { bar(); }\n",
            "if (alpha) { $$$ }",
            "if (beta) { $$$ }",
        ),
        (Language::Rust, RUST_FNS, "alpha", "fn $NAME($$$)"),
    ];
    for (lang, source, pat_a, pat_b) in pairs {
        let ab = run_in_fresh_thread(move || {
            let a = match_pattern(lang, source, pat_a).unwrap();
            let b = match_pattern(lang, source, pat_b).unwrap();
            (a, b)
        });
        let ba = run_in_fresh_thread(move || {
            let b = match_pattern(lang, source, pat_b).unwrap();
            let a = match_pattern(lang, source, pat_a).unwrap();
            (a, b)
        });
        assert_eq!(ab, ba, "pair ({pat_a:?}, {pat_b:?}) must commute");
    }
    // Discriminant sanity on the first pair's absolute values.
    let (ret, thr) = run_in_fresh_thread(|| {
        (
            match_pattern(
                Language::JavaScript,
                "function f() {\n  return 1;\n  return 2;\n}\n",
                "return $X",
            )
            .unwrap(),
            match_pattern(
                Language::JavaScript,
                "function f() {\n  return 1;\n  return 2;\n}\n",
                "throw $X",
            )
            .unwrap(),
        )
    });
    assert_eq!(lines(&ret), vec![2, 3]);
    assert!(thr.is_empty());
}
