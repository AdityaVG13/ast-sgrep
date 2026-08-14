use ast_sgrep_testkit::{assert_golden_json_at, Scrubber};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
/// Locate asgrep-mcp. `env!(CARGO_BIN_EXE_asgrep-mcp)` is unavailable when the
/// workspace rustc-wrapper is a shell script; honor `CARGO_TARGET_DIR` next.
fn mcp_bin() -> PathBuf {
    if let Some(p) = option_env!("CARGO_BIN_EXE_asgrep-mcp") {
        return PathBuf::from(p);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let exe = format!("asgrep-mcp{}", std::env::consts::EXE_SUFFIX);
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        let candidate = PathBuf::from(dir).join(profile).join(&exe);
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(profile)
        .join(exe)
}
fn rpc(payload: Value) -> Value {
    rpc_at(payload, None)
}
fn rpc_at(payload: Value, root: Option<&std::path::Path>) -> Value {
    let mut responses = rpc_session(vec![payload], root);
    responses.pop().expect("one response")
}
/// Drive several requests through ONE server process. Compact path ids (kxmc)
/// are session state, so search-then-read must share a process to be realistic.
fn rpc_session(payloads: Vec<Value>, root: Option<&std::path::Path>) -> Vec<Value> {
    rpc_session_env(payloads, root, &[])
}

fn rpc_session_env(
    payloads: Vec<Value>,
    root: Option<&std::path::Path>,
    extra_env: &[(&str, Option<&str>)],
) -> Vec<Value> {
    let mut command = Command::new(mcp_bin());
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    if let Some(root) = root {
        command.env("ASGREP_ROOT", root);
    }
    for (key, value) in extra_env {
        match value {
            Some(value) => {
                command.env(key, value);
            }
            None => {
                command.env_remove(key);
            }
        }
    }
    let mut child = command.spawn().expect("spawn MCP");
    {
        let mut stdin = child.stdin.take().unwrap();
        for payload in &payloads {
            writeln!(stdin, "{payload}").unwrap();
        }
    }
    let out = child.wait_with_output().expect("wait MCP");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf8 stdout")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("JSON-RPC"))
        .collect()
}
/// Parse the text payload of a tools/call result.
fn tool_body(response: &Value) -> Value {
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
        .expect("tool body JSON")
}
#[test]
fn initialize_returns_protocol_and_tools_capability() {
    // r2lu: a client that names no revision gets the current one.
    let r = rpc(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}));
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert_eq!(r["result"]["serverInfo"]["name"], "ast-sgrep");
    assert!(r.get("error").is_none());
}

/// r2lu: negotiation, not a hardcoded constant. An existing handshake-era
/// client must keep the revision it asked for.
#[test]
fn initialize_negotiates_the_requested_protocol_revision() {
    let legacy = rpc(json!({
        "jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{"protocolVersion":"2024-11-05"}
    }));
    assert_eq!(
        legacy["result"]["protocolVersion"], "2024-11-05",
        "legacy clients must not be forced onto a newer revision"
    );

    let current = rpc(json!({
        "jsonrpc":"2.0","id":2,"method":"initialize",
        "params":{"protocolVersion":"2025-11-25"}
    }));
    assert_eq!(current["result"]["protocolVersion"], "2025-11-25");

    // The discovery-based revision is unsupported by this handshake server and
    // must not be echoed back merely because the client requested it.
    let unknown = rpc(json!({
        "jsonrpc":"2.0","id":3,"method":"initialize",
        "params":{"protocolVersion":"2026-07-28"}
    }));
    assert_eq!(unknown["result"]["protocolVersion"], "2025-11-25");
}

