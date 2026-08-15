//! Real `asgrep-lsp` process over LSP stdio JSON-RPC (lbx1.12).
//!
//! In-process `LspBackend` coverage lives in `lsp.rs` and does not close this
//! bead. A missing binary is a hard fail: cargo always builds `asgrep-lsp`
//! before this integration test.
use ast_sgrep_lsp::path_to_file_uri;
use ast_sgrep_lsp::transport::{read_message, write_message};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

const PLANTED: &str = "planted_lbx112_lsp_stdio";

fn lsp_bin() -> PathBuf {
    if let Some(raw) = option_env!("CARGO_BIN_EXE_asgrep-lsp") {
        let path = PathBuf::from(raw);
        assert!(
            path.is_file(),
            "asgrep-lsp missing at {}; lsp_stdio_e2e requires a real process",
            path.display()
        );
        return path;
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let exe = format!("asgrep-lsp{}", std::env::consts::EXE_SUFFIX);
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        let candidate = PathBuf::from(dir).join(profile).join(&exe);
        if candidate.is_file() {
            return candidate;
        }
    }
    let fallback = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(profile)
        .join(&exe);
    assert!(
        fallback.is_file(),
        "asgrep-lsp missing at {}; lsp_stdio_e2e requires a real process",
        fallback.display()
    );
    fallback
}

struct LspProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: Arc<Mutex<String>>,
}

impl LspProcess {
    fn spawn(bin: &Path, cache_home: &Path) -> Self {
        let mut child = Command::new(bin)
            .arg("--stdio")
            .env("NO_COLOR", "1")
            .env("XDG_CACHE_HOME", cache_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|err| panic!("spawn asgrep-lsp: {err}"));
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        let stderr_pipe = child.stderr.take().expect("piped stderr");
        let stderr = Arc::new(Mutex::new(String::new()));
        let stderr_writer = Arc::clone(&stderr);
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr_pipe);
            let mut buf = String::new();
            while reader.read_line(&mut buf).unwrap_or(0) > 0 {
                if let Ok(mut held) = stderr_writer.lock() {
                    held.push_str(&buf);
                }
                buf.clear();
            }
        });
        Self {
            child,
            stdin,
            stdout,
            stderr,
        }
    }

    fn stderr_text(&self) -> String {
        self.stderr
            .lock()
            .map(|held| held.clone())
            .unwrap_or_default()
    }

    fn notify(&mut self, method: &str, params: Value) {
        write_message(
            &mut self.stdin,
            &json!({"jsonrpc":"2.0","method":method,"params":params}).to_string(),
        )
        .unwrap_or_else(|err| panic!("write {method}: {err}; stderr={}", self.stderr_text()));
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        write_message(
            &mut self.stdin,
            &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string(),
        )
        .unwrap_or_else(|err| panic!("write {method}: {err}; stderr={}", self.stderr_text()));
        loop {
            let body = read_message(&mut self.stdout)
                .unwrap_or_else(|err| {
                    panic!(
                        "read frame after {method}: {err}; stderr={}",
                        self.stderr_text()
                    )
                })
                .unwrap_or_else(|| {
                    panic!(
                        "eof before response id={id} method={method}; stderr={}",
                        self.stderr_text()
                    )
                });
            let msg: Value = serde_json::from_str(&body).unwrap_or_else(|err| {
                panic!(
                    "json after {method}: {err}; body={body}; stderr={}",
                    self.stderr_text()
                )
            });
            if msg.get("id") == Some(&json!(id)) {
                return msg;
            }
        }
    }
}

impl Drop for LspProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn assert_search_hits(label: &str, response: &Value, query: &str) {
    assert_eq!(response["jsonrpc"], "2.0", "{label} {response}");
    assert!(response.get("error").is_none(), "{label} {response}");
    let hits = response["result"]["hits"]
        .as_array()
        .unwrap_or_else(|| panic!("{label} missing hits: {response}"));
    assert!(!hits.is_empty(), "{label} empty hits: {response}");
    assert!(
        hits.iter().any(|hit| {
            hit["excerpt"].as_str().unwrap_or("").contains(query)
                || hit["symbol"].as_str() == Some(query)
        }),
        "{label} planted content missing: {response}"
    );
    assert!(
        hits.iter().all(|hit| hit["signal"].is_string()
            && hit["contributors"].is_array()
            && hit["score"].is_number()
            && hit["margin"].is_number()),
        "{label} hit shape: {response}"
    );
}

#[test]
fn stdio_initialize_reindex_search_and_shutdown() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("project");
    let src = root.join("src");
    std::fs::create_dir_all(&src).expect("src");
    std::fs::write(src.join("lib.rs"), format!("pub fn {PLANTED}() {{}}\n")).expect("lib.rs");
    let cache_home = temp.path().join("xdg-cache");
    std::fs::create_dir_all(&cache_home).expect("xdg-cache");
    let root_uri = path_to_file_uri(&root);

    let mut lsp = LspProcess::spawn(&lsp_bin(), &cache_home);
    let init = lsp.request(
        1,
        "initialize",
        json!({
            "rootUri": root_uri,
            "capabilities": {},
            "initializationOptions": { "noEmbed": true }
        }),
    );
    assert_eq!(init["id"], 1, "{init}");
    assert_eq!(init["result"]["serverInfo"]["name"], "asgrep-lsp");
    assert_eq!(
        init["result"]["capabilities"]["experimental"]["asgrepSearchProvider"],
        true
    );
    let commands = init["result"]["capabilities"]["executeCommandProvider"]["commands"]
        .as_array()
        .expect("commands");
    assert!(
        commands
            .iter()
            .any(|c| c.as_str() == Some("asgrep.reindex")),
        "{init}"
    );
    assert!(
        commands.iter().any(|c| c.as_str() == Some("asgrep.search")),
        "{init}"
    );

    lsp.notify("initialized", json!({}));

    let reindex = lsp.request(
        2,
        "workspace/executeCommand",
        json!({"command":"asgrep.reindex","arguments":[]}),
    );
    assert_eq!(reindex["result"]["status"], "reindexed", "{reindex}");

    let search = lsp.request(
        3,
        "asgrep/search",
        json!({"query": PLANTED, "semantic": false, "limit": 16}),
    );
    assert_search_hits("asgrep/search", &search, PLANTED);

    let cmd_search = lsp.request(
        4,
        "workspace/executeCommand",
        json!({"command":"asgrep.search","arguments":[PLANTED]}),
    );
    assert_search_hits("asgrep.search", &cmd_search, PLANTED);

    let shutdown = lsp.request(5, "shutdown", json!({}));
    assert_eq!(shutdown["id"], 5, "{shutdown}");
    assert!(shutdown["result"].is_null(), "{shutdown}");
    lsp.notify("exit", json!({}));
    let status = lsp.child.wait().expect("wait lsp");
    assert!(
        status.success(),
        "exit={status:?} stderr={}",
        lsp.stderr_text()
    );
}
