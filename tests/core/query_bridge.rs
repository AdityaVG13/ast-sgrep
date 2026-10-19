//! L-Q3 fixture: which parsed form feeds the hybrid embed lane.
//!
//! INTENT (pre-declared 2026-10-17, plan Phase 5 / conformance ledger):
//! the static engine embeds the query AS WRITTEN (measured: expanded
//! embeds lost a structurally exact target). `ASGREP_QEMBED_EXPANDED=1`
//! arms the repo-PPMI-expanded form — a paired frozen-suite measurement
//! switch, not a default.
//!
//! KILLS:
//! - default-flip mutant (armed by default): `unset` case must return
//!   the written form;
//! - flag-value confusion (`is_some()` instead of `== "1"`): `0`, `true`,
//!   and empty must NOT arm;
//! - clone-instead-of-borrow (identity via pointer equality).
use ast_sgrep_core::query::ParsedQuery;
use ast_sgrep_core::search::{choose_embed_query, embed_query_expansion_enabled};

#[test]
fn embed_query_expansion_switch_selects_written_form_by_default() {
    let written = ParsedQuery::parse("convert an object into json");
    let expanded = ParsedQuery::parse("convert an object into json serialize dumps");

    // Flag parsing: only the exact armed value counts.
    assert!(!embed_query_expansion_enabled(None));
    assert!(!embed_query_expansion_enabled(Some("")));
    assert!(!embed_query_expansion_enabled(Some("0")));
    assert!(!embed_query_expansion_enabled(Some("true")));
    assert!(embed_query_expansion_enabled(Some("1")));

    // Default (unarmed): the written form, by identity, for every
    // non-arming flag value.
    for env in [None, Some(""), Some("0"), Some("true")] {
        let q = choose_embed_query(
            &written,
            Some(&expanded),
            embed_query_expansion_enabled(env),
        );
        assert!(
            std::ptr::eq(q, &written),
            "default must embed as written ({env:?})"
        );
    }

    // Armed: the expanded form, by identity.
    let q = choose_embed_query(
        &written,
        Some(&expanded),
        embed_query_expansion_enabled(Some("1")),
    );
    assert!(
        std::ptr::eq(q, &expanded),
        "armed run must embed the expanded form"
    );

    // Armed but the repo has no expansion: fall back to the written form.
    let q = choose_embed_query(&written, None, true);
    assert!(
        std::ptr::eq(q, &written),
        "no expansion must not arm a phantom"
    );
}
