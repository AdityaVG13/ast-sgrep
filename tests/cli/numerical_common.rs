//! Shared harness for the CLI numerical suites (`numerical_eval.rs`,
//! `numerical_search.rs`, `numerical_limits.rs`).
//!
//! The binary/eval/gold/envelope/human-table harness now lives in
//! `ast-sgrep-testkit` (`cli` module); the suites import it directly. This
//! file keeps only the area-local leftover below: helpers with a single
//! call site that no sibling needs.
//!
//! Included via `#[path = "numerical_common.rs"]`; it is NOT a test target.

use serde_json::Value;

// Keep file-local: rank with null (miss) treated as +inf, so "larger k
// never ranks worse" is a plain `<=` comparison. Single call site (eval);
// no sibling needs it.
pub fn rank_key(value: &Value) -> usize {
    if value.is_null() {
        usize::MAX
    } else {
        value.as_u64().unwrap() as usize
    }
}
