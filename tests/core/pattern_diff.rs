//! Pattern-1 differential (ghiw.3): native `pattern:` subset vs ast-grep CLI.
//!
//! Default CI: native supported hits + unsupported fail-closed. Equality vs
//! ast-grep is **not-run** unless `ASGREP_DIFF_AST_GREP` points at an absolute
//! `ast-grep` binary (`DISC-pattern-native-subset`). Unset env must not be
//! reported as match-set Pass.
use ast_sgrep_core::{IndexOptions, SearchOptions};
use ast_sgrep_testkit::{isolated_index_session, IsolatedIndexSession};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE: &str = include_str!("../fixtures/pattern_diff/lib.rs");

const SUPPORTED: &[&str] = &[
    "process_request",
    "fn $NAME($$$)",
    "process_request($$$)",
    "$OBJ.$METHOD($$$)",
    "struct AppContext",
];

const UNSUPPORTED: &[&str] = &[
    "if ($COND) { $BODY }",
    "foo($X + 1)",
    "rule:\n  pattern: fn $A\n  fix: fn $B\n",
    "$A == $B",
];

fn indexed_fixture() -> IsolatedIndexSession {
    let session = isolated_index_session();
    session.write("lib.rs", FIXTURE);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    session
}

fn search_pattern(
    session: &IsolatedIndexSession,
    pattern: &str,
) -> Result<Vec<(String, u32)>, String> {
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let query = format!("pattern:{pattern}");
    match searcher.search(&query) {
        Ok(response) => Ok(response
            .hits
            .into_iter()
            .map(|h| {
                let name = Path::new(&h.file)
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or(h.file);
                (name, h.line_start)
            })
            .collect()),
        Err(err) => Err(err.to_string()),
    }
}

fn competitor_bin() -> Option<PathBuf> {
    let raw = std::env::var_os("ASGREP_DIFF_AST_GREP")?;
    let path = PathBuf::from(raw);
    path.is_absolute().then_some(path)
}

/// ast-grep `run --json` rows: 0-based `range.start.line` in current CLI JSON.
fn ast_grep_match_set(bin: &Path, root: &Path, pattern: &str) -> BTreeSet<(String, u32)> {
    let output = Command::new(bin)
        .args(["run", "--pattern", pattern, "--lang", "rust", "--json"])
        .arg(root)
        .output()
        .unwrap_or_else(|e| panic!("spawn ast-grep: {e}"));
    assert!(
        output.status.success(),
        "ast-grep failed: {}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("ast-grep JSON array");
    let mut out = BTreeSet::new();
    for row in value.as_array().expect("JSON array") {
        let file = row
            .get("file")
            .or_else(|| row.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let name = Path::new(file)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.to_string());
        let line0 = row
            .get("range")
            .and_then(|r| r.get("start"))
            .and_then(|s| s.get("line"))
            .and_then(|l| l.as_u64())
            .unwrap_or(0);
        out.insert((name, u32::try_from(line0 + 1).expect("line")));
    }
    out
}

#[test]
fn supported_native_patterns_hit_fixture() {
    let session = indexed_fixture();
    for pattern in SUPPORTED {
        let hits = search_pattern(&session, pattern).unwrap_or_else(|e| {
            panic!("supported {pattern} must not fail-closed: {e}");
        });
        assert!(
            !hits.is_empty(),
            "supported native pattern {pattern} must hit tests/fixtures/pattern_diff/lib.rs"
        );
    }
}

#[test]
fn exact_struct_app_does_not_match_appcontext() {
    let session = indexed_fixture();
    let hits = search_pattern(&session, "struct App").unwrap_or_else(|e| {
        panic!("struct App is a supported exact signature: {e}");
    });
    assert!(
        hits.iter().any(|(_, line)| *line == 7),
        "native exact signature should hit struct App: {hits:?}"
    );
    assert!(
        hits.iter().all(|(_, line)| *line != 9),
        "native exact signature must not treat struct App as struct AppContext: {hits:?}"
    );
}

#[test]
fn unsupported_shapes_are_empty_or_fail_closed() {
    let session = indexed_fixture();
    for pattern in UNSUPPORTED {
        match search_pattern(&session, pattern) {
            Ok(hits) => assert!(
                hits.is_empty(),
                "unsupported {pattern} must not silently hit: {hits:?} (DISC-pattern-native-subset)"
            ),
            Err(err) => assert!(
                err.contains("ast-grep is unavailable") || err.contains("fail-closed"),
                "unsupported {pattern} error must be fail-closed, got {err}"
            ),
        }
    }
}

/// Pattern-1 equality. Not-run without `ASGREP_DIFF_AST_GREP` (DISC-pattern-native-subset).
#[ignore = "not-run: set ASGREP_DIFF_AST_GREP to an absolute ast-grep binary; DISC-pattern-native-subset"]
#[test]
fn supported_match_sets_equal_ast_grep_when_env_set() {
    let Some(bin) = competitor_bin() else {
        panic!(
            "ignored test executed without ASGREP_DIFF_AST_GREP; not claiming equality (DISC-pattern-native-subset)"
        );
    };
    assert!(
        bin.is_file(),
        "ASGREP_DIFF_AST_GREP must be a file: {}",
        bin.display()
    );
    let session = indexed_fixture();
    for pattern in SUPPORTED {
        let dut: BTreeSet<_> = search_pattern(&session, pattern)
            .unwrap_or_else(|e| panic!("DUT {pattern}: {e}"))
            .into_iter()
            .collect();
        let competitor = ast_grep_match_set(&bin, &session.corpus_root, pattern);
        assert_eq!(
            dut, competitor,
            "match-set mismatch for {pattern} (supported subset, not full ast-grep parity)"
        );
    }
}
