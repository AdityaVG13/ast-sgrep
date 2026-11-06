//! napi batch-budget parity with codemode's `enforce_batch_response_budget`.
//!
//! INTENT: `SessionBatchTask` budget accounting accepts/truncates at exactly
//! the same boundary as codemode `batch.rs` (`used + bytes + 1 <=
//! MAX_BATCH_VALUE_BYTES` from the envelope reserve), with identical overflow
//! accounting (`used += trunc_bytes + 1`). KILLS: envelope-reserve drift,
//! separator-rule divergence (napi accepting a result codemode would cut).
//!
//! `SessionBatchTask::compute` is unreachable from Rust (needs a live libuv
//! task), so this suite drives the shared per-result step
//! (`enforce_result_budget`) directly. Sizes derive from the live constants
//! and measured encodings — no byte literals to rot on retune.
//!
//! Link note: node symbols stay unresolved via the napi crate's build.rs
//! (tests never call Node FFI); plain `cargo test -p
//! ast-sgrep-codemode-napi` works on macOS/Linux.

use ast_sgrep_codemode::session::encoded_len;
use ast_sgrep_codemode::{MAX_BATCH_RESPONSE_BYTES, MAX_BATCH_VALUE_BYTES};
use ast_sgrep_codemode_napi::{enforce_result_budget, JsBatchCallResult};
use serde_json::json;

fn ok_result(pad: usize) -> JsBatchCallResult {
    JsBatchCallResult {
        id: "a".to_string(),
        ok: true,
        value: Some(json!({"pad": "x".repeat(pad)})),
        error: None,
    }
}

/// INTENT: the accept/truncate boundary matches codemode's separator rule,
/// and overflow accounting mirrors codemode (`+= trunc_bytes + 1`).
/// KILLS: off-by-one acceptance, overflow-ledger drift.
#[test]
fn separator_rule_matches_codemode() {
    let bytes = encoded_len(&ok_result(64)).expect("result is encodable");

    // `used + bytes + 1 == MAX`: inside on both rules — guards overcorrection.
    let mut used = MAX_BATCH_VALUE_BYTES - bytes - 1;
    let out = enforce_result_budget(ok_result(64), &mut used).expect("budget step");
    assert!(out.ok, "boundary-1 must still accept");
    assert!(out.value.is_some(), "accepted result keeps its value");
    assert_eq!(
        used,
        MAX_BATCH_VALUE_BYTES - 1,
        "accept path adds bytes only, exactly like codemode"
    );

    // `used + bytes == MAX`: codemode's `+ 1` separator no longer fits, so the
    // result must truncate with the shared overflow text.
    let mut used = MAX_BATCH_VALUE_BYTES - bytes;
    let out = enforce_result_budget(ok_result(64), &mut used).expect("budget step");
    assert!(!out.ok, "separator rule must truncate where codemode truncates");
    assert_eq!(out.value, None, "truncated result drops its value");
    let error = out.error.as_ref().expect("truncation carries the overflow text");
    assert!(
        error.contains(&MAX_BATCH_RESPONSE_BYTES.to_string()),
        "overflow text names the response ceiling: {error}"
    );
    let trunc_bytes = encoded_len(&out).expect("truncated result is encodable");
    assert_eq!(
        used,
        MAX_BATCH_VALUE_BYTES - bytes + trunc_bytes + 1,
        "overflow accounting mirrors codemode: used += trunc_bytes + 1"
    );
}
