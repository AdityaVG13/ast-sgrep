//! Shared error-API kit for the codemode `error_api_pass{1,2,3,4}` suites.
//!
//! Wiring: each pass file declares `#[path = "error_testkit.rs"] mod error_testkit;`
//! and imports what it needs from here. The session/batch/serve builders and
//! the error-area asserts are re-exported from the testkit crate (single
//! source of truth — never duplicated here); this module holds only the
//! area-local dispatch-equivalence assert.

// Each pass target compiles this module privately and uses a different subset;
// unused helpers per target are expected, not dead code.
#![allow(dead_code, unused_imports)]

pub use ast_sgrep_testkit::{
    assert_other_preserves_cause, batch_call, batch_request,
    call_error_discriminant as discriminant, config_at, serve_lines, serve_request_line,
    session_at,
};

use ast_sgrep_codemode::tools::call_tool;
use serde_json::Value;
use std::path::Path;

/// Both dispatch surfaces must reject the same fault with the same variant.
/// session.call consumes one budget unit; tools::call_tool bypasses the
/// budget bump — the divergence is pinned, not the confusion.
///
/// Area-local: the budget-divergence coupling is specific to this
/// dispatch-equivalence relation; keep here.
pub fn assert_dispatch_equivalence(root: &Path, tool: &str, args: Value) {
    let mut via_session = session_at(root);
    let session_err = via_session
        .call(tool, args.clone())
        .expect_err("session.call must fail");
    let mut via_tool = session_at(root);
    let tool_err = call_tool(&mut via_tool, tool, args).expect_err("call_tool must fail");
    assert_eq!(
        discriminant(&session_err),
        discriminant(&tool_err),
        "tool {tool}: dispatch surfaces disagree"
    );
    assert_eq!(via_session.call_count(), 1);
    assert_eq!(via_tool.call_count(), 0);
}