/// r2lu: every search tool declares an outputSchema, and results carry typed
/// structuredContent that matches the text fallback exactly.
#[test]
fn search_results_carry_structured_content_matching_the_declared_schema() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), "fn target_symbol() {}\n").unwrap();
    ast_sgrep_core::Indexer::new(ast_sgrep_core::IndexOptions {
        root: temp.path().to_path_buf(),
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();

    let listed = rpc(json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}));
    for tool in listed["result"]["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        if name.ends_with("_search") || name == "code_search" {
            let schema = &tool["outputSchema"];
            assert_eq!(
                schema["type"], "object",
                "{name} must declare an outputSchema"
            );
            assert!(schema["properties"]["h"].is_object(), "{name} schema hits");
            assert!(schema["properties"]["p"].is_object(), "{name} schema paths");
        }
    }

    let response = rpc_at(
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"keyword_search","arguments":{"query":"target_symbol","limit":4}}}),
        Some(temp.path()),
    );
    let structured = &response["result"]["structuredContent"];
    assert!(
        structured.is_object(),
        "structuredContent missing: {response:#}"
    );
    assert_eq!(structured["v"], 1);
    assert!(structured["h"].is_array());

    // The text fallback stays, and says exactly the same thing.
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).expect("text fallback is JSON");
    assert_eq!(
        &parsed, structured,
        "text and structured content must agree"
    );
    assert!(!text.contains('\n'), "text fallback must stay minified");
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

    // kxmc: compact envelope. Hits are positional tuples
    // [id, kind, signal, symbol, snippet]; `p` maps path id to project path.
    for (name, query, expected_kind) in [
        ("keyword_search", "target_symbol", "x"),
        ("ast_search", "fn $NAME() { $$$BODY }", "p"),
        ("semantic_search", "target symbol", "e"),
        ("code_search", "target_symbol", "x"),
    ] {
        let response = rpc_at(
            json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":name,"arguments":{"query":query,"limit":8}}}),
            Some(temp.path()),
        );
        assert_eq!(response["result"]["isError"], false, "{response:#}");
        let body = tool_body(&response);
        let hits = body["h"].as_array().unwrap();
        assert!(!hits.is_empty(), "{name}: {body:#}");
        let paths = body["p"].as_object().unwrap();
        for hit in hits {
            let tuple = hit.as_array().expect("hit is a positional tuple");
            assert_eq!(tuple.len(), 5, "{name}: {hit:#}");
            assert_eq!(tuple[1], expected_kind, "{name}: {hit:#}");
            assert!(tuple[2].is_string(), "{name}: signal");
            assert!(tuple[4].is_string(), "{name}: snippet");
            // Every id resolves to a real path through the `p` table.
            let id = tuple[0].as_str().expect("id is a string");
            let (path_id, range) = id.rsplit_once(':').expect("id is <path_id>:<start>-<end>");
            assert!(paths.contains_key(path_id), "{name}: unresolved {id}");
            let (start, end) = range.split_once('-').expect("range is start-end");
            assert!(start.parse::<u32>().is_ok() && end.parse::<u32>().is_ok());
        }
        // Object keys must not reappear per hit.
        assert!(hits.iter().all(|hit| hit.get("file").is_none()));
        assert!(hits.iter().all(|hit| hit.get("ref").is_none()));
    }
}

/// kxmc: the compact id handed out by search must expand through code_read in
/// the same session, with no path reconstruction required from the agent.
#[test]
fn compact_search_ids_expand_through_code_read_in_one_session() {
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
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();

    // One process: search, then feed the returned compact id straight back.
    let search = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"keyword_search","arguments":{"query":"target_symbol","limit":4}}});
    let responses = rpc_session(vec![search.clone()], Some(temp.path()));
    let body = tool_body(&responses[0]);
    let compact_id = body["h"][0][0].as_str().expect("compact id").to_owned();
    assert!(
        !compact_id.contains('/'),
        "id must be interned: {compact_id}"
    );

    let read = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"code_read","arguments":{"ids":[compact_id]}}});
    let responses = rpc_session(vec![search, read], Some(temp.path()));
    assert_eq!(responses.len(), 2, "{responses:#?}");
    assert_eq!(
        responses[1]["result"]["isError"], false,
        "{:#}",
        responses[1]
    );
    let read_body = tool_body(&responses[1]);
    assert!(
        read_body["nodes"][0]["content"]
            .as_str()
            .unwrap()
            .contains("target_symbol"),
        "{read_body:#}"
    );
    assert_eq!(read_body["nodes"][0]["id"], "src/lib.rs#L1-L1");
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

