//! Pattern-1 differential (ghiw.3): native `pattern:` subset vs ast-grep CLI.
//!
//! Default CI: native supported hits + unsupported fail-closed. Equality vs
//! ast-grep is **not-run** unless `ASGREP_DIFF_AST_GREP` points at an absolute,
//! pinned `ast-grep` binary (`DISC-pattern-native-subset`). Unset env must not
//! be reported as match-set Pass.
use ast_sgrep_core::{IndexOptions, SearchOptions};
use ast_sgrep_testkit::{isolated_index_session, IsolatedIndexSession};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE: &str = include_str!("../fixtures/pattern_diff/lib.rs");
const PINNED_AST_GREP_VERSION: &str = "0.45.1";

const SUPPORTED: &[&str] = &[
    "process_request",
    "process_request($$$)",
    "$OBJ.$METHOD($$$)",
    // Nested statement template (ast-sgrep-yira): exact Rust token form, and
    // ast-grep agrees on the one-statement semantics for the fixture ifs.
    "if $COND { $BODY }",
];

/// Native-only normalized forms: these must hit natively but stay OUT of the
/// ast-grep equality list because ast-grep parses patterns token-exactly:
/// - `if ($COND) { $BODY }` / `if $COND: $BODY`: paren/colon forms only match
///   that literal syntax in ast-grep; the native engine normalizes them so one
///   template works across all indexed languages.
/// - `fn $NAME($$$)`: ast-grep parses the bodyless form as a trait
///   `function_signature_item`, so it matches no `fn` declarations at all.
/// - `fn $N($$$) { $STMT }`: ast-grep is visibility-exact (`pub fn` does not
///   match a pattern without `pub`); the native engine matches any function.
/// - `struct AppContext`: the bodyless struct pattern does not match
///   `struct AppContext {}` in ast-grep; the native engine matches the decl.
const SUPPORTED_NATIVE_NORMALIZED: &[&str] = &[
    "if ($COND) { $BODY }",
    "if $COND: $BODY",
    "fn $NAME($$$)",
    "fn $N($$$) { $STMT }",
    "struct AppContext",
];

const UNSUPPORTED: &[&str] = &[
    "if ($COND) { $A; $B }",
    "if (x > 0) { $BODY }",
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
    assert!(
        path.is_absolute(),
        "ASGREP_DIFF_AST_GREP must be absolute: {}",
        path.display()
    );
    Some(path)
}

fn assert_pinned_competitor(bin: &Path) {
    let output = Command::new(bin)
        .arg("--version")
        .output()
        .unwrap_or_else(|e| panic!("run ast-grep --version: {e}"));
    assert!(
        output.status.success(),
        "ast-grep --version failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("ast-grep {PINNED_AST_GREP_VERSION}"),
        "Pattern-1 keep-gate requires the pinned ast-grep version"
    );
}

/// ast-grep `run --json` rows: 0-based `range.start.line` in current CLI JSON.
fn ast_grep_match_set(bin: &Path, root: &Path, pattern: &str) -> BTreeSet<(String, u32)> {
    let output = Command::new(bin)
        .args(["run", "--pattern", pattern, "--lang", "rust", "--json"])
        .arg(root)
        .output()
        .unwrap_or_else(|e| panic!("spawn ast-grep: {e}"));
    // grep convention: exit 0 = matches, exit 1 = valid run with no matches.
    let no_matches = output.status.code() == Some(1);
    assert!(
        output.status.success() || no_matches,
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
    for pattern in SUPPORTED.iter().chain(SUPPORTED_NATIVE_NORMALIZED) {
        let hits = search_pattern(&session, pattern).unwrap_or_else(|e| {
            panic!("supported {pattern} must not fail-closed: {e}");
        });
        assert!(
            !hits.is_empty(),
            "supported native pattern {pattern} must hit tests/fixtures/pattern_diff/lib.rs"
        );
    }
}

/// Nested templates enforce statement counts (ast-sgrep-yira): `{ $STMT }` /
/// `{ $BODY }` is exactly one statement, `{ $$$ }` is any body, `{}` is empty.
#[test]
fn nested_templates_enforce_statement_counts() {
    let session = indexed_fixture();
    let lines = |pattern: &str| -> BTreeSet<u32> {
        search_pattern(&session, pattern)
            .unwrap_or_else(|e| panic!("{pattern}: {e}"))
            .into_iter()
            .map(|(_, line)| line)
            .collect()
    };
    // guard's first if has one statement; its second has two.
    assert_eq!(lines("if $COND { $BODY }"), BTreeSet::from([24]));
    // Paren and colon forms normalize to the same template.
    assert_eq!(lines("if ($COND) { $BODY }"), BTreeSet::from([24]));
    assert_eq!(lines("if $COND: $BODY"), BTreeSet::from([24]));
    // Any-body matches both ifs.
    assert_eq!(lines("if ($COND) { $$$ }"), BTreeSet::from([24, 25]));
    // Single-statement functions: other (3), tick (12), demo (19).
    // guard has three body statements; process_request/helper are empty.
    assert_eq!(lines("fn $N($$$) { $STMT }"), BTreeSet::from([3, 12, 19]));
    // Empty-body functions: process_request (1), helper (16).
    assert_eq!(lines("fn $N($$$) {}"), BTreeSet::from([1, 16]));
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
#[test]
fn supported_match_sets_equal_pinned_ast_grep_when_configured() {
    let Some(bin) = competitor_bin() else {
        eprintln!(
            "not-run: set ASGREP_DIFF_AST_GREP to pinned ast-grep {PINNED_AST_GREP_VERSION}; not claiming equality (DISC-pattern-native-subset)"
        );
        return;
    };
    assert!(
        bin.is_file(),
        "ASGREP_DIFF_AST_GREP must be a file: {}",
        bin.display()
    );
    assert_pinned_competitor(&bin);
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
