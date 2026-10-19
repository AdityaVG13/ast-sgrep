//! INTENT: L-Q1c short-question boilerplate rewrite (pre-declared
//! 2026-10-17, plan Phase 5; offline sim CoSQA +.0102 MRR@10 with RepoQA
//! bit-identical). A bare natural-language question whose recognized
//! scaffolding matches (leading "how to / what is the best way to / is
//! there a way to ...", trailing "in/using/with/for python") is rewritten
//! to its stripped form: scaffolding phrases and the frozen sim stop list
//! drop, whitespace collapses. The rewrite NEVER applies to mode-prefixed
//! queries (pattern byte-fidelity contract) and never empties a query
//! (no rewrite instead).
//!
//! KILLS: rewrite-removed mutants (raw stays original), stop-list drift
//! (raw keeps a dropped token), mode-prefix leakage (literal: rewrites),
//! empty-strip panic/fallback mutants (raw of an all-scaffolding query
//! stays original).
//!
//! ABSORBS: nothing; new lever contract. The exact phrase/stop sets are
//! frozen from the sim (benchmarks scratch lq1_sim.py, 2026-10-17) —
//! changing them changes the lever and needs a new paired arm.

use ast_sgrep_core::query::ParsedQuery;

#[test]
fn short_question_rewrite_strips_only_matched_scaffolding() {
    let q = ParsedQuery::parse("How to check if a string contains a substring in python");
    assert_eq!(q.raw, "check if string contains substring");

    // Non-matching bare query: untouched, stop words and all.
    let q = ParsedQuery::parse("find the config loader");
    assert_eq!(q.raw, "find the config loader");

    // Mode-prefixed queries are byte-faithful — never rewritten.
    let q = ParsedQuery::parse("literal:how to read a file in python");
    assert_eq!(q.raw, "literal:how to read a file in python");

    // An all-scaffolding query must NOT become empty: no rewrite.
    let q = ParsedQuery::parse("how to do it in python");
    assert_eq!(q.raw, "how to do it in python");

    // Trailing-language mention alone also triggers the rewrite.
    let q = ParsedQuery::parse("sort a list of tuples by second element using python");
    assert_eq!(q.raw, "sort list tuples by second element");

    // "what is ... way" variant matches too ("way to" optional).
    let q = ParsedQuery::parse("What's the easiest way to reverse a string");
    assert_eq!(q.raw, "reverse string");
}