#[test]
fn parse_error_uses_jsonrpc_null_id() {
    // JSON-RPC 2.0: when id cannot be detected, id MUST be null (not omitted).
    let mut command = Command::new(mcp_bin());
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = command.spawn().expect("spawn MCP");
    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, "{{not json").unwrap();
    }
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("jsonrpc"))
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let r = &lines[0];
    assert_eq!(r["jsonrpc"], "2.0");
    assert!(r["id"].is_null(), "parse error id must be null, got {r:#}");
    assert_eq!(r["error"]["code"], -32700);
}

#[test]
fn tool_roots_are_sandboxed_under_configured_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("ok.rs"), "fn ok() {}\n").unwrap();
    let response = rpc_at(
        json!({
            "jsonrpc":"2.0","id":21,"method":"tools/call",
            "params":{"name":"index_status","arguments":{"root": outside.path().to_string_lossy()}}
        }),
        Some(workspace.path()),
    );
    assert_eq!(response["result"]["isError"], true, "{response:#}");
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("escapes configured workspace"),
        "{response:#}"
    );
}

/// 9q0l: tool definitions ride in the prompt on every request, so they are the
/// largest cacheable region this server controls. Any instability here costs a
/// full cache miss per call for every connected client.
#[test]
fn tools_list_is_byte_identical_across_calls_and_processes() {
    let list = json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}});
    let same_process = rpc_session(vec![list.clone(), list.clone()], None);
    assert_eq!(same_process.len(), 2);
    let first = serde_json::to_string(&same_process[0]["result"]).unwrap();
    let second = serde_json::to_string(&same_process[1]["result"]).unwrap();
    assert_eq!(first, second, "tools/list differed within one process");

    let fresh_process = rpc(list);
    assert_eq!(
        serde_json::to_string(&fresh_process["result"]).unwrap(),
        first,
        "tools/list differed across processes"
    );

    // No per-call data may leak into a cached region.
    for tool in fresh_process["result"]["tools"].as_array().unwrap() {
        let text = serde_json::to_string(tool).unwrap();
        for volatile in ["/private/", "/tmp/", "generation", "elapsed"] {
            assert!(
                !text.contains(volatile),
                "tool definition carries per-call data {volatile}: {text}"
            );
        }
    }
}

/// 9q0l: identical query plus unchanged index must produce identical bytes, and
/// per-call accounting must stay in the trailing `z*` block.
///
/// Uses `resend_seen` so this measures the stateless encoding. Snippet elision
/// (v972) is deliberate session state and is covered by its own test.
#[test]
fn search_envelope_is_byte_stable_with_volatile_accounting_last() {
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
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();

    let search = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"keyword_search","arguments":{"query":"target_symbol","limit":4,"resend_seen":true}}});
    let responses = rpc_session(vec![search.clone(), search], Some(temp.path()));
    let first = responses[0]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let second = responses[1]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert_eq!(
        first, second,
        "repeated identical search was not byte-stable"
    );

    // Content keys precede the volatile `z*` tail on the wire.
    let tail = first.find("\"zb\"").expect("zb accounting present");
    for content_key in ["\"h\"", "\"p\"", "\"q\"", "\"v\""] {
        let at = first.find(content_key).expect("content key present");
        assert!(at < tail, "{content_key} must precede volatile accounting");
    }
    assert!(first.find("\"zn\"").unwrap() > tail || first.contains("\"zn\""));
}

/// v972: a repeated search must not resend bodies the session already sent,
/// but a reindex must invalidate that memory.
#[test]
fn repeated_search_elides_already_sent_snippets_until_reindex() {
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
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();

    let search = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"keyword_search","arguments":{"query":"target_symbol","limit":4}}});
    let reindex = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"index_repo","arguments":{}}});
    let responses = rpc_session(
        vec![search.clone(), search.clone(), reindex, search.clone()],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 4, "{responses:#?}");

    let first = responses[0]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let second = responses[1]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let after_reindex = responses[3]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();

    // Second identical call carries markers instead of bodies, and is smaller.
    let body = tool_body(&responses[1]);
    assert!(
        body["h"]
            .as_array()
            .unwrap()
            .iter()
            .all(|hit| hit[4] == "~"),
        "expected every snippet elided: {body:#}"
    );
    assert!(body["ze"].as_u64().unwrap() > 0, "elision count missing");
    assert!(
        second.len() < first.len(),
        "elided response must be smaller: {} vs {}",
        second.len(),
        first.len()
    );

    // A reindex clears the memory: bodies come back in full.
    let refreshed = tool_body(&responses[3]);
    assert!(
        refreshed["h"]
            .as_array()
            .unwrap()
            .iter()
            .all(|hit| hit[4] != "~"),
        "reindex must invalidate elision: {refreshed:#}"
    );
    assert_eq!(after_reindex.len(), first.len());
}

