//! Durable checks for CodeMode wire serde used by `codemode_serve` fuzz target.

use ast_sgrep_codemode::{BatchRequest, ServeRequest};

#[test]
fn serve_request_parses_end_and_call() {
    let end: ServeRequest = serde_json::from_str(r#"{"type":"end"}"#).unwrap();
    assert!(matches!(end, ServeRequest::End));

    let call: ServeRequest =
        serde_json::from_str(r#"{"type":"call","id":"1","tool":"search","args":{}}"#).unwrap();
    assert!(matches!(call, ServeRequest::Call { .. }));
}

#[test]
fn batch_request_parses_calls() {
    let batch: BatchRequest =
        serde_json::from_str(r#"{"calls":[{"id":"1","tool":"search","args":{}}]}"#).unwrap();
    assert_eq!(batch.calls.len(), 1);
}

#[test]
fn invalid_json_is_err_not_panic() {
    assert!(serde_json::from_str::<ServeRequest>("not json").is_err());
    assert!(serde_json::from_str::<BatchRequest>("{}").is_err()); // missing calls
}
