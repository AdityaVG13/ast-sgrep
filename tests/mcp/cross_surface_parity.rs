//! Cross-surface semantic parity gate (design + drift evidence:
//! `.internal/cross-surface-parity-investigation.md`, 2026-10-24).
//!
//! INTENT: one engine, many adapters. The same query on the same fixture
//! must return the SAME hit sequence `(path, start_line, end_line)` through
//! every delivery surface:
//! - CLI: bare invocation `asgrep --json QUERY ROOT` (machine envelope),
//! - serve worker: in-process `run_serve` — the exact entry `asgrep serve`
//!   runs, with the production session config,
//! - MCP: the stdio server's fused `search` tool, capsule wire format
//!   decoded with the canonical `resolve_compact_paths` decoder.
//!
//! KILLS: a surface silently shipping the wrong lane (the Hermes plugin's
//! `find`-instead-of-`search` bug class — mutation-proven below), per-surface
//! ranking drift, hit-order divergence, and anchor divergence.
//!
//! Narrowings (deliberate, v1): no conceptual/semantic query (proven live in
//! the investigation doc; add with a frozen embedding fixture), no param-knob
//! sweep (limit/filter parity is envelope-level, not hit-sequence-level), and
//! the index is shared via `indexed_tree` (index-build parity belongs to the
//! warm/cold suites). Rendering is deliberately NOT compared: MCP's compact
//! capsule is a token feature — the gate proves the semantics under it.
//!
//! Prerequisite: the CLI arm spawns the `asgrep` binary via testkit's
//! `asgrep_bin()` — cargo does not build sibling-crate binaries for a test
//! target, so build it first (`cargo build -p ast-sgrep-cli`) or run inside
//! an already-built workspace (DSR/RCH do).

use ast_sgrep_codemode::{run_serve, SessionConfig};
use ast_sgrep_core::SearchOptions;
use ast_sgrep_plugins::resolve_compact_paths;
use ast_sgrep_testkit::{asgrep_bin, indexed_tree, rpc_session, tool_body, tool_call};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

type Norm = Vec<(String, u64, u64)>;

const CORPUS: &[(&str, &str)] = &[
    ("bare", "gamma_widget"),
    ("multiword", "alpha marker"),
    ("literal", "word:flurbify_me"),
    ("pattern", "pattern:fn alpha_marker() {}"),
    ("defs", "defs:gamma_widget"),
    ("callers", "callers:alpha_marker"),
    ("miss", "word:nonexistent_zzz"),
];

fn fixture() -> tempfile::TempDir {
    indexed_tree(&[
        (
            "src/alpha.rs",
            "fn alpha_marker() {}\n\nfn gamma_widget() {\n    alpha_marker();\n}\n",
        ),
        (
            "src/beta.py",
            "import os\n\n\ndef beta_helper():\n    return \"flurbify_me\"\n",
        ),
        (
            "docs/notes.md",
            "Prose note: gamma_widget is only mentioned here in text.\n",
        ),
    ])
}

// --- CLI arm: bare invocation, machine envelope hits ---