/// v972: clients that do not retain earlier results can opt out.
#[test]
fn resend_seen_disables_snippet_elision() {
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
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();

    let search = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"keyword_search","arguments":{"query":"target_symbol","limit":4,"resend_seen":true}}});
    let responses = rpc_session(vec![search.clone(), search], Some(temp.path()));
    let first = responses[0]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let second = responses[1]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert_eq!(first, second, "resend_seen must keep responses identical");
    assert!(!second.contains("\"~\""), "no elision expected: {second}");
}

/// 6a3i: a miss over an unindexed root must say so, not return a bare empty
/// result the agent has to guess about.
#[test]
fn zero_hit_search_returns_a_diagnostic_miss_envelope() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), "fn present() {}\n").unwrap();

    // Nothing indexed yet: the miss must name that, not blame the query.
    let response = rpc_at(
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"keyword_search","arguments":{"query":"absent_symbol","limit":4}}}),
        Some(temp.path()),
    );
    assert_eq!(response["result"]["isError"], false, "{response:#}");
    let body = tool_body(&response);
    assert_eq!(body["why"], "empty_index", "{body:#}");
    assert_eq!(body["zn"], 0);
    assert_eq!(body["tried"], json!(["lexical"]));
    assert!(body["next"].as_str().unwrap().contains("index"));

    // Indexed, but the term genuinely is not there: a different diagnosis.
    ast_sgrep_core::Indexer::new(ast_sgrep_core::IndexOptions {
        root: temp.path().to_path_buf(),
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();
    let response = rpc_at(
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"keyword_search","arguments":{"query":"absent_symbol","limit":4}}}),
        Some(temp.path()),
    );
    let body = tool_body(&response);
    assert_eq!(body["why"], "no_match", "{body:#}");
    assert!(body.get("p").is_none(), "miss carries no path table");

    // A miss is cheaper than a hit envelope for the same query shape.
    let hit = rpc_at(
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"keyword_search","arguments":{"query":"present","limit":4}}}),
        Some(temp.path()),
    );
    let miss_bytes = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .len();
    let hit_bytes = hit["result"]["content"][0]["text"].as_str().unwrap().len();
    assert!(miss_bytes < hit_bytes, "{miss_bytes} vs {hit_bytes}");
}

fn mcp_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/mcp/fixtures")
        .join(name)
}

/// nz7i.3: freeze initialize + full tools/list descriptors (not just names).
#[test]
fn initialize_and_tools_list_match_goldens() {
    let init = rpc(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}));
    let scrubbed: Value = serde_json::from_str(
        &Scrubber::machine_contract()
            .apply(&serde_json::to_string(&init["result"]).expect("serialize initialize")),
    )
    .expect("scrubbed initialize parses");
    assert_eq!(scrubbed["protocolVersion"], "2025-11-25");
    assert_eq!(scrubbed["serverInfo"]["name"], "ast-sgrep");
    assert_eq!(scrubbed["serverInfo"]["version"], "<version>");
    assert_golden_json_at(&mcp_fixture("initialize.json"), &scrubbed);

    let listed = rpc(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    let tools = listed["result"]["tools"].clone();
    assert!(tools.as_array().expect("tools").iter().all(|tool| {
        tool.get("name").is_some()
            && tool.get("description").is_some()
            && tool.get("inputSchema").is_some()
    }));
    assert_golden_json_at(&mcp_fixture("tools_list.json"), &tools);
}
