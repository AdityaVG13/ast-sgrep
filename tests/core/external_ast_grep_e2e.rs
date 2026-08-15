//! Opt-in external `ast-grep` spawn/parse (lbx1.9).
//!
//! Production allow path: `ASGREP_ALLOW_AST_GREP=1` plus absolute `ASGREP_AST_GREP`.
//! Never searches `PATH`. Does not feed `pattern:` search (`DISC-pattern-native-subset`).
//!
//! Binary requirement: ignored spawn test needs a real `ast-grep` file.
//! When that ignored test is executed with `ASGREP_E2E_AST_GREP=1`, a missing
//! binary is a hard fail (not a green skip).
use ast_sgrep_core::{run_external_ast_grep, IndexOptions, SearchOptions};
use ast_sgrep_testkit::isolated_index_session;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn e2e_bin() -> Option<PathBuf> {
    let raw = std::env::var_os("ASGREP_AST_GREP")?;
    let path = PathBuf::from(raw);
    path.is_absolute().then_some(path)
}

#[test]
fn fail_closed_without_allow_does_not_spawn() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    std::env::remove_var("ASGREP_ALLOW_AST_GREP");
    std::env::remove_var("ASGREP_AST_GREP");
    std::env::remove_var("ASGREP_DISABLE_AST_GREP");

    let session = isolated_index_session();
    session.write(
        "planted.rs",
        "pub fn planted_lbx19() {}\npub fn other() { if true { planted_lbx19(); } }\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let none = run_external_ast_grep("if $COND { $BODY }", &session.corpus_root, Some("rust"))
        .expect("disallowed spawn must be Ok(None), not a crash");
    assert!(
        none.is_none(),
        "must not spawn ast-grep without ASGREP_ALLOW_AST_GREP: {none:?}"
    );

    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    // Multi-statement templates stay exotic (single-statement `{ $BODY }` is
    // native since ast-sgrep-yira and is asserted below).
    let err = searcher
        .search("pattern: if ($COND) { $A; $B }")
        .expect_err("exotic pattern must fail-closed when ast-grep is unavailable");
    let msg = err.to_string();
    assert!(
        msg.contains("fail-closed") || msg.contains("ast-grep is unavailable"),
        "expected fail-closed, got {msg}"
    );

    // Native nested template must serve hits in-process even though spawning
    // is disallowed: proof it never rides the external ast-grep path.
    let native = searcher
        .search("pattern: if ($COND) { $BODY }")
        .expect("native nested template must not require ast-grep");
    assert!(
        native
            .hits
            .iter()
            .any(|h| h.excerpt.contains("planted_lbx19")),
        "native if-template must hit the planted single-statement if: {native:?}"
    );
}

#[ignore = "not-run: set ASGREP_E2E_AST_GREP=1 and absolute ASGREP_AST_GREP; real ast-grep spawn"]
#[test]
fn opt_in_spawn_parses_fixture_matches() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let required = ast_sgrep_core::env_flag::env_flag("ASGREP_E2E_AST_GREP");
    let Some(bin) = e2e_bin() else {
        panic!(
            "ignored test executed without absolute ASGREP_AST_GREP{}",
            if required {
                " (ASGREP_E2E_AST_GREP=1: hard fail, binary required)"
            } else {
                "; set ASGREP_E2E_AST_GREP=1 and ASGREP_AST_GREP"
            }
        );
    };
    assert!(
        bin.is_file(),
        "ASGREP_AST_GREP must be a file: {}",
        bin.display()
    );
    std::env::set_var("ASGREP_ALLOW_AST_GREP", "1");
    std::env::set_var("ASGREP_AST_GREP", &bin);
    std::env::remove_var("ASGREP_DISABLE_AST_GREP");

    let session = isolated_index_session();
    session.write(
        "planted.rs",
        "pub fn planted_lbx19() {}\npub fn other() { if true { planted_lbx19(); } }\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });

    let matches = run_external_ast_grep("if $COND { $BODY }", &session.corpus_root, Some("rust"))
        .expect("allowed ast-grep spawn must not error")
        .expect("allow gate plus valid binary must spawn, not return None");
    assert!(
        matches.iter().any(|row| {
            Path::new(&row.file)
                .file_name()
                .is_some_and(|name| name == "planted.rs")
                && row.line_start == 2
        }),
        "expected planted.rs:2 from production ast-grep JSON parse, got {matches:?}"
    );
}
