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
fn unprefixed_is_word() {
    assert_eq!(dispatch_find_query("hello"), "word:hello");
}
