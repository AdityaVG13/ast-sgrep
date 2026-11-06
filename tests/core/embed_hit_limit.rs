//! F5: the embed hit-limit stanza lives in one helper.
//!
//! KILLS: the `EMBED_HIT_LIMIT.max(options.retained_limit())` copies spread
//! across every embed pass (S1). Fails to compile before F5 (no such helper
//! exists); passes after. Pins the rule the stanza encodes: floor 50,
//! honour larger retained limits.

use ast_sgrep_core::search::passes::embed::embed_hit_limit;
use ast_sgrep_core::SearchOptions;

fn opts_with_limit(limit: usize) -> SearchOptions {
    SearchOptions {
        limit,
        ..SearchOptions::default()
    }
}

#[test]
fn hit_limit_floors_at_fifty_and_honours_larger_limits() {
    assert_eq!(embed_hit_limit(&opts_with_limit(0)), 50);
    assert_eq!(embed_hit_limit(&opts_with_limit(1)), 50);
    assert_eq!(embed_hit_limit(&opts_with_limit(16)), 50);
    assert_eq!(embed_hit_limit(&opts_with_limit(50)), 50);
    assert_eq!(embed_hit_limit(&opts_with_limit(51)), 51);
    assert_eq!(embed_hit_limit(&opts_with_limit(200)), 200);
}
