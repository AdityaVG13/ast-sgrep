use ast_sgrep_codemode::io::dispatch_find_query;

#[test]
fn disk_io_rejects_oversized_sources_before_decoding() {
    use ast_sgrep_core::MAX_INDEX_FILE_BYTES;
    use serde_json::json;
    use std::io::Write;

    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("seed.rs"), "fn seed() {}\n").unwrap();
    let (_index, mut session) = ast_sgrep_testkit::indexed_session_at(root.path());
    // A sparse oversize file with an invalid first byte discriminates size
    // admission before decoding from an unbounded read followed by a cap.
    let mut file = std::fs::File::create(root.path().join("large.txt")).unwrap();
    file.write_all(&[0xff]).unwrap();
    file.set_len(MAX_INDEX_FILE_BYTES + 1).unwrap();
    for (tool, args) in [
        ("read", json!({"path": "large.txt"})),
        (
            "edit",
            json!({"path": "large.txt", "oldText": "old", "newText": "new"}),
        ),
    ] {
        let error = session
            .call(tool, args)
            .expect_err("oversize source")
            .to_string();
        assert!(
            error.contains("exceeds") && error.contains(&MAX_INDEX_FILE_BYTES.to_string()),
            "{tool}: size admission must precede UTF-8 decoding: {error}"
        );
    }
    std::fs::write(root.path().join("small.txt"), "\nαβ\nold\n").unwrap();
    let read = session
        .call("read", json!({"path": "small.txt", "end": 3}))
        .unwrap();
    assert_eq!(read["windows"][0]["text"], "\nαβ\nold");
    session
        .call(
            "edit",
            json!({"path": "small.txt", "oldText": "old", "newText": "new"}),
        )
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("small.txt")).unwrap(),
        "\nαβ\nnew\n"
    );
}

/// INTENT: string line refs (`path#L5-L6`, `path:5-6`) past EOF fail closed
/// naming the file's real line count — MCP `read_node` parity (sandbox.rs
/// bounds the requested range) — instead of answering ok with a phantom
/// empty window. Value-form (`{path, start, end}`) and bare-path reads keep
/// their pinned clamp-at-EOF contract (ffi_surface `..._agree_at_eof`).
/// KILLS: `read_one_window` returning `("", truncated:false)` for refs whose
/// start or end is beyond the last real line (e.g. `README.md#L999999`).
#[test]
fn string_line_refs_beyond_eof_fail_closed() {
    use serde_json::json;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("seed.rs"), "fn seed() {}\n").unwrap();
    std::fs::write(root.path().join("three.rs"), "a\n\nb\n").unwrap();
    std::fs::write(root.path().join("void.rs"), "").unwrap();
    let (_index, mut session) = ast_sgrep_testkit::indexed_session_at(root.path());
    // Unindexed fallback path (written after the index was built).
    std::fs::write(root.path().join("two.rs"), "alpha\nbeta\n").unwrap();

    // Leg 1: indexed string ref past EOF names the file and its line count.
    let error = session
        .call("read", json!({"refs": ["seed.rs#L5-L6"]}))
        .expect_err("indexed ref past EOF must fail closed")
        .to_string();
    assert!(
        error.contains("seed.rs") && error.contains("beyond") && error.contains("1 line"),
        "error must name file + bound: {error}"
    );
    // Leg 2: unindexed (disk) string ref past EOF fails the same way.
    let error = session
        .call("read", json!({"refs": ["two.rs#L999-L999"]}))
        .expect_err("disk ref past EOF must fail closed")
        .to_string();
    assert!(
        error.contains("beyond") && error.contains("two.rs"),
        "{error}"
    );
    // Leg 3: the phantom line after a final newline is not a real line.
    let error = session
        .call("read", json!({"refs": ["two.rs#L3-L3"]}))
        .expect_err("line after final newline is past EOF")
        .to_string();
    assert!(error.contains("beyond"), "{error}");
    // Leg 4: an in-range EMPTY line is a legitimate empty window, never an
    // error — the bound check must not reject empty content.
    let read = session
        .call("read", json!({"refs": ["three.rs#L2-L2"]}))
        .unwrap();
    assert_eq!(read["windows"][0]["text"], "", "{read:#}");
    // Leg 5: LSP-style colon refs share the fail-closed bound.
    let error = session
        .call("read", json!({"refs": ["seed.rs:9"]}))
        .expect_err("colon ref past EOF must fail closed")
        .to_string();
    assert!(error.contains("beyond"), "{error}");
    // Leg 6: explicit L1 on an empty file stays success (MCP emptyfile leg).
    let read = session
        .call("read", json!({"refs": ["void.rs#L1-L1"]}))
        .unwrap();
    assert_eq!(read["windows"][0]["text"], "", "{read:#}");
    // Leg 7: bare-path default windows and Value-form reads keep clamping.
    let read = session.call("read", json!({"refs": ["two.rs"]})).unwrap();
    assert_eq!(read["windows"][0]["text"], "alpha\nbeta", "{read:#}");
    let read = session
        .call("read", json!({"path": "two.rs", "start": 1, "end": 40}))
        .unwrap();
    assert_eq!(read["windows"][0]["text"], "alpha\nbeta", "{read:#}");
}

#[cfg(unix)]
#[test]
fn disk_io_rejects_nonregular_sources_without_blocking() {
    use serde_json::json;

    const CHILD_TOOL: &str = "ASTSGREP_IO_FIFO_TEST_TOOL";
    if let Ok(tool) = std::env::var(CHILD_TOOL) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("seed.rs"), "fn seed() {}\n").unwrap();
        let (_index, mut session) = ast_sgrep_testkit::indexed_session_at(root.path());
        let fifo = root.path().join("source.txt");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let args = if tool == "read" {
            json!({"path": "source.txt"})
        } else {
            json!({"path": "source.txt", "oldText": "old", "newText": "new"})
        };
        let error = session
            .call(&tool, args)
            .expect_err("FIFO is not a source file")
            .to_string();
        assert!(error.contains("regular file"), "{tool}: {error}");
        return;
    }
    // Bound the real call in a child: pre-fix File::open on the FIFO blocks
    // without a writer. The existing runner kills and reaps it on timeout.
    for tool in ["read", "edit"] {
        let output = ast_sgrep_testkit::run_drain_timeout(
            &std::env::current_exe().unwrap(),
            &[
                "--exact",
                "disk_io_rejects_nonregular_sources_without_blocking",
                "--test-threads=1",
            ],
            &[(CHILD_TOOL, tool)],
            std::time::Duration::from_secs(5),
        );
        assert!(
            output.status.success(),
            "{tool}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn blast_symbol_becomes_callers() {
    assert_eq!(
        dispatch_find_query("blast:process_request"),
        "callers:process_request"
    );
}

#[test]
fn blast_path_becomes_imports() {
    assert_eq!(
        dispatch_find_query("blast:src/auth.ts"),
        "imports:src/auth.ts"
    );
}

#[test]
fn blast_prefix_matches_case_insensitively() {
    // Parity with dispatch.ts `/^blast:/i`: any ASCII casing of the prefix
    // routes the same way (symbol -> callers, path-like -> imports).
    assert_eq!(
        dispatch_find_query("BLAST:process_request"),
        "callers:process_request"
    );
    assert_eq!(
        dispatch_find_query("Blast:src/auth.ts"),
        "imports:src/auth.ts"
    );
    assert_eq!(dispatch_find_query("bLaSt:x"), "callers:x");
}

#[test]
fn unprefixed_is_word() {
    assert_eq!(dispatch_find_query("hello"), "word:hello");
}
