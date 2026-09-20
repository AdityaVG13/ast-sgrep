//! Consolidated oracle suite: identifier splitting, tokenization, and
//! concept expansion.
//!
//! Replaces `oracle_foundry_pass{1,2,3}.rs` tokenize legs. Expectations are
//! hand-computed; errors assert discriminants, never messages.

use ast_sgrep_embed::{expand_concepts, split_ident, tokenize};

/// INTENT: ident splitting matches the hand table (camel/snake/acronym/dash/
/// digit edges) and tokenize emits deduped sorted sets that are invariant
/// under word order and layout.
///
/// KILLS: split-boundary/case-fold mutants, per-capital-split, digit-edge
/// flip, separator-swap, min-length-filter-drop, dedup-drop, sort-drop,
/// order-dependence, layout-sensitivity.
///
/// ABSORBS: pass1::split_ident_matches_hand_splits,
/// pass2::split_ident_camel_and_acronym_tables,
/// pass1::tokenize_matches_hand_sets, pass2::tokenize_dedups_and_sorts,
/// pass3::tokenize_order_invariant_and_deterministic.
///
/// DEDUP: the zebra/apple sorted pin appeared in pass2 and pass3; the sorted
/// hand set is pinned ONCE (pass3's order-invariance leg is distinct and kept).
#[test]
fn split_and_tokenize_match_hand_tables() {
    assert_eq!(split_ident("fooBar"), vec!["foo", "bar"]);
    assert_eq!(split_ident("foo_bar"), vec!["foo", "bar"]);
    assert_eq!(split_ident("fooBAR"), vec!["foo", "bar"]);
    assert_eq!(split_ident("a-b"), vec!["a", "b"]);
    assert_eq!(split_ident("ABC"), vec!["abc"]);
    // No splittable parts: falls back to the whole ident, lowercased.
    assert_eq!(split_ident(""), vec![""]);
    assert_eq!(split_ident("__"), vec!["__"]);
    // Camel/acronym/digit edges: no per-capital splits, digits stay glued.
    assert_eq!(split_ident("HTTPStatusCode"), vec!["httpstatus", "code"]);
    assert_eq!(split_ident("refreshToken"), vec!["refresh", "token"]);
    assert_eq!(split_ident("a1B2"), vec!["a1", "b2"]);
    assert_eq!(split_ident("FooBAR"), vec!["foo", "bar"]);
    assert_eq!(split_ident("A"), vec!["a"]);

    // Hand token sets incl min-length filtering and empty.
    assert_eq!(tokenize("FooBar"), vec!["bar", "foo", "foobar"]);
    assert_eq!(tokenize("a bc"), vec!["bc"]);
    assert!(tokenize("").is_empty());
    assert!(tokenize("a b c").is_empty());
    // Deduped repeats and sorted output (HashSet order would flake this).
    assert_eq!(tokenize("foo foo"), vec!["foo"]);
    assert_eq!(tokenize("zebra apple"), vec!["apple", "zebra"]);
    assert_eq!(tokenize("ab-CD"), vec!["ab", "cd"]);
    // Order- and layout-invariance around the same hand set.
    assert_eq!(tokenize("zebra apple"), tokenize("apple zebra"));
    assert_eq!(tokenize("  zebra   apple  "), vec!["apple", "zebra"]);
    // Hand: whole lowercased plus camel parts, sorted.
    assert_eq!(tokenize("HelloWorld"), vec!["hello", "helloworld", "world"]);
    // Determinism: repeated calls agree bit-exactly (BEHAVIOR-ONLY leg).
    assert_eq!(tokenize("FooBar_baz qux"), tokenize("FooBar_baz qux"));
}

/// INTENT: concept expansion fires precise trigger groups (combine→
/// conjunction, eviction→prune/cache/stale, throttle→rate/limit/quota),
/// passes unknown queries through, only ever adds terms, and formats empty
/// as one space.
///
/// KILLS: trigger-group-drop/merge (combine stealing rrf/fusion), term-drop
/// (query tokens lost), empty-format-change.
///
/// ABSORBS: pass2::expand_concepts_trigger_precision,
/// pass3::expand_concepts_superset_and_empty.
#[test]
fn expand_concepts_triggers_and_superset() {
    // Precision: combine fires the conjunction group, not rrf/fusion.
    let query = "combine two search channels in a single query";
    let expanded = expand_concepts(query);
    let tokens = tokenize(&expanded);
    assert!(tokens.contains(&"conjunction".to_string()), "{expanded:?}");
    assert!(!tokens.contains(&"rrf".to_string()), "{expanded:?}");
    assert!(!tokens.contains(&"fusion".to_string()), "{expanded:?}");
    let evicted = expand_concepts("eviction");
    for token in ["prune", "cache", "stale"] {
        assert!(evicted.contains(token), "{evicted:?}");
    }
    assert_eq!(expand_concepts("zxqy qwerty"), "zxqy qwerty qwerty zxqy");
    // Empty query: tokens and parts are empty, so the format is one space.
    assert_eq!(expand_concepts(""), " ");
    // Expansion only ADDS terms: every query token survives in the output.
    for query in [
        "throttle inbound",
        "zxqy qwerty",
        "remember query embeddings",
    ] {
        let expanded = expand_concepts(query);
        for token in tokenize(query) {
            assert!(
                tokenize(&expanded).contains(&token),
                "lost {token:?} in {expanded:?}"
            );
        }
        assert_eq!(expand_concepts(query), expand_concepts(query));
    }
    // Hand trigger: "throttle" fires the rate group terms.
    let expanded = expand_concepts("throttle inbound");
    for token in ["rate", "limit", "quota"] {
        assert!(expanded.contains(token), "{expanded:?}");
    }
}
