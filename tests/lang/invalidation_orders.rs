//! Lang order-parity contract suite: results independent of population order.
//!
//! Contract under test: the seven append-only caches are pure functions of
//! their keys, so population order, schedule interleave, and consult order
//! are unobservable through the public API. Each permutation runs in a fresh
//! thread so thread-local slots start cold. Discriminants are line vectors
//! and verdicts, never message text.

use ast_sgrep_lang::{match_pattern, native_pattern_answerable, Language};
use ast_sgrep_testkit::{match_lines as lines, run_in_fresh_thread};

const RUST_FNS: &str = "fn alpha() {}\nfn beta() {}\nfn gamma() {}\n";

/// INTENT: per-key results identical under order permutation, schedule
/// interleave, consult-order permutation, and pairwise commutation.
/// KILLS: population-order-dependence / schedule/interleave-dependence /
/// consult-order-dependence / pair-order-noncommutativity.
/// ABSORBS: round_robin_interleave_matches_batched_schedule,
/// answerability_panel_permutation_invariant, pairwise_commutativity_in_fresh_threads.
#[test]
fn panel_order_permutations_agree_across_fresh_threads() {
    // Leg 1 (anchor): same panel under forward/reverse/rotated orders agrees.
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
    assert_eq!(forward[0], vec![1]);
    assert_eq!(forward[1], vec![2]);
    assert_eq!(forward[2], vec![3]);
    assert!(!forward[3].is_empty());

    // Leg 2 (schedule interleave): batched vs round-robin over the
    // (language, pattern) matrix agree per cell.
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
    let order = [0usize, 3, 1, 4, 2, 5];
    let interleaved = run_in_fresh_thread(move || {
        let mut back = vec![Vec::new(); cells.len()];
        for cell in order.iter() {
            let (l, src, p) = cells[*cell];
            back[*cell] = lines(&match_pattern(l, src, p).unwrap());
        }
        back
    });
    assert_eq!(batched, interleaved);
    assert_eq!(batched[0], vec![1]);
    assert_eq!(batched[1], vec![1]);
    assert_eq!(batched[2], vec![1]);

    // Leg 3 (consult order): three permutations of the answerability panel
    // agree per cell.
    let answer_panel = [
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
    let fwd: Vec<usize> = (0..answer_panel.len()).collect();
    let rev: Vec<usize> = (0..answer_panel.len()).rev().collect();
    let shuffled = vec![3usize, 0, 5, 1, 4, 2];
    let a = run_in_fresh_thread(move || probe(answer_panel, &fwd));
    let b = run_in_fresh_thread(move || probe(answer_panel, &rev));
    let c = run_in_fresh_thread(move || probe(answer_panel, &shuffled));
    assert_eq!(a, b);
    assert_eq!(a, c);
    assert!(a[2]);
    assert!(!a[3]);

    // Leg 4 (pairwise commutativity): A-then-B == B-then-A per key for
    // template-sharing pairs.
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
