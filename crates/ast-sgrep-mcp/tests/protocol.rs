use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
/// Locate asgrep-mcp. `env!(CARGO_BIN_EXE_asgrep-mcp)` unavailable when workspace rustc-wrapper is a shell script.
fn mcp_bin() -> PathBuf {
    if let Some(p) = option_env!("CARGO_BIN_EXE_asgrep-mcp") {
        return PathBuf::from(p);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(profile)
        .join("asgrep-mcp")
}
fn rpc(payload: Value) -> Value {
    rpc_at(payload, None)
}
fn rpc_at(payload: Value, root: Option<&std::path::Path>) -> Value {
    let mut command = Command::new(mcp_bin());
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    if let Some(root) = root {
        command.env("ASGREP_ROOT", root);
    }
    let mut child = command.spawn().expect("spawn MCP");
    writeln!(child.stdin.take().unwrap(), "{payload}").unwrap();
    let out = child.wait_with_output().expect("wait MCP");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("JSON-RPC")
}
#[test]
fn initialize_returns_protocol_and_tools_capability() {
    let r = rpc(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}));
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert_eq!(r["result"]["serverInfo"]["name"], "ast-sgrep");
    assert!(r.get("error").is_none());
}
#[test]
fn tools_list_exposes_search_and_index_tools() {
    let r = rpc(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    assert_eq!(r["id"], 2);
    let names: Vec<_> = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            "keyword_search",
            "ast_search",
            "semantic_search",
            "code_search",
            "code_read",
            "index_status",
            "index_repo",
        ]
    );
}
#[test]
fn hierarchical_searches_return_snippets_and_ids_without_auto_fusion() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        source.join("lib.rs"),
        "fn target_symbol() { helper(); }\nfn helper() {}\n",
    )
    .unwrap();
    ast_sgrep_core::Indexer::new(ast_sgrep_core::IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: true,
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();

    for (name, query, expected_kind) in [
        ("keyword_search", "target_symbol", "asgrep"),
        ("ast_search", "fn $NAME() { $$$BODY }", "pattern"),
        ("semantic_search", "target symbol", "embed"),
        ("code_search", "target_symbol", "asgrep"),
    ] {
        let response = rpc_at(
            json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":name,"arguments":{"query":query,"limit":8}}}),
            Some(temp.path()),
        );
        assert_eq!(response["result"]["isError"], false, "{response:#}");
        let body: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        let hits = body["hits"].as_array().unwrap();
        assert!(!hits.is_empty(), "{name}: {body:#}");
        assert!(hits.iter().all(|hit| hit["kind"] == expected_kind));
        assert!(hits.iter().all(|hit| hit["ref"].is_string()));
        assert!(hits.iter().all(|hit| hit["preview"].is_string()));
        assert!(hits.iter().all(|hit| hit.get("excerpt").is_none()));
    }
}

#[test]
fn code_read_expands_ids_with_adjacent_context() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), "line one\nline two\nline three\n").unwrap();
    let response = rpc_at(
        json!({"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"code_read","arguments":{"ids":["src/lib.rs#L2-L2"],"context_lines":1}}}),
        Some(temp.path()),
    );
    assert_eq!(response["result"]["isError"], false, "{response:#}");
    let body: Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["nodes"][0]["id"], "src/lib.rs#L2-L2");
    assert_eq!(body["nodes"][0]["lines"], json!({"start":1,"end":3}));
    assert_eq!(
        body["nodes"][0]["content"],
        "line one\nline two\nline three"
    );
}

#[test]
fn code_read_rejects_invalid_budgets_stale_ranges_and_binary_files() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("text.rs"), "one\ntwo\n").unwrap();
    std::fs::write(temp.path().join("binary.rs"), [0xff, 0xfe, 0x00]).unwrap();
    let bounded = rpc_at(
        json!({"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"code_read","arguments":{"ids":["text.rs#L1-L1", "text.rs#L2-L2"],"max_chars":1}}}),
        Some(temp.path()),
    );
    assert_eq!(bounded["result"]["isError"], false, "{bounded:#}");
    let bounded: Value =
        serde_json::from_str(bounded["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let chars: usize = bounded["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["content"].as_str().unwrap().chars().count())
        .sum();
    assert!(chars <= 1);

    for arguments in [
        json!({"ids":["text.rs#L1-L99"]}),
        json!({"ids":["binary.rs#L1-L1"]}),
        json!({"ids":["../outside.rs#L1-L1"]}),
        json!({"ids":["text.rs#L01-L1"]}),
        json!({"ids":["text.rs#L4294967296-L4294967296"]}),
        json!({"ids":["text.rs#L1-L1"], "context_lines":"one"}),
        json!({"ids":["text.rs#L1-L1"], "unknown":true}),
    ] {
        let response = rpc_at(
            json!({"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"code_read","arguments":arguments}}),
            Some(temp.path()),
        );
        assert_eq!(response["result"]["isError"], true, "{response:#}");
    }
}

#[test]
fn search_tools_enforce_published_argument_schemas() {
    for arguments in [
        json!({"query":"target", "limit":0}),
        json!({"query":"target", "limit":"many"}),
        json!({"query":"", "limit":8}),
        json!({"query":"target", "root":false}),
        json!({"query":"target", "unexpected":true}),
    ] {
        let response = rpc(
            json!({"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"keyword_search","arguments":arguments}}),
        );
        assert_eq!(response["result"]["isError"], true, "{response:#}");
    }
}

#[test]
fn unknown_method_is_json_rpc_method_not_found() {
    let r = rpc(json!({"jsonrpc":"2.0","id":7,"method":"missing"}));
    assert_eq!(r["id"], 7);
    assert_eq!(r["error"]["code"], -32601);
    assert!(r.get("result").is_none());
}
#[test]
fn unknown_tool_remains_a_tool_error_result() {
    let r = rpc(
        json!({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"missing","arguments":{}}}),
    );
    assert_eq!(r["id"], 8);
    assert_eq!(r["result"]["isError"], true);
    assert!(r.get("error").is_none());
}
