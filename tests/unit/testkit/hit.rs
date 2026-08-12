use super::{hit_keys, HitKey};
use serde_json::json;
#[test]
fn normalizes_agent_github_and_gitlab_hit_keys() {
    let expected = HitKey {
        file: "src/main.rs".into(),
        line_start: 7,
        kind: "caller".into(),
        symbol: None,
        callee: Some("target".into()),
        caller: Some("source".into()),
    };
    let values = [
        json!({"hits": [{"file": "src/main.rs", "lines": {"start": 7}, "kind": "caller", "symbol": null, "callee": "target", "caller": "source"}]}),
        json!({"items": [{"path": "src/main.rs", "metadata": {"line_start": 7, "kind": "caller", "symbol": null, "callee": "target", "caller": "source"}}]}),
        json!({"data": [{"path": "src/main.rs", "startline": 7, "meta": {"kind": "caller", "symbol": null, "callee": "target", "caller": "source"}}]}),
    ];
    for value in values {
        assert_eq!(hit_keys(&value).expect("hit keys"), vec![expected.clone()]);
    }
}