fn cli_norm(root: &Path, query: &str) -> Norm {
    let root = root.to_string_lossy().into_owned();
    let out = ast_sgrep_testkit::run(&asgrep_bin(), &["--json", query, &root]);
    assert!(
        out.status.success(),
        "cli search failed for {query:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let env: Value = serde_json::from_slice(&out.stdout).expect("cli machine json");
    env.get("hits")
        .and_then(Value::as_array)
        .map(|hits| {
            hits.iter()
                .map(|h| {
                    (
                        h["file"].as_str().expect("cli hit.file").to_string(),
                        h["line_start"].as_u64().expect("cli hit.line_start"),
                        h["line_end"].as_u64().expect("cli hit.line_end"),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

// --- serve arm: the production `asgrep serve` entry, in-process ---

fn serve_config(root: &Path) -> SessionConfig {
    // Mirrors the CLI's `codemode_session_config`: ambient index under the
    // root, default limit, embed channel on, ambient freshness on.
    SessionConfig {
        root: root.to_path_buf(),
        index_path: None,
        limit: SearchOptions::default_limit(),
        use_embed: true,
        ..SessionConfig::default()
    }
}

fn serve_norms(root: &Path, tool: &str, queries: &[&str]) -> Vec<Norm> {
    let input = queries
        .iter()
        .enumerate()
        .map(|(i, q)| {
            json!({"type": "call", "id": format!("p{i}"), "tool": tool, "args": {"query": q}})
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = Vec::new();
    run_serve(serve_config(root), Cursor::new(input.as_bytes()), &mut out).expect("serve run");
    let text = String::from_utf8(out).expect("serve output utf8");
    text.lines()
        .map(|line| {
            let v: Value = serde_json::from_str(line).expect("serve line json");
            assert!(v["ok"].as_bool().unwrap_or(false), "serve call failed: {v}");
            v["value"]
                .get("hits")
                .and_then(Value::as_array)
                .map(|hits| {
                    hits.iter()
                        .map(|h| {
                            (
                                h["file"].as_str().expect("serve hit.file").to_string(),
                                h["lines"]["start"].as_u64().expect("serve hit.lines.start"),
                                h["lines"]["end"].as_u64().expect("serve hit.lines.end"),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect()
}

// --- MCP arm: stdio server, fused `search`, capsule decoded canonically ---

fn mcp_norms(root: &Path, queries: &[&str]) -> Vec<Norm> {
    let payloads: Vec<Value> = queries
        .iter()
        .enumerate()
        .map(|(i, q)| tool_call(i as u32 + 1, "search", json!({"query": q})))
        .collect();
    let responses = rpc_session(payloads, Some(root));
    responses
        .iter()
        .map(|r| {
            assert!(r.get("error").is_none(), "mcp call failed: {r}");
            let body = tool_body(r);
            let paths: HashMap<String, String> = resolve_compact_paths(&body).into_iter().collect();
            body.get("h")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .map(|row| {
                            let nid = row[0].as_str().expect("capsule nid");
                            let (base, range) = nid.split_once(':').expect("nid id:range");
                            let file = paths
                                .get(base)
                                .unwrap_or_else(|| panic!("capsule id {base} not in path table"))
                                .clone();
                            let (start, end) = range.split_once('-').expect("nid range");
                            (
                                file,
                                start.parse().expect("capsule start"),
                                end.parse().expect("capsule end"),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect()
}

#[test]
fn hit_sequences_are_identical_across_cli_serve_and_mcp() {
    let temp = fixture();
    let root = temp.path();
    let queries: Vec<&str> = CORPUS.iter().map(|(_, q)| *q).collect();

    let cli: Vec<Norm> = queries.iter().map(|q| cli_norm(root, q)).collect();
    let serve = serve_norms(root, "search", &queries);
    let mcp = mcp_norms(root, &queries);

    for (i, (label, _)) in CORPUS.iter().enumerate() {
        assert_eq!(cli[i], serve[i], "{label}: CLI vs serve diverged");
        assert_eq!(cli[i], mcp[i], "{label}: CLI vs MCP diverged");
    }
    // Non-vacuity: EVERY lane in the corpus must actually hit on at least
    // one surface (equality over empty lists proves nothing — a lane that
    // silently empties on all three surfaces would otherwise pass).
    for (i, (label, query)) in CORPUS.iter().enumerate() {
        if *label != "miss" {
            assert!(!cli[i].is_empty(), "{label} ({query:?}): lane went vacuous");
        }
    }
    // Empty-result parity: the miss query is empty everywhere.
    assert!(
        cli.last().is_some_and(|n| n.is_empty()),
        "miss query must be empty"
    );
}

#[test]
fn determinism_and_usage_error_parity() {
    let temp = fixture();
    let root = temp.path();

    // Same query twice per surface -> identical sequence.
    let c1 = cli_norm(root, "alpha marker");
    let c2 = cli_norm(root, "alpha marker");
    assert_eq!(c1, c2, "CLI must be deterministic");
    let serve = serve_norms(root, "search", &["alpha marker", "alpha marker"]);
    assert_eq!(serve[0], serve[1], "serve must be deterministic");
    let mcp = mcp_norms(root, &["alpha marker", "alpha marker"]);
    assert_eq!(mcp[0], mcp[1], "MCP must be deterministic");

    // Missing query is a usage-class rejection in every dialect.
    let cli_err = ast_sgrep_testkit::run(&asgrep_bin(), &["search", "--json"]);
    assert!(!cli_err.status.success(), "missing query must fail");
    let cli_env: Value = serde_json::from_slice(&cli_err.stdout).expect("cli usage envelope");
    assert_eq!(cli_env["error"]["kind"], "usage", "cli usage kind");

    let line = json!({"type": "call", "id": "e1", "tool": "search", "args": {}}).to_string();
    let mut out = Vec::new();
    run_serve(serve_config(root), Cursor::new(line.as_bytes()), &mut out).expect("serve run");
    let v: Value =
        serde_json::from_str(String::from_utf8_lossy(&out).lines().next().expect("line"))
            .expect("serve error json");
    assert_eq!(v["ok"], false, "serve must reject missing query");
    assert!(
        v["error"].as_str().unwrap_or("").contains("required"),
        "serve error must name the requirement: {v}"
    );

    let resp = &rpc_session(vec![tool_call(1, "search", json!({}))], Some(root))[0];
    assert!(resp.get("error").is_some(), "mcp must reject missing query");
    assert_eq!(resp["error"]["code"], -32602, "mcp usage-class code");
}
