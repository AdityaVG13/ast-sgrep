//! read numeric contract: window/char clamp math + totality + fanout.
//!
//! Single-test contract suite absorbing every `read` numeric from
//! `numerical_pass{1,2,4}`. `read` fails closed on an empty index, so every
//! section indexes first (lexical only, deterministic).

use ast_sgrep_codemode::CallError;
use ast_sgrep_testkit::indexed_session_at;
use serde_json::json;

/// INTENT=read numeric surface: start `.max(1)` / end `.max(start)` clamps, symmetric context widening with saturation, max_chars clamp [1,100000] with truncated flag, degenerate-bound defaults, beyond-EOF/EOF-saturation totality, empty-refs fail-closed, and exact 3-ref fanout windows.
/// KILLS=clamp-bound (`max` floor, EOF-saturation, char clamp, truncated-flag), off-by-one-widen / missing-saturation, type-coercion (`as_u64` None → unwrap_or), error-discriminant (Other-vs-Ok), fanout-ordering (window-reorder/drop).
/// ABSORBS=read_start_end_clamp_to_valid_range, read_context_lines_widens_symmetrically, read_char_budgets_truncate_with_flag, read_negative_start_end_default_to_one, read_beyond_eof_and_huge_end_are_total, read_empty_refs_rejects_without_panic, read_fanout_exact_window_counts
#[test]
fn read_contract() {
    // §1 ABSORBED: read_start_end_clamp_to_valid_range — start=0→line 1;
    // start=3,end=0→line 3 only (file l1..l5 with trailing newline).
    {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("f.txt"), "l1\nl2\nl3\nl4\nl5\n").expect("write");
        let (_index_dir, mut session) = indexed_session_at(temp.path());
        let floored = session
            .call("read", json!({"path": "f.txt", "start": 0}))
            .expect("start 0 runs");
        assert_eq!(floored["windows"][0]["start"], json!(1));
        assert_eq!(floored["windows"][0]["end"], json!(1));
        assert_eq!(floored["windows"][0]["text"], json!("l1"));
        let end_pinned = session
            .call("read", json!({"path": "f.txt", "start": 3, "end": 0}))
            .expect("end 0 runs");
        assert_eq!(end_pinned["windows"][0]["start"], json!(3));
        assert_eq!(end_pinned["windows"][0]["end"], json!(3));
        assert_eq!(end_pinned["windows"][0]["text"], json!("l3"));
    }
    // §2 ABSORBED: read_context_lines_widens_symmetrically — ctx 1 on line
    // 3→lines 2..4; huge ctx saturates (.min(100)) to the whole file. No
    // trailing newline: split('\n') indexing would add an empty 6th line.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("f.txt"), "l1\nl2\nl3\nl4\nl5").expect("write");
        let (_index_dir, mut session) = indexed_session_at(temp.path());
        let widened = session
            .call(
                "read",
                json!({"path": "f.txt", "start": 3, "end": 3, "context_lines": 1}),
            )
            .expect("context 1 runs");
        assert_eq!(widened["windows"][0]["start"], json!(2));
        assert_eq!(widened["windows"][0]["end"], json!(4));
        assert_eq!(widened["windows"][0]["text"], json!("l2\nl3\nl4"));
        let saturated = session
            .call(
                "read",
                json!({"path": "f.txt", "start": 3, "end": 3, "context_lines": 1_000_000_000u64}),
            )
            .expect("huge context runs");
        assert_eq!(saturated["windows"][0]["start"], json!(1));
        assert_eq!(saturated["windows"][0]["end"], json!(5));
        assert_eq!(
            saturated["windows"][0]["text"],
            json!("l1\nl2\nl3\nl4\nl5")
        );
    }
    // §3 ABSORBED: read_char_budgets_truncate_with_flag — max_chars clamps
    // [1,100000]: 0→1 char ("a", truncated); a 2500-char line pins to
    // MAX_LINE_CHARS=2000 with truncated=true.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("abc.txt"), "a\nb\nc\n").expect("write");
        std::fs::write(temp.path().join("wide.txt"), format!("{}\n", "x".repeat(2500)))
            .expect("write");
        let (_index_dir, mut session) = indexed_session_at(temp.path());
        let one_char = session
            .call("read", json!({"path": "abc.txt", "start": 1, "end": 3, "max_chars": 0}))
            .expect("max_chars 0 runs");
        assert_eq!(one_char["windows"][0]["text"], json!("a"));
        assert_eq!(one_char["windows"][0]["start"], json!(1));
        assert_eq!(one_char["windows"][0]["end"], json!(1));
        assert_eq!(one_char["windows"][0]["truncated"], json!(true));
        let wide = session
            .call("read", json!({"path": "wide.txt", "start": 1, "end": 1}))
            .expect("wide line runs");
        assert_eq!(wide["windows"][0]["text"], json!("x".repeat(2000)));
        assert_eq!(wide["windows"][0]["truncated"], json!(true));
    }
    // §4 ABSORBED: read_negative_start_end_default_to_one — `as_u64` None→
    // unwrap_or: start -1→1..1, start 3 end -1→3..3, string bounds→1..1.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("f.txt"), "l1\nl2\nl3\nl4\nl5").expect("write");
        let (_index_dir, mut session) = indexed_session_at(temp.path());
        let neg = session
            .call("read", json!({"path": "f.txt", "start": -1}))
            .expect("negative start runs");
        assert_eq!(neg["windows"][0]["start"], json!(1));
        assert_eq!(neg["windows"][0]["end"], json!(1));
        assert_eq!(neg["windows"][0]["text"], json!("l1"));
        let pinned = session
            .call("read", json!({"path": "f.txt", "start": 3, "end": -1}))
            .expect("negative end runs");
        assert_eq!(pinned["windows"][0]["start"], json!(3));
        assert_eq!(pinned["windows"][0]["end"], json!(3));
        assert_eq!(pinned["windows"][0]["text"], json!("l3"));
        let stringy = session
            .call("read", json!({"path": "f.txt", "start": "bad", "end": "bad"}))
            .expect("string bounds run");
        assert_eq!(stringy["windows"][0]["start"], json!(1));
        assert_eq!(stringy["windows"][0]["end"], json!(1));
        assert_eq!(stringy["windows"][0]["text"], json!("l1"));
    }
    // §5 ABSORBED: read_beyond_eof_and_huge_end_are_total — start past EOF
    // yields empty untruncated text at the requested start; huge end and
    // huge max_chars saturate to the full file, untruncated.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("f.txt"), "l1\nl2\nl3\nl4\nl5").expect("write");
        let (_index_dir, mut session) = indexed_session_at(temp.path());
        let past = session
            .call("read", json!({"path": "f.txt", "start": 9999}))
            .expect("beyond EOF runs");
        assert_eq!(past["windows"][0]["start"], json!(9999));
        assert_eq!(past["windows"][0]["end"], json!(9999));
        assert_eq!(past["windows"][0]["text"], json!(""));
        assert_eq!(past["windows"][0]["truncated"], json!(false));
        let widened = session
            .call("read", json!({"path": "f.txt", "start": 1, "end": 1_000_000_000u64}))
            .expect("huge end runs");
        assert_eq!(widened["windows"][0]["start"], json!(1));
        assert_eq!(widened["windows"][0]["end"], json!(5));
        assert_eq!(widened["windows"][0]["text"], json!("l1\nl2\nl3\nl4\nl5"));
        assert_eq!(widened["windows"][0]["truncated"], json!(false));
        let chars = session
            .call(
                "read",
                json!({"path": "f.txt", "start": 1, "end": 5, "max_chars": u64::MAX}),
            )
            .expect("huge max_chars runs");
        assert_eq!(chars["windows"][0]["text"], json!("l1\nl2\nl3\nl4\nl5"));
        assert_eq!(chars["windows"][0]["truncated"], json!(false));
    }
    // §6 ABSORBED: read_empty_refs_rejects_without_panic — empty refs and
    // missing path/ref/refs fail closed as Other, never Ok-empty.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("f.txt"), "l1\nl2\n").expect("write");
        let (_index_dir, mut session) = indexed_session_at(temp.path());
        let err = session
            .call("read", json!({"refs": []}))
            .expect_err("empty refs must fail");
        assert!(matches!(err, CallError::Other(_)), "got {err:?}");
        let err = session.call("read", json!({})).expect_err("missing ref must fail");
        assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    }
    // §7 ABSORBED: read_fanout_exact_window_counts — one read over 3 refs
    // yields count 3 with exact (start,end,text) in ref order; index(1) +
    // read(2) consume exactly 2 calls.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("fanout.txt"), "r1\nr2\nr3\nr4\nr5\nr6").expect("write");
        let (_index_dir, mut session) = indexed_session_at(temp.path());
        assert_eq!(session.call_count(), 1);
        let out = session
            .call(
                "read",
                json!({"refs": [
                    {"path": "fanout.txt", "start": 1, "end": 2},
                    {"path": "fanout.txt", "start": 5, "end": 6},
                    {"path": "fanout.txt", "start": 3, "end": 3},
                ]}),
            )
            .expect("fanout read runs");
        assert_eq!(session.call_count(), 2);
        assert_eq!(out["count"], json!(3));
        let windows = out["windows"].as_array().expect("windows");
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0]["start"], json!(1));
        assert_eq!(windows[0]["end"], json!(2));
        assert_eq!(windows[0]["text"], json!("r1\nr2"));
        assert_eq!(windows[1]["start"], json!(5));
        assert_eq!(windows[1]["end"], json!(6));
        assert_eq!(windows[1]["text"], json!("r5\nr6"));
        assert_eq!(windows[2]["start"], json!(3));
        assert_eq!(windows[2]["end"], json!(3));
        assert_eq!(windows[2]["text"], json!("r3"));
    }
}
