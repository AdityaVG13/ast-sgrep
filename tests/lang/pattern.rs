use ast_sgrep_lang::{
    classify_native, match_pattern, native_pattern_answerable, needs_ast_grep_fallback, Language,
    NativeKind,
};
use ast_sgrep_testkit::sample_file;
#[test]
fn literal_pattern_matches_rust_symbol() {
    let source = sample_file("src/main.rs");
    let hits = match_pattern(Language::Rust, &source, "process_request").unwrap();
    assert!(!hits.is_empty());
}
#[test]
fn literal_pattern_matching_is_case_sensitive() {
    let source = "fn Foo() {}\nfn foo() {}\nfn FOO() {}\n";
    let upper_camel = match_pattern(Language::Rust, source, "Foo").unwrap();
    let lower = match_pattern(Language::Rust, source, "foo").unwrap();
    let upper = match_pattern(Language::Rust, source, "FOO").unwrap();
    assert!(!upper_camel.is_empty());
    assert!(upper_camel.iter().all(|hit| hit.line_start == 1));
    assert!(!lower.is_empty());
    assert!(lower.iter().all(|hit| hit.line_start == 2));
    assert!(!upper.is_empty());
    assert!(upper.iter().all(|hit| hit.line_start == 3));
}
#[test]
fn literal_pattern_case_mismatch_has_no_match() {
    let source = "fn foo() {}\n";
    assert!(match_pattern(Language::Rust, source, "Foo")
        .unwrap()
        .is_empty());
}
#[test]
fn common_metavariable_patterns_are_native() {
    // Common shapes run in-process; exotic rules are fail-closed / empty, not delegated.
    assert!(!needs_ast_grep_fallback("fn $NAME($$$)"));
    assert!(!needs_ast_grep_fallback("def $NAME"));
    assert!(!needs_ast_grep_fallback("$OBJ.$METHOD($$$)"));
    assert!(!needs_ast_grep_fallback("process_request"));
    assert!(!needs_ast_grep_fallback("if ($COND) { $BODY }"));
    assert!(needs_ast_grep_fallback("if ($COND) { $A; $B }"));
}

#[test]
fn malformed_metavariable_patterns_fall_back_without_panicking() {
    // PASS 51: `foo($X + 1)` left this list — the general expression lane now
    // serves nested-call/argument-content shapes natively (see
    // dupmeta_rust_nested_call_with_literal_arg below); it was never malformed.
    for pattern in ["$)(", "foo.$M+.bar($$$)", "foo.$M.($$$)"] {
        assert!(needs_ast_grep_fallback(pattern), "{pattern}");
        assert!(
            match_pattern(Language::Rust, "fn foo() {}", pattern)
                .unwrap()
                .is_empty(),
            "{pattern}"
        );
    }
}

#[test]
fn structural_fn_pattern_matches_rust_source() {
    use ast_sgrep_lang::match_pattern;
    let source = sample_file("src/main.rs");
    let hits = match_pattern(Language::Rust, &source, "fn $NAME($$$)").unwrap();
    assert!(
        !hits.is_empty(),
        "expected native structural matches for fn $NAME($$$)"
    );
}

// EXP-005 (H-CONF-005 / H-CONF-010, pass 14): native function templates must
// agree with sg strictness — a pattern without a return-type section matches
// only declarations without one, and literal-arity patterns require the same
// parameter count.
#[test]
fn rust_fn_template_requires_return_type_and_arity_agreement() {
    let source = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn inc(v: i32) -> i32 {\n    v + 1\n}\n\nfn main() {\n    let _ = add(1, 2);\n}\n";
    let hits = match_pattern(Language::Rust, source, "fn $A($$$B) { $$$C }").unwrap();
    assert_eq!(
        hits.len(),
        1,
        "return-typed fns must not match a return-type-free template: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert!(
        hits[0].excerpt.contains("fn main"),
        "the sole hit must be the untyped zero-arg fn: {:?}",
        hits[0].excerpt
    );
    let exact = match_pattern(Language::Rust, source, "fn $A($B) { $$$C }").unwrap();
    assert!(
        exact.is_empty(),
        "a one-param template matches neither the return-typed one-param fn nor the zero-param fn: {:?}",
        exact.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

// H-CONF-018 (pass 15): ruby call nodes carry their callee in the `method`
// field, which neither the `function`/`name` field probe nor the
// `call_expression` first-child fallback inspected — every ruby call pattern
// came back silently empty. Expected sets are the ast-grep 0.45.2 oracle's
// (probed 2026-09-03): `$F($$$A)` / `greet($$$A)` hit only the receiver-free
// call on line 9; receiver calls (`text.upcase`) and operator calls
// (`"hello " + name`) match no bare-name pattern.
#[test]
fn ruby_call_patterns_match_receiver_free_calls() {
    let source = "def greet(name)\n  \"hello \" + name\nend\n\ndef shout(text)\n  text.upcase\nend\n\ngreet(\"world\")\n";
    let wildcard = match_pattern(Language::Ruby, source, "$F($$$A)").unwrap();
    assert_eq!(
        wildcard.iter().map(|h| h.line_start).collect::<Vec<_>>(),
        vec![9],
        "$F($$$A) must hit exactly the receiver-free call: {:?}",
        wildcard.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    let literal = match_pattern(Language::Ruby, source, "greet($$$A)").unwrap();
    assert_eq!(
        literal.iter().map(|h| h.line_start).collect::<Vec<_>>(),
        vec![9],
        "greet($$$A) must hit exactly the receiver-free call: {:?}",
        literal.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    for pattern in ["upcase($$$A)", "$O.$M($$$A)"] {
        assert!(
            match_pattern(Language::Ruby, source, pattern)
                .unwrap()
                .is_empty(),
            "{pattern} must not match receiver or operator calls"
        );
    }
}

#[test]
fn ts_and_swift_fn_templates_require_return_type_agreement() {
    let ts = "function load(url: string): string {\n  return url;\n}\n\nfunction keep(x: string) {\n  return x;\n}\n";
    let hits = match_pattern(Language::TypeScript, ts, "function $A($B) { $$$C }").unwrap();
    assert_eq!(
        hits.len(),
        1,
        "TS return-typed function must not match: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert!(
        hits[0].excerpt.contains("function keep"),
        "{:?}",
        hits[0].excerpt
    );

    let swift = "func greet(name: String) -> String {\n    return name\n}\n\nfunc run() {\n    let _ = greet(name: \"x\")\n}\n";
    let hits = match_pattern(Language::Swift, swift, "func $A() { $$$B }").unwrap();
    assert_eq!(
        hits.len(),
        1,
        "swift return-typed func must not match: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert!(
        hits[0].excerpt.contains("func run"),
        "{:?}",
        hits[0].excerpt
    );
}

// Pass 22 (H-CONF-009): dotted member-chain call patterns. Every expected set
// below mirrors a live probe of the pinned reference (ast-grep 0.45.2)
// recorded in the pass-22 spec matrix
// (gauntlet workspace scripts/oracle/pass22_member_matrix.py, oracle mode).

/// Java grammars split the callee across `object` + `name` fields; the chain
/// must be reassembled. sg probes: `obj.helper($$$A)` [3], `a.b.c($$$A)` [4],
/// `$O.$M($$$A)` [3,4,5], `obj.$M($$$A)` [3], `$O.helper($$$A)` [3],
/// `this.render($$$A)` [5]; `helper($$$A)` [] (trailing-name patterns never
/// match a member chain) and `$F($$$A)` [] (the callee is not one node, so a
/// single metavariable has nothing to bind).
#[test]
fn java_member_call_chains_match_dotted_patterns() {
    let source = "class T {\n    void go() {\n        obj.helper(1);\n        a.b.c(2);\n        this.render(3);\n    }\n}\n";
    let lines = |pattern: &str| {
        match_pattern(Language::Java, source, pattern)
            .unwrap()
            .iter()
            .map(|h| h.line_start)
            .collect::<Vec<u32>>()
    };
    assert_eq!(lines("obj.helper($$$A)"), vec![3u32]);
    assert_eq!(lines("a.b.c($$$A)"), vec![4u32]);
    assert_eq!(lines("$O.$M($$$A)"), vec![3u32, 4, 5]);
    assert_eq!(lines("obj.$M($$$A)"), vec![3u32]);
    assert_eq!(lines("$O.helper($$$A)"), vec![3u32]);
    assert_eq!(lines("this.render($$$A)"), vec![5u32]);
    assert_eq!(lines("self.render($$$A)"), Vec::<u32>::new());
    assert_eq!(lines("helper($$$A)"), Vec::<u32>::new());
    assert_eq!(lines("$F($$$A)"), Vec::<u32>::new());
}

/// Python `attribute` nodes are member-chain kinds: wildcard dotted patterns
/// must resolve the full chain. sg probes: `$O.$M($$$A)` [1,2,3],
/// `obj.$M($$$A)` [1], `$O.helper($$$A)` [1], `self.render($$$A)` [3],
/// `helper($$$A)` [], `$F($$$A)` [1,2,3] (python's callee IS one node).
#[test]
fn python_member_call_chains_match_wildcard_patterns() {
    let source = "obj.helper(1)\na.b.c(2)\nself.render(3)\n";
    let lines = |pattern: &str| {
        match_pattern(Language::Python, source, pattern)
            .unwrap()
            .iter()
            .map(|h| h.line_start)
            .collect::<Vec<u32>>()
    };
    assert_eq!(lines("$O.$M($$$A)"), vec![1u32, 2, 3]);
    assert_eq!(lines("obj.$M($$$A)"), vec![1u32]);
    assert_eq!(lines("$O.helper($$$A)"), vec![1u32]);
    assert_eq!(lines("self.render($$$A)"), vec![3u32]);
    assert_eq!(lines("helper($$$A)"), Vec::<u32>::new());
    assert_eq!(lines("$F($$$A)"), vec![1u32, 2, 3]);
}

/// A leading metavariable segment absorbs a multi-segment receiver
/// (`$O.$M($$$A)` matches `a.b.c(2)` with `$O` = `a.b`); a literal leading
/// segment requires the exact chain. sg probes (typescript):
/// `$O.$M($$$A)` [1,2,3], `obj.$M($$$A)` [1], `helper($$$A)` [],
/// `$F($$$A)` [1,2,3].
#[test]
fn wildcard_call_paths_absorb_multi_segment_receivers() {
    let source = "obj.helper(1);\na.b.c(2);\nthis.render(3);\n";
    let lines = |pattern: &str| {
        match_pattern(Language::TypeScript, source, pattern)
            .unwrap()
            .iter()
            .map(|h| h.line_start)
            .collect::<Vec<u32>>()
    };
    assert_eq!(lines("$O.$M($$$A)"), vec![1u32, 2, 3]);
    assert_eq!(lines("obj.$M($$$A)"), vec![1u32]);
    assert_eq!(lines("$O.helper($$$A)"), vec![1u32]);
    assert_eq!(lines("this.render($$$A)"), vec![3u32]);
    assert_eq!(lines("helper($$$A)"), Vec::<u32>::new());
    assert_eq!(lines("$F($$$A)"), vec![1u32, 2, 3]);
}

/// Ruby receiver calls count as path calls only when they carry a
/// parenthesized argument list and a `.` operator. sg probes:
/// `Foo.bar($$$A)` [2], `$O.$M($$$A)` [2]; `bar($$$A)`, `$F($$$A)`,
/// `upcase($$$A)` all [] (bare `text.upcase` and operator `"a" + "b"` are not
/// call shapes — the pass-15 oracle-exact pins stand).
#[test]
fn ruby_receiver_calls_with_arguments_match_path_patterns() {
    let source = "def doit(x)\n  Foo.bar(1)\n  text.upcase\n  \"a\" + \"b\"\nend\n";
    let lines = |pattern: &str| {
        match_pattern(Language::Ruby, source, pattern)
            .unwrap()
            .iter()
            .map(|h| h.line_start)
            .collect::<Vec<u32>>()
    };
    assert_eq!(lines("Foo.bar($$$A)"), vec![2u32]);
    assert_eq!(lines("$O.$M($$$A)"), vec![2u32]);
    assert_eq!(lines("bar($$$A)"), Vec::<u32>::new());
    assert_eq!(lines("$F($$$A)"), Vec::<u32>::new());
    assert_eq!(lines("upcase($$$A)"), Vec::<u32>::new());
}

// ---------------------------------------------------------------------------
// H-CONF-025 (pass 43): repeated metavariable names UNIFY (sg semantics).
// A name used twice in one pattern is ONE variable bound once; binding it to
// a second, different text must reject the candidate. The subject previously
// bound repeated names independently (each occurrence its own wildcard) and
// over-matched every registered face (FUZZ38-R1-DUPMETA-{PY,GO,TS}).
// ---------------------------------------------------------------------------

/// Registered repro: `def $A($A): $$$C` py — subject matched greet.py:1,6
/// where pinned sg returns [] (name node text != parameter text).
#[test]
fn dupmeta_py_def_suite_rejects_independent_binding() {
    let source = "def greet(name):\n    msg = \"hello \" + name\n    return msg\n\n\ndef shout(text):\n    return text.upper()\n";
    let hits = match_pattern(Language::Python, source, "def $A($A): $$$C").unwrap();
    assert!(
        hits.is_empty(),
        "duplicate $A (decl name vs parameter) must unify and reject: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

/// Registered repro: `func $A() { $A }` go — subject matched server.go:9
/// (main) where pinned sg returns [] (decl name != body statement).
#[test]
fn dupmeta_go_func_body_rejects_independent_binding() {
    let source = "package server\n\nimport \"fmt\"\n\nfunc Greet(name string) string {\n\treturn \"hello \" + name\n}\n\nfunc main() {\n\tfmt.Println(Greet(\"world\"))\n}\n";
    let hits = match_pattern(Language::Go, source, "func $A() { $A }").unwrap();
    assert!(
        hits.is_empty(),
        "duplicate $A (decl name vs body) must unify and reject: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

/// Registered repro: `function $A($B) { $A }` ts — subject matched util.ts:5
/// where pinned sg returns [] ($A reused across decl and body; $B binds once).
#[test]
fn dupmeta_ts_function_body_rejects_independent_binding() {
    let source = "export function helper(x: number) {\n  console.log(x)\n}\n";
    let hits = match_pattern(Language::TypeScript, source, "function $A($B) { $A }").unwrap();
    assert!(
        hits.is_empty(),
        "duplicate $A (decl name vs body) must unify and reject: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

/// Positive control (sg agrees): when every occurrence of the repeated name
/// binds the SAME text, unification holds and the match survives — and the
/// unified capture is exposed once in the envelope.
#[test]
fn dupmeta_equal_text_unification_still_matches() {
    let source = "def dup(dup):\n    return dup\n";
    let hits = match_pattern(Language::Python, source, "def $A($A): $$$C").unwrap();
    assert_eq!(
        hits.len(),
        1,
        "equal-text unification must still match exactly once: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("dup"));
}

/// Repeated argument name: `$F($A, $A)` requires both arguments to carry the
/// same text (sg unification); distinct-text args must reject.
#[test]
fn dupmeta_repeated_argument_name_requires_equal_text() {
    let source = "fn main() {\n    let a = add(1, 2);\n    let b = add(3, 3);\n}\n";
    let hits = match_pattern(Language::Rust, source, "add($A, $A)").unwrap();
    assert_eq!(
        hits.len(),
        1,
        "add(3, 3) must match, add(1, 2) must reject: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].line_start, 3);
}

// ---------------------------------------------------------------------------
// H-CONF-021 (pass 43): `$`-less literal-content patterns must MATCH nodes
// whose complete text equals the pattern. The literal lane previously pushed
// only identifier-kind nodes, so these registered faces silently answered
// `ok:true` empty where pinned sg matches — a fail-open.
// ---------------------------------------------------------------------------

/// Registered face: `pattern:1` rust — sg matches the number literal;
/// subject answered empty.
#[test]
fn literal_content_number_pattern_matches_rust() {
    let source = "fn calc() -> i32 {\n    1\n}\n";
    let hits = match_pattern(Language::Rust, source, "1").unwrap();
    assert_eq!(
        hits.len(),
        1,
        "number literal must match its node: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].line_start, 2);
}

/// Registered face: `greet("world")` py — sg matches the inner call.
#[test]
fn literal_content_call_pattern_matches_python() {
    let source = "def main():\n    print(greet(\"world\"))\n";
    let hits = match_pattern(Language::Python, source, "greet(\"world\")").unwrap();
    assert_eq!(
        hits.len(),
        1,
        "literal-argument call must match: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].line_start, 2);
}

/// Registered face: `add(1, 2)` rust — sg matches the two-arg call.
#[test]
fn literal_content_call_pattern_matches_rust() {
    let source = "fn main() {\n    let s = add(1, 2);\n}\n";
    let hits = match_pattern(Language::Rust, source, "add(1, 2)").unwrap();
    assert_eq!(
        hits.len(),
        1,
        "two-arg literal call must match: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].line_start, 2);
}

/// Registered face: `text.upper()` py — sg matches the zero-arg member call.
#[test]
fn literal_content_member_call_pattern_matches_python() {
    let source = "def shout(text):\n    return text.upper()\n";
    let hits = match_pattern(Language::Python, source, "text.upper()").unwrap();
    assert_eq!(
        hits.len(),
        1,
        "member call must match: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].line_start, 2);
}

/// Registered family face: `return msg` py — the full statement node text.
#[test]
fn literal_content_return_statement_matches_python() {
    let source = "def greet(name):\n    return msg\n";
    let hits = match_pattern(Language::Python, source, "return msg").unwrap();
    assert_eq!(
        hits.len(),
        1,
        "full statement text must match: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].line_start, 2);
}

/// Pass-30 interplay guard: a valid `$`-less pattern with no equal-text node
/// stays a valid EMPTY result (the fail-closed guard still exempts `$`-less
/// patterns; the literal lane matching content never turns it into an error).
#[test]
fn literal_content_pattern_without_equal_node_stays_ok_empty() {
    let source = "fn main() {}\n";
    assert!(match_pattern(Language::Rust, source, "zzz_no_such_content")
        .unwrap()
        .is_empty());
}

/// Identifier-literal lane is unchanged: exact ident text still matches
/// (case-sensitive), including decl-level name matches.
#[test]
fn literal_identifier_lane_unchanged() {
    let source = "fn Foo() {}\nfn foo() {}\n";
    let lower = match_pattern(Language::Rust, source, "foo").unwrap();
    assert!(!lower.is_empty());
    assert!(lower.iter().all(|h| h.line_start == 2));
}

// ---------------------------------------------------------------------------
// PASS 51 (r9-remediation): the duplicate-metavariable family BEYOND the three
// declaration shapes the pass-43 fix covered. Pass 48 (fresh-eyes B, ledger
// CONFORMANCE_NEGATIVE_RESULTS §9) found 6 TrueDivergences where the subject
// failed CLOSED ("requires structural fallback") while pinned sg 0.45.2
// answered with unified matches, plus the `$O.$O($$$A)` receiver-equality
// overmatch. Every expected set below is a live probe of the pinned oracle
// (ast-grep 0.45.2, probed 2026-09-04) against these exact sources.
// ---------------------------------------------------------------------------

fn lines_of(hits: &[ast_sgrep_lang::PatternMatch]) -> Vec<u32> {
    hits.iter().map(|h| h.line_start).collect()
}

/// Klass projection: the sorted DISTINCT line set (the oracle harness
/// compares (file, line) pairs, so same-line outer/inner emission
/// multiplicity is invisible at the parity contract level).
fn klass_lines(hits: &[ast_sgrep_lang::PatternMatch]) -> Vec<u32> {
    let mut lines: Vec<u32> = hits.iter().map(|h| h.line_start).collect();
    lines.sort_unstable();
    lines.dedup();
    lines
}

/// P48-V1 (ledger `p48-dupmeta-general-family-failclosed-where-sg-answers`):
/// `$A == $A == $A` python — operator-chain expression with a repeated
/// metavariable. sg: 1 hit, A=x.
#[test]
fn dupmeta_py_comparison_chain_matches_with_unification() {
    assert!(!needs_ast_grep_fallback("$A == $A == $A"));
    let source = "x = 1\nif x == x == x:\n    chained = True\n";
    let hits = match_pattern(Language::Python, source, "$A == $A == $A").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2u32],
        "comparison chain must match its node exactly once: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("x"));
}

/// P48-V2: `foo($A, bar($A))` python — nested call inside an argument list.
/// sg: 1 hit, A=arg.
#[test]
fn dupmeta_py_nested_call_argument_matches_with_unification() {
    assert!(!needs_ast_grep_fallback("foo($A, bar($A))"));
    let source = "def f(arg):\n    return foo(arg, bar(arg))\n";
    let hits = match_pattern(Language::Python, source, "foo($A, bar($A))").unwrap();
    assert_eq!(lines_of(&hits), vec![2u32], "{:?}", hits);
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("arg"));
}

/// P48-V5: `fn $A($A: u32) -> u32 { $A }` rust — typed param + return type +
/// single-statement body, name repeated across all three positions. sg: 1 hit
/// (the unifying fn), A=dup; the distinct-text twin must reject.
#[test]
fn dupmeta_rust_typed_param_return_body_unifies() {
    assert!(!needs_ast_grep_fallback("fn $A($A: u32) -> u32 { $A }"));
    let source =
        "fn dup(dup: u32) -> u32 {\n    dup\n}\n\nfn other(name: u32) -> u32 {\n    name\n}\n";
    let hits = match_pattern(Language::Rust, source, "fn $A($A: u32) -> u32 { $A }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1u32],
        "only the fn whose name/param/body all say `dup` may match: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("dup"));
}

/// P48-V6: `Some($A).unwrap_or($A)` rust — method call whose receiver is
/// itself a call (callee containing a parenthesized sub-call). sg: 1 hit, A=7;
/// the unequal-arg twin must reject.
#[test]
fn dupmeta_rust_method_chain_on_call_unifies() {
    assert!(!needs_ast_grep_fallback("Some($A).unwrap_or($A)"));
    let source =
        "fn main() {\n    let v = Some(7).unwrap_or(7);\n    let w = Some(7).unwrap_or(9);\n}\n";
    let hits = match_pattern(Language::Rust, source, "Some($A).unwrap_or($A)").unwrap();
    assert_eq!(lines_of(&hits), vec![2u32], "{:?}", hits);
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("7"));
}

/// P48-V7: `function $A($B) { return $A; }` typescript — decl whose body is a
/// return statement. sg: 1 hit, A=rec, B=`pivot: number` (sg binds the WHOLE
/// parameter — the whole-node-text metavariable rule); the name/body mismatch
/// twin and the non-return twin must reject.
#[test]
fn dupmeta_ts_return_body_unifies_and_binds_whole_param() {
    assert!(!needs_ast_grep_fallback("function $A($B) { return $A; }"));
    let source = "function rec(pivot: number) {\n  return rec;\n}\n\nfunction other(pivot: number) {\n  return rec;\n}\n\nfunction rec2(p: number) {\n  rec2;\n}\n";
    let hits = match_pattern(
        Language::TypeScript,
        source,
        "function $A($B) { return $A; }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1u32],
        "only the rec/rec self-return may match: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("rec"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("pivot: number"),
        "a param metavariable binds the whole parameter node (sg semantics)"
    );
}

/// P48-V8: `func $A() { _ = $A }` go — blank-identifier assignment body.
/// sg: 2 hits (dup, helper), A bound per site; `_ = 7` must reject.
#[test]
fn dupmeta_go_blank_assignment_body_unifies() {
    assert!(!needs_ast_grep_fallback("func $A() { _ = $A }"));
    let source = "func dup() {\n\t_ = dup\n}\n\nfunc helper() {\n\t_ = helper\n}\n\nfunc other() {\n\t_ = 7\n}\n";
    let hits = match_pattern(Language::Go, source, "func $A() { _ = $A }").unwrap();
    assert_eq!(lines_of(&hits), vec![1u32, 5], "{:?}", hits);
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("dup"));
    assert_eq!(
        hits[1].captures.get("A").map(String::as_str),
        Some("helper")
    );
}

/// P48-V3 (ledger `p48-dollarO-dollarO-py-positive-face-overmatch`):
/// `$O.$O($$$A)` python — the receiver position must bind through
/// `bind_capture` (including the whole-multi-segment-receiver rule), so only
/// receiver==method sites survive. sg: exactly `builder.builder()`, O=builder.
#[test]
fn dupmeta_py_member_call_receiver_method_equality() {
    let source = "obj = Obj()\nobj.method()\nobj.other()\nbuilder = Wrap()\nbuilder.builder()\na = Ambient()\nresult = a.b.c(1)\nmissing = a.b.d(1)\nsame = Trip()\nsame.same.same(1)\n";
    let hits = match_pattern(Language::Python, source, "$O.$O($$$A)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![5u32],
        "receiver/method equality must leave only builder.builder(): {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(
        hits[0].captures.get("O").map(String::as_str),
        Some("builder")
    );
}

/// Same family face: `foo($X + 1)` rust — literal arithmetic inside an
/// argument list (the expression-content face of the P48-V2 shape). sg: hits
/// only the `+ 1` call, X=x.
#[test]
fn dupmeta_rust_nested_call_with_literal_arg() {
    assert!(!needs_ast_grep_fallback("foo($X + 1)"));
    let source = "fn main() {\n    foo(x + 1);\n    foo(x + 2);\n}\n";
    let hits = match_pattern(Language::Rust, source, "foo($X + 1)").unwrap();
    assert_eq!(lines_of(&hits), vec![2u32], "{:?}", hits);
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("x"));
}

/// The registered fail-closed CONTRACT rows (cases.jsonl subject_expect=
/// fail_closed) must stay loud rejections: the general expression lane may not
/// swallow declaration/binding keywords outside the native prefix set, `$$$`
/// templates, comment tails, or multi-line declaration tails.
/// PASS 122 (F4) premise correction, disclosed: `let $A = $B` LEFT this list —
/// sg 0.45.2 ANSWERS the spelling on js AND rust match files (first-hand grid
/// /tmp/phase122/f4, n1 each), so the js/ts `lexical_declaration` root is now
/// admitted and served; the RUST per-file census class stays census-loud
/// (rust builds root at `let_statement`, still refused by the root-kind
/// gate), and the walk-only assertion below still pins the rust face empty.
#[test]
fn registered_fail_closed_spellings_stay_fail_closed() {
    for pattern in [
        "fn $A($$$B) -> $C { $$$D }",
        "RETURN $A",
        "return $B// noteA",
        "def $A($B):\n    return $C",
        "int $A($B) { $$$C }",
        "fun $A() { $$$B }",
        "if ($COND) { $A; $B }",
        "$)(",
    ] {
        assert!(
            needs_ast_grep_fallback(pattern),
            "{pattern} must stay fail-closed"
        );
        assert!(
            match_pattern(Language::Rust, "fn foo() {}", pattern)
                .unwrap()
                .is_empty(),
            "{pattern} must match nothing natively"
        );
    }
    // PASS 122: `let $A = $B` left the ingress-fail-closed list above (sg
    // ANSWERS the js/rust spellings — see the doc correction); its rust
    // walk-only face is still pinned empty here.
    assert!(
        match_pattern(Language::Rust, "fn foo() {}", "let $A = $B")
            .unwrap()
            .is_empty(),
        "rust let face must still match nothing natively"
    );
}

// ---------------------------------------------------------------------------
// PASS 54 (r9-design-implementation): H-CONF-022 metavar grammar v2 +
// H-CONF-021 literal isomorphic matching (rule R3). Every expected class/set
// below is a live probe of the pinned oracle (ast-grep 0.45.2) recorded in the
// gauntlet workspace artifacts/conformance/pass53/ matrices and re-probed
// 2026-09-04 before these cells were written. Failure-first: every new cell
// ran RED against the pass-51 tree except the explicitly-guarded preservation
// rows (canonical controls, loud residuals, T-B4/T-B5/T-B6 mutation guards).
// ---------------------------------------------------------------------------

// -- H-CONF-022 v2 grammar --------------------------------------------------
//
// Canonical NAME := ASCII [A-Z_][A-Z0-9_]* (tail grammar CORRECTED by
// F26-0182, pass 69a — the pass-34/53/54 pin `[A-Za-z0-9_]*` tail admitted
// lowercase tails sg's tokenizer rejects; see f26_0182_... below). A
// NON-canonical `$`-token is a NON-canonical metavariable: in py/rust-class
// languages sg parses it into an ERROR node of the pattern tree that matches
// nothing (accepted-empty, or exit 8 where per-language error recovery
// fails). The subject must answer ok:true empty on every such face — never
// the pass-51 general-lane wildcard overmatch (pass-53 matrix: `greet($a)`
// 10 live hits, `$o($A)` 13). In `$`-name languages (js/ts/php-lowercase)
// the same tokens are LITERAL code and answer through the literal lane
// (F68a-1, pass 69a — see f68a_1_... below).

/// v2 row 5: lowercase in argument position — zero candidates, native ingress.
/// sg: accepted-empty on the same probe corpus.
#[test]
fn metavar_v2_lowercase_arg_is_never_match_not_wildcard() {
    assert!(!needs_ast_grep_fallback("greet($a)"), "valid ingress");
    let source =
        "def greet(name):\n    return name\nr1 = greet(\"world\")\nr2 = greet(  \"world\"  )\n";
    let hits = match_pattern(Language::Python, source, "greet($a)").unwrap();
    assert!(
        hits.is_empty(),
        "lowercase-led $a must match nothing: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

/// v2 row 5: lowercase callee faces (`$o($A)`, `$a(1)`) — the general lane
/// must not turn them into wildcard member/arity calls (13 live overmatches
/// on the pass-53 corpus). sg: exit 8 (residual empty-vs-error divergence —
/// the subject keeps ok:true empty, never an overmatch).
#[test]
fn metavar_v2_lowercase_callee_never_match() {
    assert!(!needs_ast_grep_fallback("$o($A)"));
    assert!(!needs_ast_grep_fallback("$a(1)"));
    let source = "t = (1, 2)\nt.t(1)\nq = a(1)\nobj = Obj()\nobj.method()\n";
    for pattern in ["$o($A)", "$a(1)"] {
        let hits = match_pattern(Language::Python, source, pattern).unwrap();
        assert!(
            hits.is_empty(),
            "{pattern} must match nothing: {:?}",
            hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
        );
    }
}

/// v2 row 5: statement-leading lowercase assignment — the general lane's
/// assignment root previously absorbed every assignment on the corpus.
#[test]
fn metavar_v2_lowercase_stmt_leading_never_match() {
    assert!(!needs_ast_grep_fallback("$a = 1"));
    let source = "a = 1\nb = 2\n";
    assert!(match_pattern(Language::Python, source, "$a = 1")
        .unwrap()
        .is_empty());
}

/// v2 row 5: lowercase in a declaration-name position with a canonical `$$$`
/// body. sg: accepted-empty; the held-out literal control `def a(x): $$$B`
/// MATCHES the same def, so the `$` prefix alone kills the match — error-node
/// semantics, refuting the pass-34 literal-ident reading.
#[test]
fn metavar_v2_lowercase_decl_ingress_native_zero_hits() {
    assert!(!needs_ast_grep_fallback("def $a(x): $$$B"));
    let source = "def a(x):\n    a = 1\n    return a\n";
    assert!(match_pattern(Language::Python, source, "def $a(x): $$$B")
        .unwrap()
        .is_empty());
}

/// v2 rows 4/5: lowercase after `$$$` and mixed-case tails. `foo($$$a)` was a
/// live wildcard-args overmatch; `let $aB = 1;` keeps the loud class today
/// and must become a native zero-hit face.
#[test]
fn metavar_v2_lowercase_multi_and_mixed_never_match() {
    assert!(!needs_ast_grep_fallback("foo($$$a)"));
    assert!(!needs_ast_grep_fallback("let $aB = 1;"));
    let source = "fn main() {\n    foo(1);\n    foo(1, 2);\n    let ab = 1;\n}\n";
    for pattern in ["foo($$$a)", "let $aB = 1;"] {
        assert!(
            match_pattern(Language::Rust, source, pattern)
                .unwrap()
                .is_empty(),
            "{pattern} must match nothing"
        );
    }
}

/// v2 row 10: non-ASCII `$Ü` — the unicode-wide `is_pattern_ident` made
/// `greet($Ü)` a live overmatch; the ASCII check closes it. The garbage-led
/// token keeps the LOUD fail-closed class (not a silent empty).
#[test]
fn metavar_v2_non_ascii_metavar_closes_overmatch_and_stays_loud() {
    let source = "def greet(name):\n    return name\nr = greet(\"world\")\n";
    assert!(match_pattern(Language::Python, source, "greet($Ü)")
        .unwrap()
        .is_empty());
    assert!(needs_ast_grep_fallback("greet($Ü)"));
}

/// v2 row 6/7 (THE HOLE): `fn $3.14() { }` classified as
/// `Function{name:None, body:Exactly(0)}` — a wildcard empty-body fn that
/// silently matched `fn x() {}`. Must be a loud reject (sg exit-8 class).
#[test]
fn metavar_v2_garbage_fn_head_is_loud_reject_never_wildcard() {
    let source = "fn x() {}\nfn y() { 1 }\n";
    for pattern in ["fn $3.14() { }", "fn $0x1F() { }"] {
        assert!(needs_ast_grep_fallback(pattern), "{pattern} must stay loud");
        assert!(
            match_pattern(Language::Rust, source, pattern)
                .unwrap()
                .is_empty(),
            "{pattern} must not wildcard-match an empty-body fn"
        );
    }
}

/// v2 row 2: `$$A` is sg's UNIVERSAL node metavar — matches EVERY node
/// including the module docstring (line 1), comments, strings, and anonymous
/// tokens (344 raw sg rows on the pass-53 py corpus). The subject answered
/// loud exit-2 — a divergence.
#[test]
fn metavar_v2_universal_dollar_dollar_matches_every_node() {
    let source = "x = 1\ny = greet(\"world\")\n";
    let docstring_src =
        "\"\"\"module docstring\nline two\n\"\"\"\n# a comment\nz = greet(\"world\")\n";
    let hits = match_pattern(Language::Python, source, "$$A").unwrap();
    assert!(
        !hits.is_empty(),
        "$$A must be a universal match, not a loud reject"
    );
    let doc_hits = match_pattern(Language::Python, docstring_src, "$$A").unwrap();
    assert!(
        doc_hits.iter().any(|h| h.line_start == 1),
        "docstring node must match: {:?}",
        doc_hits.iter().map(|h| h.line_start).collect::<Vec<_>>()
    );
    assert!(
        doc_hits.iter().any(|h| h.excerpt.contains("# a comment")),
        "comment nodes must match"
    );
    assert!(
        doc_hits.iter().any(|h| h.excerpt == "("),
        "anonymous tokens must match"
    );
    let anon = match_pattern(Language::Python, source, "$$_").unwrap();
    assert_eq!(
        hits.len(),
        anon.len(),
        "$$_ must have the same row count as $$A (sg held-out cell)"
    );
}

/// v2 row 2 interplay: `$$MATCH` keeps the reserved-envelope overwrite
/// semantics — every universal match carries MATCH == the matched node text.
#[test]
fn metavar_v2_universal_match_reserved_key_overwrite_preserved() {
    let source = "x = 1\ny = 2\n";
    let hits = match_pattern(Language::Python, source, "$$MATCH").unwrap();
    assert!(!hits.is_empty(), "$$MATCH must be a universal match");
    for hit in &hits {
        assert_eq!(
            hit.captures.get("MATCH").map(String::as_str),
            Some(hit.excerpt.as_str()),
            "reserved MATCH key must carry the node text: {:?}",
            hit.excerpt
        );
    }
}

/// v2 rows 3/4/8/9: `$$`, bare `$`, `$$$` keep the loud-reject class
/// (registered residuals — sg gives accepted-empty/exit 8 there; the subject
/// never overmatches and never answers a silent empty).
/// PASS 71a cell correction (70c-F3, sg 0.45.2 re-probed 2026-09-06 with the
/// literal faces present): `$$a`'s registered LOUD basis — "sg answers
/// nothing there" — is REFUTED for js/ts/php (sg answers `$$x + 2`,
/// `$$tot + 1` … literally), and the F3 remediation routes the whole
/// 2/3-dollar NON-canonical family through the same language-free gate as
/// the 1-dollar family: lane languages answer sg's literal faces, rejecting
/// languages (py rc=8, rust ERROR-node, php poison) fall to the registered
/// NeverMatches accepted-empty class — byte-identical to the §19.1
/// py/rust parse-empty cells. `$$a` therefore asserts the corrected posture
/// here (empty, admitted ingress) instead of loud; the register owes the
/// loudness-class rider for the `$$`-led rows.
#[test]
fn metavar_v2_dollar_garbage_stay_loud() {
    for pattern in ["$$", "$", "$$$"] {
        assert!(needs_ast_grep_fallback(pattern), "{pattern} must stay loud");
        assert!(match_pattern(Language::Rust, "fn x() {}\n", pattern)
            .unwrap()
            .is_empty());
    }
    for pattern in ["$$a", "$$x"] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} rides the corrected 2-dollar gate (admitted ingress)"
        );
        assert!(
            match_pattern(Language::Rust, "fn x() {}\n", pattern)
                .unwrap()
                .is_empty(),
            "{pattern} must answer nothing in rust (sg ERROR-node face)"
        );
    }
}

/// v2 canonical controls (GREEN pre-fix by design): `def $A($$$B): $$$C`,
/// `def $_($$$B): $$$C`, `$F($$$A)` keep their exact row sets while the
/// grammar tightens around them.
#[test]
fn metavar_v2_canonical_controls_byte_unchanged() {
    let source = "def greet(name):\n    return name\n\ndef shout(text):\n    return text.upper()\n";
    assert_eq!(
        lines_of(&match_pattern(Language::Python, source, "def $A($$$B): $$$C").unwrap()),
        vec![1u32, 4]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::Python, source, "def $_($$$B): $$$C").unwrap()),
        vec![1u32, 4]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::Python, source, "$F($$$A)").unwrap()),
        vec![5u32]
    );
}

/// The pass-53 held-out discriminating pair, subject side: `$a` face becomes
/// ok:true empty; the literal-name face keeps its PRE-EXISTING loud class
/// (literal-argument decl templates are outside this design — guarded so this
/// pass cannot change it silently).
#[test]
fn metavar_v2_lowercase_def_empty_literal_def_control_unchanged() {
    assert!(!needs_ast_grep_fallback("def $a(x): $$$B"));
    assert!(needs_ast_grep_fallback("def a(x): $$$B"));
    let source = "def a(x):\n    a = 1\n    return a\n";
    assert!(match_pattern(Language::Python, source, "def $a(x): $$$B")
        .unwrap()
        .is_empty());
}

// -- H-CONF-021 rule R3 (literal isomorphic matching) -----------------------
//
// A `$`-less literal pattern matches a candidate iff their trees are
// isomorphic: whitespace of every kind and code-side trailing commas are
// invisible; comments are invisible iff attached INSIDE an arguments node
// (a comment child of the call node itself blocks); a PATTERN-side trailing
// comma is significant; any AST-kind delta blocks. The pass-43 exact-text
// arm stays the fast path; this arm only ADDS matches.

/// Compact r-variant corpus mirroring the pass-53 py probe layout.
const R3_PY: &str = "def greet(name):\n    return name\n\ndef callers():\n    r1 = greet(\"world\")\n    r2 = greet(  \"world\"  )\n    r3 = greet(\n        \"world\",\n    )\n    r4 = greet(  # inline comment\n        \"world\",\n    )\n    r5 = greet(\"world\", \"extra\")\n    r6 = greet(\n        \"wor\" \"ld\",\n    )\n    r7 = greet(\n\n        \"world\"\n    )\n    r8 = greet(\n        \"world\",\n        # trailing comment after arg\n    )\n    r9 = greet (\"world\")\n    return r1\n";

/// Compact x-variant corpus mirroring the pass-53 rs probe layout.
const R3_RS: &str = "fn calc(one: i32, two: i32) -> i32 {\n    let x1 = calc(1, 2);\n    let x2 = calc( 1, 2 );\n    let x3 = calc(\n        1,\n        2,\n    );\n    let x4 = calc(1 /* one */, 2);\n    let x5 = calc(1, 2, 3);\n    let x6 = calc(\n        1, // trailing line comment\n        2,\n    );\n    let x7 = calc /* between */ (1, 2);\n    let x8 = calc(/* lead */ 1, 2);\n    let x9 = calc(1, 2,);\n    let x10 = calc(\n        1,\n        2\n    );\n    x1 + x2\n}\n";

/// sg set on the py corpus is {18,19,20,30,59,67,72,73}: exact, spaced,
/// multiline+comma, blank lines, trailing-comment-inside-args, and
/// callee-paren-gap variants ALL match (T-B1 + T-B2 + whitespace faces).
/// One scoped residual: sg's python fork also EXCLUDES the pre-first-arg
/// inline-comment face (r4) because sg attaches that comment at the call
/// node, while our vendored tree-sitter-python attaches it inside
/// `argument_list` — under the attachment rule the face matches (line 10
/// below). Blocking it positionally would break sg's rust x13 face, which
/// our tree also attaches inside the container; recorded as a registered
/// grammar-attachment variance, never a silent overmatch.
#[test]
fn literal_r3_whitespace_variants_match_structurally() {
    let hits = match_pattern(Language::Python, R3_PY, "greet(\"world\")").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![5u32, 6, 7, 10, 17, 21, 25],
        "T-B1/T-B2: spaced, multiline, blank-line, trailing-comment, and paren-gap variants must match (line 10 = the documented py-attachment residual): {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

/// sg set on the rs corpus includes x4 (block comment beside an arg) and x13
/// (leading block comment INSIDE the arguments) — tree attachment inside the
/// arguments node makes comments invisible (T-B3).
#[test]
fn literal_r3_comment_inside_arguments_is_invisible() {
    let hits = match_pattern(Language::Rust, R3_RS, "calc(1, 2)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2u32, 3, 4, 8, 10, 15, 16, 17],
        "T-B3: comment-inside-args sites must match; exact/spaced/multiline keep matching: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

/// sg x7 face: a comment between callee and paren is a child of the CALL node
/// itself — it BLOCKS the match (mutation-kill guard for the comment guard).
/// The py pre-first-arg face is the documented grammar-attachment variance
/// (our tree-sitter-python attaches it inside the arguments container, sg's
/// fork at the call node) — asserted at its honest value, never silent.
#[test]
fn literal_r3_comment_on_call_node_blocks() {
    let rs = match_pattern(Language::Rust, R3_RS, "calc(1, 2)").unwrap();
    assert!(
        !lines_of(&rs).contains(&14u32),
        "T-B4: comment-on-call-node site must NOT match: {:?}",
        rs.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    let py = match_pattern(Language::Python, R3_PY, "greet(\"world\")").unwrap();
    assert!(
        lines_of(&py).contains(&10u32),
        "py pre-first-arg comment attaches inside the arguments container in our tree (sg's fork attaches it at the call node — registered variance): {:?}",
        py.iter().map(|h| h.line_start).collect::<Vec<_>>()
    );
}

/// sg x5/r5/r6 faces: arg-count deltas, implicit string concatenation, and
/// keyword-argument shapes block the match (AST-kind sensitivity).
#[test]
fn literal_r3_ast_deltas_block() {
    let rs = match_pattern(Language::Rust, R3_RS, "calc(1, 2)").unwrap();
    assert!(
        !lines_of(&rs).contains(&9u32),
        "3-arg call must not match a 2-arg pattern"
    );
    let py = match_pattern(Language::Python, R3_PY, "greet(\"world\")").unwrap();
    for line in [13u32, 14] {
        assert!(
            !lines_of(&py).contains(&line),
            "line {line} (extra arg / concatenated_string) must not match"
        );
    }
}

/// sg round-2 P2 cell: a trailing comma in the PATTERN is significant — it
/// matches ONLY the sites whose source carries one ({x3, x6, x9} here; sg
/// {8,14,43} on the probe corpus); the comma-less multiline site is excluded.
#[test]
fn literal_r3_pattern_trailing_comma_is_significant() {
    let hits = match_pattern(Language::Rust, R3_RS, "calc(1, 2,)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![4u32, 10, 16],
        "T-B7: only trailing-comma sites may match: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

/// sg training cell: a spaced PATTERN parses to the same AST — identical
/// match set to the plain pattern.
#[test]
fn literal_r3_spaced_pattern_matches_like_plain() {
    let plain = match_pattern(Language::Rust, R3_RS, "calc(1, 2)").unwrap();
    let spaced = match_pattern(Language::Rust, R3_RS, "calc( 1 , 2 )").unwrap();
    assert_eq!(lines_of(&plain), lines_of(&spaced));
    assert_eq!(plain.len(), 8, "plain set must be the full R3 set");
}

// ---------------------------------------------------------------------------
// PASS 60 (r11-remediation): the r10 findings, priority-ordered.
// H-CONF-029 (regression-class fail-opens), H-CONF-026 (single-vs-rest
// namespaces), F58-3 (chained-receiver overmatch), F58-1 (comment-in-string),
// F58-2 (statement-root heads), H-CONF-027/028/030 (java expression +
// R3 residuals). Every expected set is a live probe of the pinned oracle
// (ast-grep 0.45.2) captured in the gauntlet workspace
// artifacts/conformance/pass60/pass60_probe_pre.json (2026-09-05) and the
// pass-58/59 FailureBundles. Failure-first: every cell below ran RED against
// the pass-59 tree (sha16 e424ca9cac2ff926) except the explicitly-guarded
// preservation rows at the end of the section.
// ---------------------------------------------------------------------------

/// H-CONF-026: sg keeps single and multi metavariable namespaces DISTINCT
/// (`metaVariables.single.B` and `metaVariables.multi.B` coexist), so a name
/// used BOTH as `$B` and `$$$B` is two variables, not one. The pass-43
/// `bind_capture` unification rejected such candidates outright — a silent
/// under-match where sg answers (6 fuzz TDs, seed 20260905).
#[test]
fn hconf026_single_and_rest_namespaces_are_distinct() {
    // Control: distinct names keep matching (the pass-43 face is unchanged).
    let src = "fn main() {\n    done();\n}\n";
    assert_eq!(
        lines_of(&match_pattern(Language::Rust, src, "fn $B() { $$$C }").unwrap()),
        vec![1u32],
        "distinct-name control must keep matching"
    );
    // THE face: same name, single + rest — texts necessarily differ.
    let hits = match_pattern(Language::Rust, src, "fn $B() { $$$B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1u32],
        "single-B (name) and multi-B (body) are DISTINCT namespaces (sg): {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].captures.get("B").map(String::as_str), Some("main"));
    assert_eq!(
        hits[0].captures.get("$$$B").map(String::as_str),
        Some("done();"),
        "the rest binding keys its own namespace"
    );
    // The single-single unification contract (pass-43) is NOT relaxed:
    // one name bound to two different texts still rejects.
    let twin = "fn main() {\n    main();\n}\nfn other() {\n    main();\n}\n";
    let uni = match_pattern(Language::Rust, twin, "fn $A() { $A(); }").unwrap();
    assert_eq!(
        lines_of(&uni),
        vec![1u32],
        "single-single unification survives"
    );
}

/// F58-3: the whole-receiver binding must be FAITHFUL to the receiver node.
/// When the receiver is itself a call, the flattened tail segments have no
/// faithful text; sg decomposes pattern callees into plain member chains and
/// refuses call-carrying receivers (sg matches `b.b(1)` only — pass-58 v3
/// corpus probe: sg {1} vs subject {1,2,3}).
#[test]
fn f58_3_chained_call_receiver_never_unifies() {
    let src = "b.b(1)\ny.first().first()\nwrap().wrap()\na.b().c(2)\ng().h()\n";
    let hits = match_pattern(Language::Python, src, "$O.$O($$$A)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1u32],
        "only the plain member chain may match; receiver-is-call must reject: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    // Flat controls stay green (pass-58 2b).
    let flat = "same = Trip()\nsame.same.same(1)\nz.z.z.z(4)\nobj = Obj()\nobj.method(k)\n";
    let flat_hits = match_pattern(Language::Python, flat, "$O.$O($$$A)").unwrap();
    assert!(
        flat_hits.is_empty(),
        "flat multi-segment controls must stay rejected: {:?}",
        flat_hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

/// F58-1: comment syntax INSIDE a string literal is string content, not a
/// comment — the general-lane guard must not fail `parse($U, "https://default")`
/// closed where sg answers the site (url.py:1, pass-58 FailureBundle F58-1).
#[test]
fn f58_1_comment_syntax_inside_string_is_content() {
    assert!(
        !needs_ast_grep_fallback("parse($U, \"https://default\")"),
        "URL slashes are string content; the pattern must stay native"
    );
    let src = "p1 = parse(u, \"https://default\")\np3 = parse(w, \"plain\")\n";
    let hits = match_pattern(Language::Python, src, "parse($U, \"https://default\")").unwrap();
    assert_eq!(lines_of(&hits), vec![1u32], "{:?}", hits);
    assert_eq!(hits[0].captures.get("U").map(String::as_str), Some("u"));
}

/// F58-2: a lowercase statement-root head (`return $A`) is sg-valid — sg
/// answers return statements (v2.py:5,10; java/ts answered; rust
/// accepted-empty). The bare-keyword head guard reserved everything outside
/// DECL_PATTERN_PREFIXES for the loud contract, so the subject failed closed
/// where sg answers (pass-58 FailureBundle F58-2).
#[test]
fn f58_2_return_head_is_a_native_statement_template() {
    assert!(
        !needs_ast_grep_fallback("return $A"),
        "lowercase `return` + metavar is sg-valid ingress"
    );
    let py = "def f(x):\n    return x\n\ndef g():\n    return 7\n\nfast_return_a(1)\n";
    let hits = match_pattern(Language::Python, py, "return $A").unwrap();
    assert_eq!(lines_of(&hits), vec![2u32, 5], "{:?}", hits);
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("x"));
    // rust serves the shape too (sg accepted-empty there — 0 hits is parity).
    let rs = "fn f() -> i32 {\n    return 1;\n}\n";
    let rs_hits = match_pattern(Language::Rust, rs, "return $A").unwrap();
    assert_eq!(lines_of(&rs_hits), vec![2u32], "{:?}", rs_hits);
}

/// H-CONF-027: a meta head with a LITERAL tail binds and matches (java
/// `System.out.println(total)`, Main.java:8) — the pass-51 lane templated
/// meta tails but silently answered empty on literal tails (2 fuzz TDs).
#[test]
fn hconf027_meta_head_literal_tail_matches_java() {
    assert!(!needs_ast_grep_fallback("$O.out.println(total)"));
    let src = "public class Main {\n    public static void main(String[] args) {\n        int total = add(1, 2);\n        System.out.println(total);\n    }\n}\n";
    let hits = match_pattern(Language::Java, src, "$O.out.println(total)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![4u32],
        "meta head + literal member tail must bind O=System and match: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert_eq!(
        hits[0].captures.get("O").map(String::as_str),
        Some("System")
    );
}

/// H-CONF-028: java expression CONTENT answers through the general lane
/// (`$B + $A`) and the literal R3 arm (`return a + b`) where sg matches
/// Main.java:3 — python answered identical shapes, the gap was java-specific
/// (the `;`-less context wrapper could not parse java statements).
#[test]
fn hconf028_java_expression_content_matches() {
    assert!(!needs_ast_grep_fallback("$B + $A"));
    let src = "public class Main {\n    public static int add(int a, int b) {\n        return a + b;\n    }\n}\n";
    let gen = match_pattern(Language::Java, src, "$B + $A").unwrap();
    assert_eq!(
        lines_of(&gen),
        vec![3u32],
        "java binary-expression metavar content must match: {:?}",
        gen.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    let lit = match_pattern(Language::Java, src, "return a + b").unwrap();
    assert_eq!(
        lines_of(&lit),
        vec![3u32],
        "java literal return statement (R3 arm) must match: {:?}",
        lit.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

/// H-CONF-030 (i): a code-side trailing comma is invisible in the PYTHON
/// literal lane too — `beta(1, 2)` must match `beta(1, 2,)` (lit_py.py:7;
/// sg answers, the rust mirror already matched).
#[test]
fn hconf030_i_py_code_side_trailing_comma_is_invisible() {
    // Unit-level RED (pass 60): the search-layer byte prefilter must not
    // require the whole pattern text — the trailing comma face is invisible
    // to the R3 lane, so the prefilter literal must be a code token.
    assert_eq!(
        ast_sgrep_lang::required_pattern_literal("beta(1, 2)").as_deref(),
        Some("beta"),
        "whole-pattern-text prefilter dropped files holding structural matches"
    );
    let src = "beta(1, 2,)\ndelta(7, 9)\n";
    let hits = match_pattern(Language::Python, src, "beta(1, 2)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1u32],
        "code-side trailing comma must not block: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

/// H-CONF-030 (ii): container comment TEXT is invisible — the comment content
/// itself (not just its presence) must not decide the match
/// (`area( /* note */ 9)` vs code `area( /* mid */ 9)`; sg answers).
#[test]
fn hconf030_ii_comment_text_is_invisible_inside_arguments() {
    // Unit-level RED (pass 60): comment TEXT must not enter the prefilter —
    // the old whole-text literal required the pattern-side comment bytes the
    // matching file never contains.
    assert_eq!(
        ast_sgrep_lang::required_pattern_literal("area( /* note */ 9)").as_deref(),
        Some("area"),
        "comment text leaked into the byte prefilter"
    );
    let src =
        "fn area(r: i32) -> i32 {\n    r * r\n}\nfn main() {\n    let d = area( /* mid */ 9);\n}\n";
    let hits = match_pattern(Language::Rust, src, "area( /* note */ 9)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![5u32],
        "comment text inside arguments must not block: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

/// H-CONF-030 (iii): a let-rooted literal pattern answers via the R3 arm —
/// silent ok:true-0 was the under-match (lit_rs.rs:10, lit_rs_crlf.rs:5).
#[test]
fn hconf030_iii_let_rooted_literal_matches_structurally() {
    let src = "fn area(r: i32) -> i32 {\n    r * r\n}\nfn main() {\n    let a = area(9);\n}\n";
    let hits = match_pattern(Language::Rust, src, "let a = area(9)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![5u32],
        "let-rooted literal must match its statement: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
}

/// H-CONF-030 (iv): a BOM-led pattern is stripped like sg strips it — the
/// pattern matches its site instead of degrading to a silent empty.
#[test]
fn hconf030_iv_bom_led_pattern_is_stripped() {
    let src =
        "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\nfn main() {\n    let r = add(1, 2);\n}\n";
    let hits = match_pattern(Language::Rust, src, "\u{feff}add(1, 2)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![5u32],
        "BOM-led pattern must match like its bare twin: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    let bare = match_pattern(Language::Rust, src, "add(1, 2)").unwrap();
    assert_eq!(
        lines_of(&hits),
        lines_of(&bare),
        "BOM twin must equal the bare set"
    );
}

/// PASS 60 (fuzz F26-0601 family): a PATTERN-side comment inside an argument
/// container is a REQUIRED SLOT (sg): it must not widen the match to
/// comment-less sites, while text-free comment variants still match.
#[test]
fn pass60_pattern_comment_is_a_required_slot() {
    // Slot absent in source: no match (pre-pass-60 the whole-text prefilter
    // masked this over-match; the prefilter must stay sound without it).
    let src =
        "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\nfn main() {\n    let s = add(1, 2);\n}\n";
    let hits = match_pattern(Language::Rust, src, "add(1, /* NOTE */ 2)").unwrap();
    assert!(
        hits.is_empty(),
        "pattern-side comment must be a required slot, not invisible: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    // Slot present with DIFFERENT text: matches (030-ii, sg answers).
    let mid = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\nfn main() {\n    let s = add(1, /* mid */ 2);\n}\n";
    let variant = match_pattern(Language::Rust, mid, "add(1, /* NOTE */ 2)").unwrap();
    assert_eq!(lines_of(&variant), vec![5u32], "{:?}", variant);
    // Comment-free pattern still matches comment-carrying source (pass-54
    // source-side invisibility is unchanged when no slot demands it).
    let free = match_pattern(Language::Rust, mid, "add(1, 2)").unwrap();
    assert_eq!(lines_of(&free), vec![5u32], "{:?}", free);
}

// -- PASS 60 preservation guards (GREEN-by-design; must stay put) ------------

/// The registered fail-closed CONTRACT keeps its loud rows: `//` outside a
/// string is still comment syntax (`return $B// noteA` row from pass-51/54),
/// python `#` comment tails stay loud (sg exits 8 there — the 029
/// comment-glued sub-shapes), and `$$$`/newline/keyword rows are untouched.
/// PASS 122 (F4, f122d): the `let $A = $B` row leaves this language-free
/// loud list — sg 0.45.2 ANSWERS the spelling (grid /tmp/phase122/f4, js
/// and rust n1), so the js/ts faces route to the declarator walk; the rust
/// face keeps its loud contract as a PER-LANGUAGE census row
/// (`let_statement` root refused, asserted in f122d) instead of a
/// language-free ingress row.
#[test]
fn pass60_registered_comment_and_statement_rows_stay_fail_closed() {
    for pattern in [
        "return $B// noteA",
        "foo($A) # note",
        "$A + $A# note",
        "int $A($B) { $$$C }",
        "RETURN $A",
    ] {
        assert!(
            needs_ast_grep_fallback(pattern),
            "{pattern} must stay fail-closed"
        );
    }
    // The if-template lane keeps multi-statement bodies loud.
    assert!(needs_ast_grep_fallback("if ($COND) { $A; $B }"));
}

/// H-CONF-029 (lang-level unit guards, land with the fix): a pattern the
/// classifier rejects and the general lane cannot template FOR THIS LANGUAGE
/// is unanswerable natively — core's walk must be able to distinguish it
/// from per-file source-parse robustness. The member-prefixed if-template
/// family (`$O.if ($B) { $BODY }` ts + siblings) and the garbage-head /
/// misrepresentative-template shapes are unanswerable on their recorded
/// languages; the dup-meta family stays answerable everywhere it was.
#[test]
fn hconf029_per_language_answerability_matches_recorded_faces() {
    use ast_sgrep_lang::native_pattern_answerable;
    // member-prefixed if-templates (sub-shape 1, ×13 fuzz faces share the shape)
    assert!(!native_pattern_answerable(
        Language::TypeScript,
        "$O.if ($B) { $BODY }"
    ));
    assert!(!native_pattern_answerable(
        Language::Rust,
        "$O.if ($COND) { $BODY }"
    ));
    // garbage-number heads (F26 garbage-head faces)
    assert!(!native_pattern_answerable(
        Language::Rust,
        "1_000 ($A) { $B }"
    ));
    // comment-glued python shapes (F26 comment faces; `#` outside strings)
    assert!(!native_pattern_answerable(
        Language::Python,
        "$A + $A# note"
    ));
    // misrepresentative java template: `$O.IF ($A) { $B }` parses as a
    // `block`, not the call the pattern text advertises — unanswerable.
    assert!(!native_pattern_answerable(
        Language::Java,
        "$O.IF ($A) { $B }"
    ));
    // dup-meta family stays answerable (pass-51 faces keep serving)
    assert!(native_pattern_answerable(
        Language::Python,
        "$A == $A == $A"
    ));
    assert!(native_pattern_answerable(
        Language::Python,
        "foo($A, bar($A))"
    ));
    assert!(native_pattern_answerable(
        Language::Rust,
        "Some($A).unwrap_or($A)"
    ));
    // classifier-accepted and $-less patterns are always answerable
    assert!(native_pattern_answerable(Language::Rust, "fn $NAME($$$)"));
    assert!(native_pattern_answerable(Language::Python, "parse_data"));
}

// ---------------------------------------------------------------------------
// PASS 63 (r13-remediation): H-CONF-031 / F62-1..F62-5 contracts, each
// sg 0.45.2-probed (fixtures/pass63_r13_remediation probe matrix). RED
// evidence: the pre-fix binary answered the exact opposite of these
// assertions (silent ok:true-0 where sg exits 8; loud refuse / silent
// empty where sg answers).
// ---------------------------------------------------------------------------

/// H-CONF-031 (P0): decl/call template acceptance is a PER-LANGUAGE
/// question. sg's pattern gate (expando parse + single-node descent)
/// rejects `fn $A($B) { $$$ }` under the typescript grammar (exit 8) while
/// accepting `def $A($B): $$$C` under rust (ERROR-wrapped single child,
/// accepted with a warning) — the language-blind classifier answered
/// silent ok:true-0 on the first family and kept the second answerable.
#[test]
fn hconf031_decl_template_acceptance_is_language_aware() {
    // sg exit-8 faces: unanswerable per language.
    assert!(!native_pattern_answerable(
        Language::TypeScript,
        "fn $A($B) { $$$ }"
    ));
    assert!(!native_pattern_answerable(
        Language::Python,
        "fn $A($B) { $$C }"
    ));
    assert!(!native_pattern_answerable(
        Language::Python,
        "$O.$O.a\n.b.c($A)"
    ));
    assert!(!native_pattern_answerable(
        Language::Python,
        "function $A($B) { $$$C }"
    ));
    assert!(!native_pattern_answerable(
        Language::Rust,
        "func $A($B) { $$$C }"
    ));
    // sg-accepted mirrors stay answerable (empty-run or hits).
    assert!(native_pattern_answerable(
        Language::Python,
        "def $A($B): $$$C"
    ));
    assert!(native_pattern_answerable(
        Language::Rust,
        "def $A($B): $$$C"
    ));
    assert!(native_pattern_answerable(
        Language::Rust,
        "fn $A($B) { $$$C }"
    ));
    assert!(native_pattern_answerable(
        Language::Java,
        "fn $A($B) { $$$C }"
    ));
    // Registered native shapes keep their cross-language answerability.
    assert!(native_pattern_answerable(Language::Rust, "fn $NAME($$$)"));
    assert!(native_pattern_answerable(
        Language::Python,
        "foo($A, bar($A))"
    ));
}

/// F62-1: `#` is comment syntax ONLY in python/ruby/php. rust attributes,
/// C preprocessor, swift selectors, and js private fields are real syntax —
/// sg answers them; the language-free `#` refusal failed them closed.
#[test]
fn f62_1_hash_syntax_faces_answer_like_sg() {
    assert!(!needs_ast_grep_fallback("#[derive($A)]"));
    assert!(!needs_ast_grep_fallback("#![allow($A)]"));
    assert!(!needs_ast_grep_fallback("#include $X"));
    assert!(!needs_ast_grep_fallback("#define $X $Y"));
    assert!(!needs_ast_grep_fallback("this.#x = $V"));
    assert!(!needs_ast_grep_fallback("tag(r#\"$A\"#)"));
    // The registered python comment-glued faces STAY fail-closed.
    assert!(needs_ast_grep_fallback("def greet($A): $C # tail"));
    assert!(!native_pattern_answerable(
        Language::Python,
        "def greet($A): $C # tail"
    ));
    assert!(!native_pattern_answerable(
        Language::Python,
        "$A + $A # note"
    ));
}

#[test]
fn f62_1_rust_attribute_templates_match() {
    let src = "#![allow(dead_code)]\n#[derive(Debug)]\n#[serde(rename = \"x\")]\nstruct S;\n";
    let derive = match_pattern(Language::Rust, src, "#[derive($A)]").unwrap();
    assert_eq!(lines_of(&derive), vec![2u32], "{derive:?}");
    assert_eq!(
        derive[0].captures.get("A").map(String::as_str),
        Some("Debug")
    );
    let any_attr = match_pattern(Language::Rust, src, "#[$A]").unwrap();
    assert_eq!(lines_of(&any_attr), vec![2u32, 3], "{any_attr:?}");
    let inner = match_pattern(Language::Rust, src, "#![allow($A)]").unwrap();
    assert_eq!(lines_of(&inner), vec![1u32], "{inner:?}");
}

#[test]
fn f62_1_raw_string_metavar_is_literal_text_like_sg() {
    // sg probes: `tag(r#"$A"#)` = exit 1 ran-empty — accepted pattern,
    // zero hits (the metavariable bytes are literal text inside the raw
    // string). The pre-fix tree refused the pattern outright.
    let src = "fn f() {\n    tag(r#\"hi\")\n    tag(r#\"bye\")\n}\n";
    assert!(!needs_ast_grep_fallback("tag(r#\"$A\"#)"));
    let hits = match_pattern(Language::Rust, src, "tag(r#\"$A\"#)").unwrap();
    assert!(
        hits.is_empty(),
        "raw-string metavar must bind nothing: {hits:?}"
    );
}

#[test]
fn f62_1_c_preprocessor_templates_match() {
    let src = "#include <a.h>\n#include \"b.h\"\n#define MAX 3\n#include <c.h>\n";
    let includes = match_pattern(Language::C, src, "#include $X").unwrap();
    assert_eq!(lines_of(&includes), vec![1u32, 2, 4], "{includes:?}");
    let defines = match_pattern(Language::C, src, "#define $X $Y").unwrap();
    assert_eq!(lines_of(&defines), vec![3u32], "{defines:?}");
    assert_eq!(
        defines[0].captures.get("X").map(String::as_str),
        Some("MAX")
    );
}

#[test]
fn f62_1_js_private_field_assignment_matches() {
    let src = "class C {\n  #x = 1;\n  set(v) {\n    this.#x = v;\n  }\n}\n";
    let hits = match_pattern(Language::JavaScript, src, "this.#x = $V").unwrap();
    assert_eq!(lines_of(&hits), vec![4u32], "{hits:?}");
    assert_eq!(hits[0].captures.get("V").map(String::as_str), Some("v"));
}

/// F62-2: the statement-head family sg answers at statement root
/// (raise/yield py+rb, throw ts+java, await js, bare break/continue js).
#[test]
fn f62_2_statement_head_family_answers() {
    let py =
        "def f(cond):\n    if cond:\n        raise ValueError('bad')\n    yield 1\n    yield 2\n";
    let raises = match_pattern(Language::Python, py, "raise $A").unwrap();
    assert_eq!(lines_of(&raises), vec![3u32], "{raises:?}");
    assert_eq!(
        raises[0].captures.get("A").map(String::as_str),
        Some("ValueError('bad')")
    );
    let yields = match_pattern(Language::Python, py, "yield $A").unwrap();
    assert_eq!(lines_of(&yields), vec![4u32, 5], "{yields:?}");

    let ts = "function f(cond: boolean): void {\n    if (cond) {\n        throw new Error('bad');\n    }\n    throw new Error('worse');\n}\n";
    let throws = match_pattern(Language::TypeScript, ts, "throw $A").unwrap();
    assert_eq!(lines_of(&throws), vec![3u32, 5], "{throws:?}");

    let java = "class T {\n    void f(boolean cond) {\n        if (cond) {\n            throw new IllegalStateException(\"bad\");\n        }\n        throw new RuntimeException(\"worse\");\n    }\n}\n";
    let jthrows = match_pattern(Language::Java, java, "throw $A").unwrap();
    assert_eq!(lines_of(&jthrows), vec![4u32, 6], "{jthrows:?}");

    let rb = "def f\n  raise ArgumentError, 'bad'\n  yield 1\nend\n\ndef g\n  raise 'plain'\nend\n";
    let rraises = match_pattern(Language::Ruby, rb, "raise $A").unwrap();
    assert_eq!(lines_of(&rraises), vec![2u32, 7], "{rraises:?}");
    // sg probe: the 2-arg command binds the WHOLE argument list text.
    assert_eq!(
        rraises[0].captures.get("A").map(String::as_str),
        Some("ArgumentError, 'bad'")
    );

    let js = "async function f(p) {\n    await p;\n    await q();\n    if (p) { break; }\n    continue;\n}\n";
    let awaits = match_pattern(Language::JavaScript, js, "await $A").unwrap();
    assert_eq!(lines_of(&awaits), vec![2u32, 3], "{awaits:?}");
    let brk = match_pattern(Language::JavaScript, js, "break").unwrap();
    assert_eq!(lines_of(&brk), vec![4u32], "{brk:?}");
    let cont = match_pattern(Language::JavaScript, js, "continue").unwrap();
    assert_eq!(lines_of(&cont), vec![5u32], "{cont:?}");

    // Uppercase head keeps the registered loud contract (sg exit 8).
    assert!(needs_ast_grep_fallback("RETURN $A"));
}

/// F62-5: `return $A` templates across the languages sg answers — the C
/// context now terminates the statement (`;`), and the terminator
/// lenience aligns ASI-omitted template roots with `;`-ful candidates.
#[test]
fn f62_5_return_statement_templates_cover_c_and_terminated_sources() {
    let c = "int f(int x) {\n    if (x) {\n        return x;\n    }\n    return 0;\n}\n";
    let c_hits = match_pattern(Language::C, c, "return $A").unwrap();
    assert_eq!(lines_of(&c_hits), vec![3u32, 5], "{c_hits:?}");
    assert_eq!(c_hits[0].captures.get("A").map(String::as_str), Some("x"));

    let ts = "function f(cond: boolean): number {\n    if (cond) {\n        return cond;\n    }\n    return 0;\n}\n";
    let ts_hits = match_pattern(Language::TypeScript, ts, "return $A").unwrap();
    assert_eq!(lines_of(&ts_hits), vec![3u32, 5], "{ts_hits:?}");

    // Registered faces keep answering.
    let py = "def f(x):\n    return x\n\ndef g():\n    return 7\n";
    let py_hits = match_pattern(Language::Python, py, "return $A").unwrap();
    assert_eq!(lines_of(&py_hits), vec![2u32, 5], "{py_hits:?}");
}

/// F62-3: the faithful-path veto fires only when unification could not
/// decide anyway. sg binds wildcard receivers to the receiver's literal
/// text on subscript / macro receivers (`arr[0]`, `vec![1, 2]`), and the
/// duplicate-name chains keep rejecting through capture equality.
#[test]
fn f62_3_nonfaithful_receivers_bind_by_text_duplicates_still_reject() {
    let src = "fn demo(arr: Vec<u32>, w: Vec2) -> usize {\n    let a = arr[0].len();\n    let b = w.v[1].len();\n    let c = vec![1, 2].len();\n    a + b + c\n}\n";
    let wild = match_pattern(Language::Rust, src, "$O.$M($$$A)").unwrap();
    assert_eq!(lines_of(&wild), vec![2u32, 3, 4], "{wild:?}");
    assert_eq!(
        wild[0].captures.get("O").map(String::as_str),
        Some("arr[0]")
    );
    assert_eq!(wild[0].captures.get("M").map(String::as_str), Some("len"));
    assert_eq!(
        wild[2].captures.get("O").map(String::as_str),
        Some("vec![1, 2]")
    );

    let named = match_pattern(Language::Rust, src, "$A.$B($$$C)").unwrap();
    assert_eq!(lines_of(&named), vec![2u32, 3, 4], "{named:?}");

    let dup = match_pattern(Language::Rust, src, "$O.$O($$$A)").unwrap();
    assert!(
        dup.is_empty(),
        "same-name chains must stay rejected: {dup:?}"
    );
}

/// F58-3 control (kept green by capture equality, not the blanket veto):
/// receiver-is-call chains bind head="y.first()" vs tail="first" and the
/// duplicate name rejects.
#[test]
fn f62_3_receiver_call_chains_still_reject_same_name() {
    let src = "y.first().first()\nwrap().wrap()\ng().h()\n";
    let hits = match_pattern(Language::Python, src, "$O.$O($$$A)").unwrap();
    assert!(hits.is_empty(), "{hits:?}");
    let distinct = match_pattern(Language::Python, src, "$O.$M($$$A)").unwrap();
    // sg answers the OUTER and the INNER two-segment call on line 1
    // (probed: 4 hits — (1,0) twice — plus lines 2 and 3).
    assert_eq!(lines_of(&distinct), vec![1u32, 1, 2, 3], "{distinct:?}");
}

/// F62-4: pattern comment-slots align by raw comma-adjacency (sg probes:
/// `calc(1, /* n */ 2)` answers only the post-comma source; different
/// comment TEXT in the same slot still answers — pass-60 030-ii).
#[test]
fn f62_4_comment_slots_align_by_comma_adjacency() {
    let src = "fn calc(a: u32, b: u32) -> u32 { a + b }\n\nfn demo() -> u32 {\n    let x = calc(1 /* mid */, 2);\n    let y = calc(1, /* n */ 2);\n    let z = calc(3, 7);\n    x + y + z\n}\n";
    let n_slot = match_pattern(Language::Rust, src, "calc(1, /* n */ 2)").unwrap();
    assert_eq!(
        lines_of(&n_slot),
        vec![5u32],
        "post-comma slot must answer only the /* n */ line: {n_slot:?}"
    );
    let mid_slot = match_pattern(Language::Rust, src, "calc(1 /* mid */, 2)").unwrap();
    assert_eq!(
        lines_of(&mid_slot),
        vec![4u32],
        "glued slot keeps its own face: {mid_slot:?}"
    );
    let no_comment = match_pattern(Language::Rust, src, "calc($A, $B)").unwrap();
    assert_eq!(lines_of(&no_comment), vec![4u32, 5, 6], "{no_comment:?}");
    // Different comment text in the SAME slot still answers (pass-60 030-ii).
    let other_text = match_pattern(Language::Rust, src, "calc(1, /* NOTE */ 2)").unwrap();
    assert_eq!(lines_of(&other_text), vec![5u32], "{other_text:?}");
}

// ---------------------------------------------------------------------------
// PASS 65 (r15-remediation) — failure-first RED tests for the pass-64
// findings. Every expectation below is pinned against sg 0.45.2 probes
// (fixtures/pass65_r15 corpora); each test MUST fail on the pre-fix tree.
// ---------------------------------------------------------------------------

/// F64-1: statement-head siblings sg answers that the subject fails closed
/// on — `await $A` (python/csharp), `throw $A` (csharp), `defer $A` /
/// `go $A` (go). Registered statement-head faces stay green as controls.
#[test]
fn f64_1_statement_head_siblings_answer_await_throw_defer_go() {
    // python await (sg: lines 2,3,4)
    let py = "async def fetch_all(session):\n    a = await session.get(1)\n    b = await session.get(2)\n    c = await other()\n    return a, b, c\n";
    let awaits = match_pattern(Language::Python, py, "await $A").unwrap();
    assert_eq!(lines_of(&awaits), vec![2u32, 3, 4], "{awaits:?}");
    assert_eq!(
        awaits[0].captures.get("A").map(String::as_str),
        Some("session.get(1)")
    );

    // csharp throw / throw new (sg: {3,4} and {4})
    let cs = "class Store {\n    void Load() {\n        throw new SystemException();\n        throw new Exception(\"bad\");\n    }\n}\n";
    let throws = match_pattern(Language::CSharp, cs, "throw $A").unwrap();
    assert_eq!(lines_of(&throws), vec![3u32, 4], "{throws:?}");
    let throw_new = match_pattern(Language::CSharp, cs, "throw new Exception($A)").unwrap();
    assert_eq!(lines_of(&throw_new), vec![4u32], "{throw_new:?}");

    // csharp await (sg: {3})
    let cs2 = "class C {\n    async Task M() {\n        await Task.Delay(1);\n    }\n}\n";
    let cawaits = match_pattern(Language::CSharp, cs2, "await $A").unwrap();
    assert_eq!(lines_of(&cawaits), vec![3u32], "{cawaits:?}");

    // go defer / go (sg: defer {6,8}, go {7,9} on this corpus)
    let go = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tdefer fmt.Println(\"done\")\n\tgo worker(1)\n\tdefer cleanup()\n\tgo runner()\n}\n";
    let defers = match_pattern(Language::Go, go, "defer $A").unwrap();
    assert_eq!(lines_of(&defers), vec![6u32, 8], "{defers:?}");
    let gos = match_pattern(Language::Go, go, "go $A").unwrap();
    assert_eq!(lines_of(&gos), vec![7u32, 9], "{gos:?}");

    // Registered statement-head faces keep their pass-62 answers.
    let py_raise = match_pattern(
        Language::Python,
        "def g():\n    raise ValueError\n",
        "raise $A",
    )
    .unwrap();
    assert_eq!(lines_of(&py_raise), vec![2u32], "{py_raise:?}");
    let java_throw = match_pattern(
        Language::Java,
        "class M { void r() { throw new E(); } }\n",
        "throw $A",
    )
    .unwrap();
    assert_eq!(lines_of(&java_throw), vec![1u32], "{java_throw:?}");
}

/// F64-2: three-segment receiver chains `$O.$M1($$$A).$M2($$$B)` answer like
/// sg (rust {19,20,22,23,24} incl. the 4-seg inner-prefix line, python
/// {18,19,21,22,23,24}, ts {12,13,15}); the same-name veto
/// `$O.$O($$$A).$O($$$B)` keeps sg's equality semantics (only y.y().y()
/// answers), and the two-segment contract is unchanged.
#[test]
fn f64_2_three_segment_receiver_chains_bind_like_sg() {
    let rs = "struct Alpha;\nstruct Beta;\nstruct Gamma;\n\nimpl Alpha {\n    fn first(&self) -> Beta { Beta }\n}\n\nimpl Beta {\n    fn second(&self) -> Gamma { Gamma }\n}\n\nimpl Gamma {\n    fn third(&self) -> u32 { 3 }\n}\n\nfn main() {\n    let alpha = Alpha;\n    let x = alpha.first().second();\n    let y = alpha.first().third();\n    let b = Beta;\n    let z = b.second().third();\n    let dup = b.second().second();\n    let deep = alpha.first().second().third();\n}\n";
    let three = match_pattern(Language::Rust, rs, "$O.$M1($$$A).$M2($$$B)").unwrap();
    assert_eq!(
        klass_lines(&three),
        vec![19u32, 20, 22, 23, 24],
        "{three:?}"
    );

    let dup_src =
        "fn main() {\n    let y = Y;\n    let a = y.y().y();\n    let b = y.y().z();\n}\n";
    let dup = match_pattern(Language::Rust, dup_src, "$O.$O($$$A).$O($$$B)").unwrap();
    assert_eq!(
        lines_of(&dup),
        vec![3u32],
        "same-name veto: only y.y().y(): {dup:?}"
    );
    let two = match_pattern(Language::Rust, dup_src, "$O.$M($$$A)").unwrap();
    // sg answers BOTH chain nodes per line (JSON probe 0.45.2: 4 rows —
    // outer cols 12-21 with O=y.y(), inner cols 12-17 with O=y); the subject
    // keeps that exact multiplicity on the two-segment contract.
    assert_eq!(
        lines_of(&two),
        vec![3u32, 3, 4, 4],
        "two-segment contract unchanged: {two:?}"
    );

    let py = "class Alpha:\n    def first(self):\n        return Beta()\n\nclass Beta:\n    def second(self):\n        return Gamma()\n\n    def second_more(self, extra):\n        return Gamma()\n\nclass Gamma:\n    def third(self):\n        return 3\n\ndef main():\n    alpha = Alpha()\n    x = alpha.first().second()\n    y = alpha.first().third()\n    b = Beta()\n    z = b.second().third()\n    dup = b.second().second()\n    deep = alpha.first().second().third()\n    argd = alpha.first().second_more(9)\n";
    let py_three = match_pattern(Language::Python, py, "$O.$M1($$$A).$M2($$$B)").unwrap();
    assert_eq!(
        klass_lines(&py_three),
        vec![18u32, 19, 21, 22, 23, 24],
        "{py_three:?}"
    );
    let py_dup = match_pattern(Language::Python, py, "$O.$O($$$A).$O($$$B)").unwrap();
    assert!(
        py_dup.is_empty(),
        "same-name three-seg must veto: {py_dup:?}"
    );

    let ts = "class Alpha {\n  first(): Beta { return new Beta(); }\n}\nclass Beta {\n  second(): Gamma { return new Gamma(); }\n}\nclass Gamma {\n  third(): number { return 3; }\n}\nfunction main(): void {\n  const alpha = new Alpha();\n  const x = alpha.first().second();\n  const y = alpha.first().third();\n  const b = new Beta();\n  const z = b.second().third();\n  const opt = maybe()?.load();\n  const opt2 = user?.profile?.load();\n}\n";
    let ts_three = match_pattern(Language::TypeScript, ts, "$O.$M1($$$A).$M2($$$B)").unwrap();
    assert_eq!(klass_lines(&ts_three), vec![12u32, 13, 15], "{ts_three:?}");
}

/// F64-3: the cpp/c preproc directive family answers like sg — directive
/// kind + directive-head text, branch bodies ignored — while `#endif`
/// (anonymous token, sg-empty) stays unanswerable and `#if defined($A)`
/// binds the nested metavar like sg (probe-corrected this pass).
#[test]
fn f64_3_preproc_directive_family_answers_like_sg() {
    let cpp = "#include <vector>\n#ifdef FEATURE\n#define TWICE(x) ((x) * 2)\n#endif\n#ifndef MISSING\n#pragma once\n#endif\nint main() { return TWICE(2); }\n";
    let ifdef = match_pattern(Language::Cpp, cpp, "#ifdef $A").unwrap();
    assert_eq!(lines_of(&ifdef), vec![2u32], "{ifdef:?}");
    assert_eq!(
        ifdef[0].captures.get("A").map(String::as_str),
        Some("FEATURE")
    );
    let ifndef = match_pattern(Language::Cpp, cpp, "#ifndef $A").unwrap();
    assert_eq!(lines_of(&ifndef), vec![5u32], "{ifndef:?}");
    let concrete = match_pattern(Language::Cpp, cpp, "#ifdef FEATURE").unwrap();
    assert_eq!(lines_of(&concrete), vec![2u32], "{concrete:?}");

    // Controls that must keep today's contract.
    let pragma = match_pattern(Language::Cpp, cpp, "#pragma $X").unwrap();
    assert_eq!(lines_of(&pragma), vec![6u32], "{pragma:?}");
    let define = match_pattern(Language::Cpp, cpp, "#define TWICE(x) ((x) * 2)").unwrap();
    assert_eq!(lines_of(&define), vec![3u32], "{define:?}");
    let endif = match_pattern(Language::Cpp, cpp, "#endif").unwrap();
    assert!(
        endif.is_empty(),
        "#endif (sg-empty) must not answer: {endif:?}"
    );

    let c = "#include \"local.h\"\n#ifdef FEATURE\n#define WICE(y) ((y) * 3)\n#endif\n#if defined(FEATURE)\nint w = 1;\n#endif\nint main() { return 0; }\n";
    let c_ifdef = match_pattern(Language::C, c, "#ifdef $A").unwrap();
    assert_eq!(lines_of(&c_ifdef), vec![2u32], "{c_ifdef:?}");
    let c_if = match_pattern(Language::C, c, "#if $A").unwrap();
    assert_eq!(lines_of(&c_if), vec![5u32], "{c_if:?}");
    let c_concrete = match_pattern(Language::C, c, "#ifdef FEATURE").unwrap();
    assert_eq!(lines_of(&c_concrete), vec![2u32], "{c_concrete:?}");
    // PASS 65a probe correction: sg 0.45.2 ANSWERS `#if defined($A)` (region
    // match at the directive head, binding the metavar INSIDE the condition
    // text: A = FEATURE). The previous "no sg answer" pin encoded a claim the
    // fresh probe refuted; the preproc lane must bind nested metavars like sg
    // instead of failing closed where sg answers.
    let defined_mv = match_pattern(Language::C, c, "#if defined($A)").unwrap();
    assert_eq!(lines_of(&defined_mv), vec![5u32], "{defined_mv:?}");
    assert_eq!(
        defined_mv[0].captures.get("A").map(String::as_str),
        Some("FEATURE"),
        "{defined_mv:?}"
    );
    assert!(
        !needs_ast_grep_fallback("#if defined($A)"),
        "defined($A) must answer natively like sg (probed 0.45.2: A=FEATURE)"
    );
    assert!(
        !needs_ast_grep_fallback("#ifdef $A"),
        "ifdef metavar face must now answer natively"
    );
}

/// F64-4: ruby string-interpolation metavariables bind like sg — the
/// interpolation content is the capture (N = "session.user"), while the
/// registered no-interpolation string-metavar refusal stays loud.
#[test]
fn f64_4_ruby_interpolation_metavar_binds() {
    let rb = "name = \"user-#{session.user}\"\nother = \"plain\"\ngreeting = \"hello-#{user.name}-bye\"\nid = \"##{serial}\"\ndef render(x)\n  \"v=#{x}\"\nend\n";
    let user = match_pattern(Language::Ruby, rb, "name = \"user-#{$N}\"").unwrap();
    assert_eq!(lines_of(&user), vec![1u32], "{user:?}");
    assert_eq!(
        user[0].captures.get("N").map(String::as_str),
        Some("session.user")
    );

    let hello = match_pattern(Language::Ruby, rb, "\"hello-#{$A}-bye\"").unwrap();
    assert_eq!(lines_of(&hello), vec![3u32], "{hello:?}");
    assert_eq!(
        hello[0].captures.get("A").map(String::as_str),
        Some("user.name")
    );

    let serial = match_pattern(Language::Ruby, rb, "\"##{$D}\"").unwrap();
    assert_eq!(lines_of(&serial), vec![4u32], "{serial:?}");
    assert_eq!(
        serial[0].captures.get("D").map(String::as_str),
        Some("serial")
    );

    // Concrete interpolation-free literals keep matching exactly.
    let plain = match_pattern(Language::Ruby, rb, "other = \"plain\"").unwrap();
    assert_eq!(lines_of(&plain), vec![2u32], "{plain:?}");
    // Registered control: a metavar inside a plain (non-interpolation)
    // string stays refused/loud in other languages (pass-54 contract).
    assert!(needs_ast_grep_fallback("greet(\"($A)\")"));
}

/// F64-5: literal-lane roots whose value is a member call answer like sg —
/// `let r2 = q.len();`, `hh = u.len()`, `t = "a#b".split("#")`,
/// `const v1 = maybe.load();` — while the registered plain-root faces stay
/// green.
#[test]
fn f64_5_literal_lane_answers_member_call_valued_roots() {
    let rs = "fn main() {\n    let q = vec![1, 2, 3];\n    let w = vec![4];\n    let r2 = q.len();\n    let s3 = q.len() as u32;\n    let u5 = w.count();\n    let t4 = q;\n    let a = area(9);\n    alpha.beta().gamma();\n}\nfn area(x: u32) -> u32 { x }\nstruct Alpha;\nimpl Alpha { fn beta(&self) -> Gamma { Gamma } }\nstruct Gamma;\nimpl Gamma { fn gamma(&self) -> u32 { 1 } }\n";
    let r2 = match_pattern(Language::Rust, rs, "let r2 = q.len();").unwrap();
    assert_eq!(lines_of(&r2), vec![4u32], "{r2:?}");
    let s3 = match_pattern(Language::Rust, rs, "let s3 = q.len() as u32;").unwrap();
    assert_eq!(lines_of(&s3), vec![5u32], "{s3:?}");
    let u5 = match_pattern(Language::Rust, rs, "let u5 = w.count();").unwrap();
    assert_eq!(lines_of(&u5), vec![6u32], "{u5:?}");

    let py = "u = [1, 2, 3]\nhh = u.len()\nt = \"a#b\".split(\"#\")\ndd = compute(9)\nbb = \"xyz\"\nalpha = Alpha()\nalpha.beta().gamma()\n";
    let hh = match_pattern(Language::Python, py, "hh = u.len()").unwrap();
    assert_eq!(lines_of(&hh), vec![2u32], "{hh:?}");
    let tt = match_pattern(Language::Python, py, "t = \"a#b\".split(\"#\")").unwrap();
    assert_eq!(lines_of(&tt), vec![3u32], "{tt:?}");

    let ts =
        "const maybe = { load: () => 1 };\nconst v1 = maybe.load();\nconst v2 = maybe?.load();\n";
    let v1 = match_pattern(Language::TypeScript, ts, "const v1 = maybe.load();").unwrap();
    assert_eq!(lines_of(&v1), vec![2u32], "{v1:?}");

    // Registered controls: plain roots already answered and stay answered.
    let plain_let = match_pattern(Language::Rust, rs, "let t4 = q;").unwrap();
    assert_eq!(lines_of(&plain_let), vec![7u32], "{plain_let:?}");
    let plain_call = match_pattern(Language::Rust, rs, "alpha.beta().gamma();").unwrap();
    assert_eq!(lines_of(&plain_call), vec![9u32], "{plain_call:?}");
    let py_dd = match_pattern(Language::Python, py, "dd = compute(9)").unwrap();
    assert_eq!(lines_of(&py_dd), vec![4u32], "{py_dd:?}");
}

/// F64-6: bare break/continue/throw/yield/raise answer kind-level like sg —
/// the bare head matches every statement of its family regardless of
/// arguments or trailing semicolon — while arg-ful heads keep their
/// captures and the registered js faces stay green.
#[test]
fn f64_6_bare_statement_heads_answer_kind_level() {
    let java = "class Main {\n    void run() {\n        for (int i = 0; i < 3; i++) {\n            continue;\n        }\n        while (true) {\n            break;\n        }\n        throw new RuntimeException();\n    }\n}\n";
    assert_eq!(
        lines_of(&match_pattern(Language::Java, java, "break").unwrap()),
        vec![7u32]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::Java, java, "break;").unwrap()),
        vec![7u32]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::Java, java, "continue").unwrap()),
        vec![4u32]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::Java, java, "continue;").unwrap()),
        vec![4u32]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::Java, java, "throw").unwrap()),
        vec![9u32]
    );

    // csharp bare heads (sg: break {8}/{7}, continue {4}, throw kind-level
    // over every throw_statement incl. `throw new ...`).
    let cs = "class Store {\n    void Load() {\n        throw new SystemException();\n        throw new Exception(\"bad\");\n    }\n    int Pick() {\n        if (true) { return 1; }\n        break\n    }\n}\n";
    let cs2 = "class C {\n    void M() {\n        for (int i = 0; i < 3; i++) {\n            continue;\n        }\n        while (true) {\n            break;\n        }\n        throw new Exception();\n    }\n}\n";
    assert_eq!(
        lines_of(&match_pattern(Language::CSharp, cs, "break").unwrap()),
        vec![8u32]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::CSharp, cs2, "break").unwrap()),
        vec![7u32]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::CSharp, cs2, "continue").unwrap()),
        vec![4u32]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::CSharp, cs, "throw").unwrap()),
        vec![3u32, 4]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::CSharp, cs2, "throw").unwrap()),
        vec![9u32]
    );

    // rust break is kind-level over `break 9;` and `break;` (sg {3,6}).
    let rs =
        "fn f() {\n    loop {\n        break 9;\n    }\n    loop {\n        break;\n    }\n}\n";
    assert_eq!(
        lines_of(&match_pattern(Language::Rust, rs, "break").unwrap()),
        vec![3u32, 6]
    );

    // python bare yield / raise (sg kind-level {2,3,4} / {7,8}).
    let py = "def gen():\n    yield\n    yield 1\n    yield 2\n\ndef risky():\n    raise\n    raise ValueError\n\ndef loopy():\n    for i in range(3):\n        break\n    return i\n";
    assert_eq!(
        lines_of(&match_pattern(Language::Python, py, "yield").unwrap()),
        vec![2u32, 3, 4]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::Python, py, "raise").unwrap()),
        vec![7u32, 8]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::Python, py, "raise $A").unwrap()),
        vec![8u32]
    );

    // Registered pass-63 js faces stay green.
    let js = "function loopy() {\n  for (let i = 0; i < 3; i++) {\n    if (i === 1) { break; }\n    if (i === 2) { continue; }\n  }\n}\n";
    assert_eq!(
        lines_of(&match_pattern(Language::JavaScript, js, "break").unwrap()),
        vec![3u32]
    );
    assert_eq!(
        lines_of(&match_pattern(Language::JavaScript, js, "continue").unwrap()),
        vec![4u32]
    );
}

/// F64-7 (registered adjudication, scoped back): ts optional-chain segments
/// are not plain member segments — sg refuses `?.` candidates for a `.`
/// template, so the subject must too.
#[test]
fn f64_7_ts_optional_chain_receivers_stay_unbound() {
    let ts = "class Alpha {\n  first(): Beta { return new Beta(); }\n}\nclass Beta {\n  second(): Gamma { return new Gamma(); }\n}\nclass Gamma {\n  third(): number { return 3; }\n}\nfunction main(): void {\n  const alpha = new Alpha();\n  const x = alpha.first().second();\n  const y = alpha.first().third();\n  const b = new Beta();\n  const z = b.second().third();\n  const opt = maybe()?.load();\n  const opt2 = user?.profile?.load();\n}\n";
    let two = match_pattern(Language::TypeScript, ts, "$O.$M($$$A)").unwrap();
    assert_eq!(klass_lines(&two), vec![12u32, 13, 15], "{two:?}");
    let three = match_pattern(Language::TypeScript, ts, "$O.$M1($$$A).$M2($$$B)").unwrap();
    assert_eq!(lines_of(&three), vec![12u32, 13, 15], "{three:?}");

    let opt_src = "const a = b()?.c();\nconst d = e?.f();\n";
    assert!(
        match_pattern(Language::TypeScript, opt_src, "$O.$M($$$A)")
            .unwrap()
            .is_empty(),
        "optional-chain sources must not answer a plain-dot template"
    );

    // PASS 65a probe correction: sg 0.45.2 answers a `;`-ful concrete chain
    // pattern ONLY as an `;`-inclusive expression_statement (probe: on a
    // corpus holding both shapes, the declaration-embedded chain does NOT
    // answer and the bare statement does at cols 2-25). The previous [12]
    // pin encoded the subject's `;`-blind root descent — the H-CONF-034
    // class this pass closes — so the declaration-embedded face is now
    // correctly empty and the bare-statement face answers line 3.
    let plain = match_pattern(Language::TypeScript, ts, "alpha.first().second();").unwrap();
    assert!(
        plain.is_empty(),
        "declaration-embedded chain must not answer a `;`-ful pattern: {plain:?}"
    );
    let bare_src = "function main(): void {\n  const x = alpha.first().second();\n  alpha.first().second();\n}\n";
    let bare = match_pattern(Language::TypeScript, bare_src, "alpha.first().second();").unwrap();
    assert_eq!(lines_of(&bare), vec![3u32], "{bare:?}");
    // The `;`-less pattern keeps answering both shapes (sg {2,3}).
    let no_semi = match_pattern(Language::TypeScript, bare_src, "alpha.first().second()").unwrap();
    assert_eq!(lines_of(&no_semi), vec![2u32, 3], "{no_semi:?}");
}

/// H-CONF-034 (registered adjudication, scoped back): a pattern-trailing
/// `;` is significant — sg does not match a semicolon-less source with a
/// semicolon-ful pattern, while a semicolon-less pattern still matches a
/// `;`-ful source (pass-62 ASI lenience, one-directional).
#[test]
fn f64_hconf034_pattern_semicolon_stays_significant() {
    let ts = "function f(cond: boolean): number {\n    if (cond) { return cond; }\n    return 2\n}\nfunction g(): number {\n    return 3;\n}\n";
    let semi = match_pattern(Language::TypeScript, ts, "return $A;").unwrap();
    assert_eq!(lines_of(&semi), vec![2u32, 6], "{semi:?}");
    let no_semi = match_pattern(Language::TypeScript, ts, "return $A").unwrap();
    assert_eq!(lines_of(&no_semi), vec![2u32, 3, 6], "{no_semi:?}");
}

// ---------------------------------------------------------------------------
// PASS 65f (F64-7 scope-back) — ts/js optional-chain faces, token-exact per
// sg 0.45.2 (probes in /tmp/pass65f, transcript in the pass report). sg's
// rule: the `?.` is a required anonymous connector token. The explicit
// spelling `$O?.$M($$$A)` must PARSE and answer exactly the optional-chain
// call set (incl. head folding); the plain `$O.$M($$$A)` must never answer
// optional receivers; receiver captures must never carry a glued trailing
// `?` (the proven codemod corruption).
// ---------------------------------------------------------------------------

/// The 65d probe corpus: 6 plain faces {1,4,5,7,8,9}, 3 optional {2,3,6}.
const F64_7_TS_CORPUS: &str = "const r1 = a.b();\nconst r2 = a?.b();\nconst r3 = user?.profile?.load();\nobj.method(1, 2);\nconst r4 = a.method(3);\nconst r5 = cfg?.get(\"k\");\nconst r6 = arr.map((x) => x);\ncfg.get(\"k\");\nconsole.log(r1, r2);\n";

/// F64-7 (1): `$O?.$M($$$A)` parses, is answerable ingress, and answers
/// EXACTLY the optional-chain call set with sg's captures — including the
/// nested-chain head fold (`$O` = `user?.profile` on
/// `user?.profile?.load()`). The plain faces {1,4,5,7,8,9} stay unanswered
/// by the optional template (token-exact both directions).
#[test]
fn f64_7_scopeback_optional_template_answers_optional_chains() {
    assert!(
        !needs_ast_grep_fallback("$O?.$M($$$A)"),
        "$O?.$M($$$A) must classify natively (sg parses and answers it, 0.45.2)"
    );
    assert!(
        native_pattern_answerable(Language::TypeScript, "$O?.$M($$$A)"),
        "ts must accept the optional-chain template like sg"
    );
    assert!(
        native_pattern_answerable(Language::JavaScript, "$O?.$M($$$A)"),
        "js must accept the optional-chain template like sg"
    );

    let hits = match_pattern(Language::TypeScript, F64_7_TS_CORPUS, "$O?.$M($$$A)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2u32, 3, 6],
        "optional template must answer exactly the optional faces: {hits:?}"
    );
    let by_line = |line: u32| {
        hits.iter()
            .find(|h| h.line_start == line)
            .unwrap_or_else(|| panic!("missing hit on line {line}"))
    };
    assert_eq!(by_line(2).captures.get("O").map(String::as_str), Some("a"));
    assert_eq!(by_line(2).captures.get("M").map(String::as_str), Some("b"));
    assert_eq!(
        by_line(3).captures.get("O").map(String::as_str),
        Some("user?.profile"),
        "wildcard head folds the nested chain exactly like sg (no trailing ?)"
    );
    assert_eq!(
        by_line(3).captures.get("M").map(String::as_str),
        Some("load")
    );
    assert_eq!(
        by_line(6).captures.get("O").map(String::as_str),
        Some("cfg")
    );
    assert_eq!(
        by_line(6).captures.get("M").map(String::as_str),
        Some("get")
    );
    assert_eq!(
        by_line(6).captures.get("$$$A").map(String::as_str),
        Some("\"k\""),
        "the rest-arg capture binds in the multi namespace like sg A=[\"k\"]"
    );

    // Token-exact: the optional template never answers the plain faces.
    let plain_src = "const r1 = a.b();\nobj.method(1, 2);\n";
    let on_plain = match_pattern(Language::TypeScript, plain_src, "$O?.$M($$$A)").unwrap();
    assert!(
        on_plain.is_empty(),
        "`?.` template must not answer `.` receivers (sg token-exact): {on_plain:?}"
    );
}

/// F64-7 (1b): head folding across call receivers, plain connectors inside
/// the folded head, and nested-chain multiplicity — all sg 0.45.2 probed:
/// `maybe()?.load()` binds O=`maybe()`; `a.b?.c()` binds O=`a.b`;
/// `conn?.open()?.send(1)` emits BOTH rows (outer O=`conn?.open()`,
/// inner O=`conn`); plain chains {1,3} stay unanswered.
#[test]
fn f64_7_scopeback_optional_head_folding_and_nesting() {
    let src = "const x = alpha.first().second();\nconst opt = maybe()?.load();\nconst z = b.second().third();\nconst opt2 = user?.profile?.load();\nconst nest = conn?.open()?.send(1);\nconst mixed = a.b?.c();\n";
    let hits = match_pattern(Language::TypeScript, src, "$O?.$M($$$A)").unwrap();
    let rows: Vec<(u32, &str)> = hits
        .iter()
        .map(|h| (h.line_start, h.excerpt.as_str()))
        .collect();
    assert_eq!(
        rows,
        vec![
            (2, "maybe()?.load()"),
            (4, "user?.profile?.load()"),
            (5, "conn?.open()?.send(1)"),
            (5, "conn?.open()"),
            (6, "a.b?.c()"),
        ],
        "outer-before-inner per line, exactly sg's rows: {rows:?}"
    );
    let by = |line: u32, text: &str| {
        hits.iter()
            .find(|h| h.line_start == line && h.excerpt == text)
            .unwrap_or_else(|| panic!("missing row {line} {text}"))
    };
    assert_eq!(
        by(2, "maybe()?.load()")
            .captures
            .get("O")
            .map(String::as_str),
        Some("maybe()"),
        "call receiver folds whole (sg O=maybe())"
    );
    assert_eq!(
        by(4, "user?.profile?.load()")
            .captures
            .get("O")
            .map(String::as_str),
        Some("user?.profile")
    );
    assert_eq!(
        by(5, "conn?.open()?.send(1)")
            .captures
            .get("O")
            .map(String::as_str),
        Some("conn?.open()")
    );
    assert_eq!(
        by(5, "conn?.open()").captures.get("O").map(String::as_str),
        Some("conn"),
        "the inner optional call answers too (sg 2 rows on line 5)"
    );
    assert_eq!(
        by(6, "a.b?.c()").captures.get("O").map(String::as_str),
        Some("a.b"),
        "the folded head may contain plain connectors (sg O=a.b)"
    );
    let plain = match_pattern(Language::TypeScript, src, "$O.$M($$$A)").unwrap();
    assert_eq!(
        lines_of(&plain),
        vec![1u32, 1, 3, 3],
        "plain template keeps the registered two-segment contract (f64_2: \
         sg answers BOTH chain nodes per line on 3-segment chains): {plain:?}"
    );
}

/// F64-7 (2): the connector token is exact in BOTH directions on js too
/// (TSX grammar): plain template answers only plain receivers, optional
/// template only optional ones, with the same captures as ts.
#[test]
fn f64_7_scopeback_js_connector_token_exact() {
    let js = "const r1 = a.b();\nconst r2 = a?.b();\nconst r3 = user?.profile?.load();\nconst r5 = cfg?.get(\"k\");\ncfg.get(\"k\");\n";
    let plain = match_pattern(Language::JavaScript, js, "$O.$M($$$A)").unwrap();
    assert_eq!(
        lines_of(&plain),
        vec![1u32, 5],
        "js plain template must not answer optional receivers: {plain:?}"
    );
    let opt = match_pattern(Language::JavaScript, js, "$O?.$M($$$A)").unwrap();
    assert_eq!(
        lines_of(&opt),
        vec![2u32, 3, 4],
        "js optional template answers exactly the optional faces: {opt:?}"
    );
    assert_eq!(
        opt.iter()
            .find(|h| h.line_start == 3)
            .unwrap()
            .captures
            .get("O")
            .map(String::as_str),
        Some("user?.profile")
    );
}

/// F64-7 (3): codemod safety — receiver captures are exactly the bytes a
/// `log($O, $M)` rewrite interpolates, so every capture must be valid
/// expression text. The proven corruption (`log(a?, b);`) is structurally
/// impossible: plain-template captures never see an optional receiver
/// (zero matches there), and optional-template captures never carry a
/// glued trailing `?`.
#[test]
fn f64_7_scopeback_captures_are_rewrite_safe() {
    // Plain faces rewrite to `log(a, b)` — the bytes sg writes.
    let plain_hits = match_pattern(Language::TypeScript, F64_7_TS_CORPUS, "$O.$M($$$A)").unwrap();
    assert_eq!(lines_of(&plain_hits), vec![1u32, 4, 5, 7, 8, 9]);
    let first = &plain_hits[0];
    let o = first.captures.get("O").map(String::as_str).unwrap();
    let m = first.captures.get("M").map(String::as_str).unwrap();
    let rewrite = format!("log({o}, {m})");
    assert_eq!(
        rewrite, "log(a, b)",
        "plain face must rewrite to sg's bytes"
    );
    assert!(!rewrite.contains('?'), "no ? may glue into a plain rewrite");

    // Zero plain-template matches on optional receivers = zero planned edits
    // there = the 65d corruption (`log(a?, b);`) cannot be produced.
    let opt_only = "const r2 = a?.b();\nconst r3 = user?.profile?.load();\n";
    assert!(
        match_pattern(Language::TypeScript, opt_only, "$O.$M($$$A)")
            .unwrap()
            .is_empty(),
        "plain template must plan zero edits on optional chains"
    );

    // The explicit optional template rewrites through clean captures: no
    // capture value ends with `?` (the glued-? class), so interpolating any
    // capture is syntactically valid.
    let opt_hits = match_pattern(Language::TypeScript, F64_7_TS_CORPUS, "$O?.$M($$$A)").unwrap();
    assert!(!opt_hits.is_empty());
    for hit in &opt_hits {
        for (name, value) in &hit.captures {
            assert!(
                !value.ends_with('?'),
                "capture {name}={value:?} carries a glued trailing ? (the 65d corruption class)"
            );
        }
    }
    let folded = opt_hits
        .iter()
        .find(|h| h.line_start == 3)
        .expect("folded row");
    let o = folded.captures.get("O").map(String::as_str).unwrap();
    let m = folded.captures.get("M").map(String::as_str).unwrap();
    assert_eq!(
        format!("log({o}, {m})"),
        "log(user?.profile, load)",
        "the optional face rewrites to the optional form sg would emit"
    );
}

/// F64-7 guards: languages without a `?.` member connector keep today's
/// contract. python has no optional chaining (sg refuses the pattern);
/// rust's `?` is the try operator — sg's `$O?.$M($$$A)` there means
/// try-heads (a DIFFERENT family, out of F64-7 scope) and stays refused;
/// the rust plain template keeps matching try-chains with sg's hit set
/// (sg 0.45.2: {2,3}); csharp/kotlin plain templates keep sg's
/// connector-exact {3}-only answer; php keeps its registered `->` empty.
#[test]
fn f64_7_scopeback_other_languages_unaffected() {
    assert!(
        !native_pattern_answerable(Language::Python, "$O?.$M($$$A)"),
        "py has no optional chaining; sg refuses the pattern"
    );
    assert!(
        !native_pattern_answerable(Language::Rust, "$O?.$M($$$A)"),
        "rust ? is the try operator, not a member connector — refusal unchanged"
    );
    assert!(
        match_pattern(Language::Python, "y = plain.value()\n", "$O?.$M($$$A)")
            .unwrap()
            .is_empty()
    );
    assert!(match_pattern(
        Language::Rust,
        "fn f() {\n    let y = a?.b();\n}\n",
        "$O?.$M($$$A)"
    )
    .unwrap()
    .is_empty());

    // rust plain template on try-chains: sg answers all three rows (the ?
    // belongs to the receiver expression, not the connector).
    let rs = "fn f() -> Result<u32, E> {\n    let y = a?.b();\n    let z = fallback()?.method();\n    let w = plain.value();\n    Ok(1)\n}\n";
    let hits = match_pattern(Language::Rust, rs, "$O.$M($$$A)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2u32, 3, 4],
        "rust try-chains keep answering the plain template (sg 2,3,4): {hits:?}"
    );

    // csharp/kotlin: sg is connector-exact on plain templates too.
    let cs = "var y = a?.b();\nvar z = obj?.Run(1);\nvar w = plain.value();\n";
    assert_eq!(
        lines_of(&match_pattern(Language::CSharp, cs, "$O.$M($$$A)").unwrap()),
        vec![3u32],
        "csharp plain template must not answer null-conditional receivers (sg line 3 only)"
    );
    let kt = "val y = a?.b()\nval z = obj?.run(1)\nval w = plain.value()\n";
    assert_eq!(
        lines_of(&match_pattern(Language::Kotlin, kt, "$O.$M($$$A)").unwrap()),
        vec![3u32],
        "kotlin plain template must not answer safe-call receivers (sg line 3 only)"
    );
    // PASS 66a flip 1 (regression guard): kotlin `$O?.$M($$$A)` answers
    // sg-exactly — {3,4,5} on the opt_kt corpus, with the wildcard head
    // folding `user?.profile` on the double-safe-call line.
    let kt_opt = "fun m() {\n    val a1 = a.b()\n    val a2 = a?.b()\n    val a3 = cfg?.get(\"k\", 1)\n    val a4 = user?.profile?.load()\n}\n";
    let kt_hits = match_pattern(Language::Kotlin, kt_opt, "$O?.$M($$$A)").unwrap();
    assert_eq!(
        klass_lines(&kt_hits),
        vec![3u32, 4, 5],
        "kotlin optional template must answer sg-exactly (66a flip 1): {kt_hits:?}"
    );
    assert!(native_pattern_answerable(Language::Kotlin, "$O?.$M($$$A)"));
    let php = "<?php\n$y = $plain->value();\n";
    assert!(
        match_pattern(Language::Php, php, "$O.$M($$$A)")
            .unwrap()
            .is_empty(),
        "php dot-templates stay empty (registered `->` semantics)"
    );
    // PASS 66a rider guards: the swift/csharp optional refusals are
    // REGISTERED divergences (sg answers both) — the gates must keep them
    // fail-closed until an owner generalizes those grammars.
    assert!(
        !native_pattern_answerable(Language::CSharp, "$O?.$M($$$A)"),
        "csharp `$O?.$M($$$A)` keeps the registered EXIT2 refusal (sg answers 4,5,6)"
    );
    assert!(
        !native_pattern_answerable(Language::Swift, "$O?.$M($$$A)"),
        "swift keeps the registered refusal (sg answers the plain-chain face)"
    );
}

/// P7 regression guard: universal/literal emission keeps byte order and
/// never duplicates a byte range. Passes pre-fix (the linear scan dedups);
/// the hash-set rewrite must keep it green, and the drop-dedup mutant must
/// kill it.
#[test]
fn f64_walk_universal_dedup_preserves_order_and_uniqueness() {
    let src = "fn main() { alpha }\nfn beta() { alpha alpha }\n";
    for pattern in ["alpha", "$$A"] {
        let hits = match_pattern(Language::Rust, src, pattern).unwrap();
        assert!(!hits.is_empty(), "{pattern} must answer");
        let ranges: Vec<(usize, usize)> = hits.iter().map(|h| (h.byte_start, h.byte_end)).collect();
        // The sound pre-order invariant: starts never decrease; on a tie the
        // OUTER (longer) range comes first (parents precede children); and no
        // byte range repeats. The duplicate clause is the dedup contract's
        // mutation kill cell: same-span statement/identifier pairs (the
        // bare `alpha` expression_statement spans exactly its identifier)
        // answer once, so the drop-dedup mutant doubles a range and dies.
        assert!(
            ranges
                .windows(2)
                .all(|w| { w[0].0 < w[1].0 || (w[0].0 == w[1].0 && w[0].1 > w[1].1) }),
            "{pattern} emission must stay in pre-order and duplicate-free: {ranges:?}"
        );
        let unique: std::collections::HashSet<(usize, usize)> = ranges.iter().copied().collect();
        assert_eq!(
            unique.len(),
            ranges.len(),
            "{pattern} duplicated a byte range"
        );
    }
}

// ---------------------------------------------------------------------------
// PASS 67a — r17 remediation of the pass-66a findings (RED-first pins).
// Every expected set below is a live sg 0.45.2 probe against these exact
// sources (re-probed this pass in /tmp/pass67a: ch_rs/ch_ts/opt_ts/pp_c/
// pp_cpp/opt_php/h_rb fixtures of artifacts/conformance/pass66a + scratch
// rule probes). Failure IS the discrimination claim: each test names the
// mutant classes it kills.
// ---------------------------------------------------------------------------

/// The pass-66a chain fixture (fx/ch_rs), inline verbatim.
const F66A_RS_CORPUS: &str = "struct S { f: u32 }\nimpl S {\n    fn a(&self) -> S { S { f: 1 } }\n    fn b(&self) -> S { S { f: 2 } }\n    fn c(&self) -> u32 { self.f }\n}\nfn main() {\n    let s = S { f: 0 };\n    let r = s.a().b().c();\n    let t = s.a().a().c();\n    let four = s.a().b().a().c();\n    s.a().b().c();\n    let q = s.a().c().b();\n    let deep = Deep { x: 1 };\n    let v = deep.x.y().z();\n    let z = s.a();\n}\n";

/// The pass-66a chain fixture (fx/ch_ts), inline verbatim.
const F66A_TS_CORPUS: &str = "const a1 = alpha.first().second();\nconst a2 = alpha.beta().gamma().delta();\nconst a3 = user?.profile?.load();\nconst a4 = conn?.open()?.send(1);\nconst a5 = a?.b().c();\nconst a6 = x.y.z();\nconst a7 = maybe()?.load();\nconst a8 = obj.method(1, 2);\nconst a9 = a.b.c().d();\n";

/// F66a-1: a 3-segment chain's leading METAVARIABLE head absorbs a
/// property-access receiver prefix exactly like sg (0.45.2: `deep.x.y().z()`
/// answers O=`deep.x` M1=y M2=z; `a.b.c().d()` answers O=`a.b`), while a
/// LITERAL head pins the chain to the exact segment count (no absorption:
/// `alpha.$M1($$$A).$M2($$$B)` answers `alpha.beta().gamma().delta()` only
/// through the exact-length inner node, M1=beta M2=gamma — one row). The
/// same-name veto must hold through the absorbed head, and the per-segment
/// argument check must fire on receiver-headed chains (sg rejects
/// `x.a(9).b(3)` for `$O.$M1().$M2($$$B)`; the misaligned inner-call zip
/// previously skipped that check).
/// PASS 67a scope-back (sg 0.45.2 probes): a folded head absorbs head-INTERNAL
/// optional links and binds its EXPRESSION span — O=`a?.b()` on
/// `a?.b().c().d()`, O=`s.a().b()` on `s.a().b().a().c()` — while the
/// `?.` veto stays position-scoped (an optional connector INTO an aligned
/// call segment still refuses: `x?.y().z()` answers nothing).
/// Mutant cells: drop head absorption (prop faces silent — the RED state);
/// absorb behind a literal head (the exact-length capture pin doubles);
/// drop the receiver-call walk (inner prop-call node over-answers O=deep);
/// drop the inner argument check (the arity-twin line answers); restore the
/// wholesale member veto (the fold-corpus line 1 face goes silent — the
/// pre-67a state); bind the leaf-joined head (`a?.b` / `s.a().b` — the
/// capture cells).
#[test]
fn f66a_1_chain_heads_absorb_property_receivers() {
    let three = match_pattern(Language::Rust, F66A_RS_CORPUS, "$O.$M1($$$A).$M2($$$B)").unwrap();
    assert_eq!(
        klass_lines(&three),
        vec![9u32, 10, 11, 12, 13, 15],
        "3-seg chains must answer prop-head receivers like sg: {three:?}"
    );
    let prop = three
        .iter()
        .find(|h| h.line_start == 15)
        .expect("the deep.x.y().z() face must answer");
    assert_eq!(
        prop.captures.get("O").map(String::as_str),
        Some("deep.x"),
        "the wildcard head absorbs the property prefix exactly like sg"
    );
    assert_eq!(prop.captures.get("M1").map(String::as_str), Some("y"));
    assert_eq!(prop.captures.get("M2").map(String::as_str), Some("z"));

    let ts_three = match_pattern(
        Language::TypeScript,
        F66A_TS_CORPUS,
        "$O.$M1($$$A).$M2($$$B)",
    )
    .unwrap();
    assert_eq!(
        klass_lines(&ts_three),
        vec![1u32, 2, 9],
        "ts prop-head face (line 9, O=a.b) must answer; the call-free chain \
         x.y.z() (line 6) answers in neither engine: {ts_three:?}"
    );
    let ab = ts_three
        .iter()
        .find(|h| h.line_start == 9)
        .expect("a.b.c().d() must answer");
    assert_eq!(ab.captures.get("O").map(String::as_str), Some("a.b"));

    // Literal head pins the exact length: sg answers line 1 (M1=first) and
    // line 2 ONLY through the exact-length inner node (M1=beta M2=gamma) —
    // never the absorbed outer row (M1=gamma). One row per line.
    let lit = match_pattern(
        Language::TypeScript,
        F66A_TS_CORPUS,
        "alpha.$M1($$$A).$M2($$$B)",
    )
    .unwrap();
    let lit_rows: Vec<(u32, &str, &str)> = lit
        .iter()
        .map(|h| {
            (
                h.line_start,
                h.captures.get("M1").map(String::as_str).unwrap_or(""),
                h.captures.get("M2").map(String::as_str).unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(
        lit_rows,
        vec![(1u32, "first", "second"), (2, "beta", "gamma")],
        "literal head must pin the exact segment count (no absorption): {lit_rows:?}"
    );

    // Same-name veto survives absorption: sg answers NOTHING for
    // `$O.$O($$$A).$O($$$B)` on `y.f().y().y()` (absorbed head O=y.f can
    // never equal the tail O bindings) while the exact-length `y.y().y()`
    // face keeps answering.
    let dup =
        "fn main() {\n    let y = Y;\n    let a = y.y().y();\n    let b = y.f().y().y();\n}\n";
    let dup_hits = match_pattern(Language::Rust, dup, "$O.$O($$$A).$O($$$B)").unwrap();
    assert_eq!(
        klass_lines(&dup_hits),
        vec![3u32],
        "same-name veto holds through absorbed heads (only y.y().y()): {dup_hits:?}"
    );

    // Inner argument check fires on receiver-headed chains: sg answers only
    // the arity-correct line.
    let arity = "const t1 = x.a(9).b(3);\nconst t2 = x.a().b(3);\n";
    let arity_hits = match_pattern(Language::TypeScript, arity, "$O.$M1().$M2($$$B)").unwrap();
    assert_eq!(
        klass_lines(&arity_hits),
        vec![2u32],
        "the receiver call's argument template must be checked (sg line 2 only): {arity_hits:?}"
    );

    // PASS 67a scope-back: head-internal optional connectors fold and the
    // head binds its EXPRESSION span (sg 0.45.2 probed on these exact
    // sources); the position-scoped veto keeps `x?.y().z()` silent (the `?.`
    // is the connector INTO the aligned call segment there).
    let fold = "const f1 = a?.b().c().d();\nconst f2 = x?.y().z();\nconst f3 = a.b.c().d();\n";
    let fold_hits = match_pattern(Language::TypeScript, fold, "$O.$M1($$$A).$M2($$$B)").unwrap();
    assert_eq!(
        klass_lines(&fold_hits),
        vec![1u32, 3],
        "folded optional-call heads answer; the aligned-optional face does not: {fold_hits:?}"
    );
    let f1 = fold_hits
        .iter()
        .find(|h| h.line_start == 1)
        .expect("fold-corpus line 1 row");
    assert_eq!(
        f1.captures.get("O").map(String::as_str),
        Some("a?.b()"),
        "the folded head binds the whole expression, connector bytes excluded"
    );
    assert_eq!(f1.captures.get("M1").map(String::as_str), Some("c"));
    assert_eq!(f1.captures.get("M2").map(String::as_str), Some("d"));
    let f3 = fold_hits
        .iter()
        .find(|h| h.line_start == 3)
        .expect("fold-corpus line 3 row");
    assert_eq!(f3.captures.get("O").map(String::as_str), Some("a.b"));
    // Absorbed CALL heads bind the expression span through the call parens
    // (sg line 11 row: O=`s.a().b()`), never the leaf-joined text.
    let rs_four = three
        .iter()
        .find(|h| h.line_start == 11)
        .expect("the four-call rs line must answer");
    assert_eq!(
        rs_four.captures.get("O").map(String::as_str),
        Some("s.a().b()"),
        "an absorbed call head binds the expression span (sg 0.45.2)"
    );
}

/// F66a-3: the ts optional chain `$O?.$M1($$$A).$M2($$$B)` answers
/// `a?.b().c()` (O=a M1=b M2=c) and `x?.y().z()` — sg 0.45.2 ANSWERS the
/// shape (this corrects 65f's registered sg-EMPTY claim). The spelled
/// connectors are token-exact: the `?.` must link the head into M1's call
/// and every later connector must be plain `.` — the optional-mid sources
/// (`user?.profile?.load()`, `conn?.open()?.send(1)`) stay unanswered, and
/// the plain template still never answers an optional tail.
/// PASS 67a scope-back (sg 0.45.2 probes): the lane generalizes to
/// per-connector flags — the MID-chain optional spelling `$O.$M1()?.$M2($$$B)`
/// is native and answers exactly its `?.`-linked faces; a 4-seg optional-head
/// template answers the deeper chain; a folded CALL head binds the
/// expression span WITH its argument list (O=`a.b()`).
/// Mutant cells: drop the later-connector-plain rule (lines 3/4 answer);
/// drop the head-optional rule (plain faces answer the optional template);
/// refuse the shape outright (RED state — silent-0/EXIT2); flip one
/// mid-chain flag (the m2/m3 faces over- or under-answer); bind the
/// leaf-span head (`a.b` on the c1 face — the capture cell).
#[test]
fn f66a_3_optional_chains_answer_mid_chain_faces() {
    assert!(
        !needs_ast_grep_fallback("$O?.$M1($$$A).$M2($$$B)"),
        "the 3-seg optional chain must be native (sg 0.45.2 answers it)"
    );
    assert!(native_pattern_answerable(
        Language::TypeScript,
        "$O?.$M1($$$A).$M2($$$B)"
    ));
    assert!(native_pattern_answerable(
        Language::JavaScript,
        "$O?.$M1($$$A).$M2($$$B)"
    ));

    let hits = match_pattern(
        Language::TypeScript,
        F66A_TS_CORPUS,
        "$O?.$M1($$$A).$M2($$$B)",
    )
    .unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![5u32],
        "optional chain answers exactly a?.b().c() (sg): {hits:?}"
    );
    let row = hits.first().expect("line 5 row");
    assert_eq!(row.captures.get("O").map(String::as_str), Some("a"));
    assert_eq!(row.captures.get("M1").map(String::as_str), Some("b"));
    assert_eq!(row.captures.get("M2").map(String::as_str), Some("c"));

    let opt = "const a1 = a.b();\nconst a2 = a?.b();\nconst a3 = cfg?.get(\"k\", 1);\nconst a4 = user?.name;\nconst a5 = user?.profile?.load();\nconst a6 = a?.b?.c?.d();\nconst a7 = x?.y().z();\nconst a8 = conn?.open()?.send(1);\nconst a9 = maybe()?.load();\nconst a10 = user?.$M();\n";
    let opt_hits = match_pattern(Language::TypeScript, opt, "$O?.$M1($$$A).$M2($$$B)").unwrap();
    assert_eq!(
        klass_lines(&opt_hits),
        vec![7u32],
        "x?.y().z() answers; optional-mid chains (5,8) do not (sg): {opt_hits:?}"
    );
    assert_eq!(
        opt_hits
            .first()
            .unwrap()
            .captures
            .get("O")
            .map(String::as_str),
        Some("x")
    );

    // Head absorption folds plain member prefixes (sg: O=a.b on
    // `a.b?.c().d()`); a mid-optional source never answers.
    let mix = "const h = deep?.x.y().z();\nconst k = a.b?.c().d();\nconst l = u?.v()?.w();\n";
    let mix_hits = match_pattern(Language::TypeScript, mix, "$O?.$M1($$$A).$M2($$$B)").unwrap();
    let rows: Vec<(u32, &str)> = mix_hits
        .iter()
        .map(|h| {
            (
                h.line_start,
                h.captures.get("O").map(String::as_str).unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![(2u32, "a.b")],
        "only the plain-mid optional chain answers, with the folded head: {rows:?}"
    );

    // The plain template keeps its veto (connector-scoped, both directions).
    let plain = match_pattern(
        Language::TypeScript,
        F66A_TS_CORPUS,
        "$O.$M1($$$A).$M2($$$B)",
    )
    .unwrap();
    assert_eq!(
        klass_lines(&plain),
        vec![1u32, 2, 9],
        "plain chains must not gain the optional face (line 5): {plain:?}"
    );

    // PASS 67a scope-back: the MID-chain optional spelling is native and
    // token-exact per connector (sg 0.45.2 probed on these exact sources).
    assert!(
        !needs_ast_grep_fallback("$O.$M1()?.$M2($$$B)"),
        "the mid-chain optional spelling must be native (sg answers it)"
    );
    assert!(native_pattern_answerable(
        Language::TypeScript,
        "$O.$M1()?.$M2($$$B)"
    ));
    let mid = "const m1 = a.b()?.c();\nconst m2 = a.b().c();\nconst m3 = a?.b()?.c();\n";
    let mid_hits = match_pattern(Language::TypeScript, mid, "$O.$M1()?.$M2($$$B)").unwrap();
    assert_eq!(
        klass_lines(&mid_hits),
        vec![1u32],
        "the mid template answers exactly the ?. face: {mid_hits:?}"
    );
    let m1 = mid_hits.first().expect("m1 row");
    assert_eq!(m1.captures.get("O").map(String::as_str), Some("a"));
    assert_eq!(m1.captures.get("M1").map(String::as_str), Some("b"));
    assert_eq!(m1.captures.get("M2").map(String::as_str), Some("c"));
    // Alignment in the other direction: the all-plain call chain answers
    // only the plain template.
    let mid_plain = match_pattern(Language::TypeScript, mid, "$O.$M1($$$A).$M2($$$B)").unwrap();
    assert_eq!(
        klass_lines(&mid_plain),
        vec![2u32],
        "the plain template must not gain the mid-optional face (line 1): {mid_plain:?}"
    );

    // A folded CALL head binds the expression span WITH its argument list
    // (sg 0.45.2: O=`a.b()` on `a.b()?.c().d()`), and the mid template
    // answers the same corpus through the inner sub-chain rows.
    let span_src = "const c1 = a.b()?.c().d();\nconst c2 = a.b()?.c();\n";
    let span_hits =
        match_pattern(Language::TypeScript, span_src, "$O?.$M1($$$A).$M2($$$B)").unwrap();
    assert_eq!(
        klass_lines(&span_hits),
        vec![1u32],
        "the 3-seg optional template answers the deep face only: {span_hits:?}"
    );
    assert_eq!(
        span_hits
            .first()
            .unwrap()
            .captures
            .get("O")
            .map(String::as_str),
        Some("a.b()"),
        "the folded call head keeps its argument list (sg capture)"
    );
    let span_mid = match_pattern(Language::TypeScript, span_src, "$O.$M1()?.$M2($$$B)").unwrap();
    assert_eq!(
        klass_lines(&span_mid),
        vec![1u32, 2],
        "the mid template answers both inner a.b()?.c() rows: {span_mid:?}"
    );
    for hit in &span_mid {
        assert_eq!(hit.captures.get("O").map(String::as_str), Some("a"));
    }

    // 4-seg optional-head template (sg answers the deeper chain; the
    // misaligned faces stay silent).
    assert!(
        !needs_ast_grep_fallback("$O?.$M1($$$A).$M2($$$B).$M3($$$C)"),
        "the 4-seg optional chain must be native"
    );
    let deep = "const p1 = a?.b().c().d();\nconst p3 = a.b()?.c().d();\nconst p6 = a.b.c().d();\n";
    let deep_hits = match_pattern(
        Language::TypeScript,
        deep,
        "$O?.$M1($$$A).$M2($$$B).$M3($$$C)",
    )
    .unwrap();
    assert_eq!(
        klass_lines(&deep_hits),
        vec![1u32],
        "the 4-seg optional template answers exactly the head-optional chain: {deep_hits:?}"
    );
    let p1 = deep_hits.first().expect("p1 row");
    assert_eq!(p1.captures.get("O").map(String::as_str), Some("a"));
    assert_eq!(p1.captures.get("M3").map(String::as_str), Some("d"));
}

/// F66a-2: tree-sitter-c (0.24.x, like cpp) folds BOTH spellings into
/// `preproc_ifdef` — there is no dedicated `preproc_ifndef` kind in the
/// vendored grammar, so the c kind map must answer `#ifndef` through the
/// shared kind with the anonymous head-token check. sg answers c `#ifndef
/// $A` with A=MISSING exactly like cpp.
/// Mutant cells: revert to the dead `preproc_ifndef` mapping (silent-0 —
/// the RED state); drop the anonymous-token disambiguation (`#ifdef`
/// patterns would answer `#ifndef` regions and vice versa).
#[test]
fn f66a_2_c_ifndef_answers_through_shared_kind() {
    let c = "#define FEATURE_A 1\n#define FEATURE_B 1\n\nint main(void) {\n#if defined(FEATURE_A) && defined(FEATURE_B)\n    return 1;\n#elif defined(FEATURE_A)\n    return 2;\n#else\n    return 3;\n#endif\n}\n\n#ifdef FEATURE_A\n#ifdef FEATURE_B\nint nested_both(void) { return 0; }\n#endif\nint nested_outer(void) { return 1; }\n#endif\n\n#ifndef MISSING\nint has_not(void) { return 2; }\n#endif\n\n#if FEATURE_A\nint plain_if(void) { return 3; }\n#endif\n\n#undef FEATURE_B\nint after_undef(void) { return 4; }\n";
    let ifndef = match_pattern(Language::C, c, "#ifndef $A").unwrap();
    assert_eq!(
        lines_of(&ifndef),
        vec![21u32],
        "c #ifndef must answer like sg: {ifndef:?}"
    );
    assert_eq!(
        ifndef[0].captures.get("A").map(String::as_str),
        Some("MISSING"),
        "the directive head binds like sg"
    );
    let concrete = match_pattern(Language::C, c, "#ifndef MISSING").unwrap();
    assert_eq!(lines_of(&concrete), vec![21u32], "{concrete:?}");

    // The head-token check keeps ifdef/ifndef disjoint on the shared kind.
    let ifdef = match_pattern(Language::C, c, "#ifdef FEATURE_A").unwrap();
    assert_eq!(
        lines_of(&ifdef),
        vec![14u32],
        "#ifdef must not reach the #ifndef region: {ifdef:?}"
    );
    let wrong = match_pattern(Language::C, c, "#ifdef MISSING").unwrap();
    assert!(
        wrong.is_empty(),
        "#ifdef must not answer the ifndef region: {wrong:?}"
    );
    let ifndef_wrong = match_pattern(Language::C, c, "#ifndef FEATURE_A").unwrap();
    assert!(
        ifndef_wrong.is_empty(),
        "#ifndef must not answer the ifdef regions: {ifndef_wrong:?}"
    );

    // cpp parity unchanged (the shared-kind rule was already right there).
    let cpp = "#define FEATURE_A 1\n#define FEATURE_B 1\n\nint main() {\n#if defined(FEATURE_A) && defined(FEATURE_B)\n    return 1;\n#elif defined(FEATURE_A)\n    return 2;\n#endif\n}\n\n#ifndef MISSING\nint has_not() { return 3; }\n#endif\n";
    let cpp_ifndef = match_pattern(Language::Cpp, cpp, "#ifndef $A").unwrap();
    assert_eq!(lines_of(&cpp_ifndef), vec![12u32], "{cpp_ifndef:?}");
}

/// F66a-4: compound `#if` conditions with 2+ metavariables answer like sg —
/// the condition template unifies STRUCTURALLY (sg binds `$A` in
/// `#if $A && defined($B)` to the WHOLE left operand `defined(FEATURE_A)`,
/// not to a text slice), and a parseable condition with no matching arm
/// (`||` form) is answerable-and-empty, not EXIT2.
/// Mutant cells: text-slice binding (A=FEATURE_A on the mixed face — the
/// capture cell); refuse 2+ metas outright (RED state); structural mutant
/// binding inside `defined()` on the mixed face (A must stay the whole
/// operand).
#[test]
fn f66a_4_compound_preproc_conditions_answer_like_sg() {
    let c = "#define FEATURE_A 1\n#define FEATURE_B 1\n\nint main(void) {\n#if defined(FEATURE_A) && defined(FEATURE_B)\n    return 1;\n#elif defined(FEATURE_A)\n    return 2;\n#else\n    return 3;\n#endif\n}\n\n#ifdef FEATURE_A\n#ifdef FEATURE_B\nint nested_both(void) { return 0; }\n#endif\nint nested_outer(void) { return 1; }\n#endif\n\n#ifndef MISSING\nint has_not(void) { return 2; }\n#endif\n\n#if FEATURE_A\nint plain_if(void) { return 3; }\n#endif\n\n#undef FEATURE_B\nint after_undef(void) { return 4; }\n";
    let both = match_pattern(Language::C, c, "#if defined($A) && defined($B)").unwrap();
    assert_eq!(lines_of(&both), vec![5u32], "{both:?}");
    assert_eq!(
        both[0].captures.get("A").map(String::as_str),
        Some("FEATURE_A")
    );
    assert_eq!(
        both[0].captures.get("B").map(String::as_str),
        Some("FEATURE_B")
    );

    // THE structural cell: `$A` = the whole left operand (sg 0.45.2).
    let mixed = match_pattern(Language::C, c, "#if $A && defined($B)").unwrap();
    assert_eq!(lines_of(&mixed), vec![5u32], "{mixed:?}");
    assert_eq!(
        mixed[0].captures.get("A").map(String::as_str),
        Some("defined(FEATURE_A)"),
        "sg binds the leading metavariable to the whole operand"
    );
    assert_eq!(
        mixed[0].captures.get("B").map(String::as_str),
        Some("FEATURE_B")
    );

    // Parseable-but-unmatched compound conditions are answerable-and-empty.
    assert!(
        !needs_ast_grep_fallback("#if defined($A) || defined($B)"),
        "the || compound parses in sg (exit 0, empty) — must be native"
    );
    assert!(native_pattern_answerable(
        Language::C,
        "#if defined($A) || defined($B)"
    ));
    assert!(
        match_pattern(Language::C, c, "#if defined($A) || defined($B)")
            .unwrap()
            .is_empty(),
        "no || arm exists in the corpus — empty like sg"
    );

    // Guards: the single-metavar contracts stay byte-identical.
    let whole = match_pattern(Language::C, c, "#if $A").unwrap();
    assert_eq!(lines_of(&whole), vec![5u32, 25], "{whole:?}");
    assert_eq!(
        whole[0].captures.get("A").map(String::as_str),
        Some("defined(FEATURE_A) && defined(FEATURE_B)"),
        "Whole binding of a lone metavariable is unchanged"
    );
    assert_eq!(
        whole[1].captures.get("A").map(String::as_str),
        Some("FEATURE_A")
    );
    // The Inner face is corpus-shaped (sg 0.45.2 probed both): on a SIMPLE
    // condition the registered pass-65 contract binds inside (A = the
    // identifier); on the COMPOUND condition the Inner template cannot unify
    // and sg answers empty — the Structural lane owns the compound faces.
    let inner_simple = match_pattern(
        Language::C,
        "#if defined(FEATURE_A)\nint a(void);\n#endif\n",
        "#if defined($A)",
    )
    .unwrap();
    assert_eq!(lines_of(&inner_simple), vec![1u32], "{inner_simple:?}");
    assert_eq!(
        inner_simple[0].captures.get("A").map(String::as_str),
        Some("FEATURE_A")
    );
    assert!(
        match_pattern(Language::C, c, "#if defined($A)")
            .unwrap()
            .is_empty(),
        "the Inner template stays empty on the compound condition (sg parity)"
    );
}

/// F66a-5: `#define $A` answers object-like macro defines (with or without
/// a value) on c and cpp exactly like sg; `#define $A $B` additionally
/// binds the value and demands one; function-like defines are a different
/// kind and never answer the object-like template.
/// Mutant cells: answer function-like defines too (kind collapse — line 3
/// of the corpus answers); drop the value-presence demand (`$A $B`
/// answering a valueless define); drop the lane (RED silent-0).
#[test]
fn f66a_5_object_like_define_answers_like_sg() {
    let src =
        "#define PLAIN\n#define VAL 1\n#define FUNC(x) ((x)+1)\n#define OTHER 2\nint v = VAL;\n";
    let name = match_pattern(Language::C, src, "#define $A").unwrap();
    assert_eq!(
        lines_of(&name),
        vec![1u32, 2, 4],
        "object-like defines answer; FUNC (function-like) does not: {name:?}"
    );
    assert_eq!(name[0].captures.get("A").map(String::as_str), Some("PLAIN"));
    assert_eq!(name[1].captures.get("A").map(String::as_str), Some("VAL"));

    let valued = match_pattern(Language::C, src, "#define $A $B").unwrap();
    assert_eq!(
        lines_of(&valued),
        vec![2u32, 4],
        "a spelled value must exist to answer: {valued:?}"
    );
    assert_eq!(valued[0].captures.get("A").map(String::as_str), Some("VAL"));
    assert_eq!(valued[0].captures.get("B").map(String::as_str), Some("1"));
    assert_eq!(
        valued[1].captures.get("A").map(String::as_str),
        Some("OTHER")
    );
    assert_eq!(valued[1].captures.get("B").map(String::as_str), Some("2"));

    let concrete = match_pattern(Language::C, src, "#define VAL").unwrap();
    assert_eq!(
        lines_of(&concrete),
        vec![2u32],
        "concrete object-like define answers head-level (value ignored): {concrete:?}"
    );

    // cpp parity (the pass-66a probe: sg {1,2} on pp_cpp) and the pinned
    // general-lane face (`#define $X $Y` on `#define MAX 3`) stay green.
    let cpp = "#define FEATURE_A 1\n#define FEATURE_B 1\nint main() { return 0; }\n";
    let cpp_defines = match_pattern(Language::Cpp, cpp, "#define $A").unwrap();
    assert_eq!(lines_of(&cpp_defines), vec![1u32, 2], "{cpp_defines:?}");
    let maxes = match_pattern(
        Language::C,
        "#include <a.h>\n#define MAX 3\n",
        "#define $X $Y",
    )
    .unwrap();
    assert_eq!(lines_of(&maxes), vec![2u32], "{maxes:?}");
    assert_eq!(maxes[0].captures.get("X").map(String::as_str), Some("MAX"));
    assert_eq!(maxes[0].captures.get("Y").map(String::as_str), Some("3"));
}

/// F66a-6: the php nullsafe connector `?->` joins the optional lane with
/// sg's token-exact rule (0.45.2: `$O?->$M($$$A)` answers exactly the
/// `?->` receivers — never the plain `->` ones — and folds wildcard heads
/// across nullsafe links, O=`g?->h` on `g?->h?->i()`). The plain DOT
/// template keeps rejecting php receivers entirely.
/// Mutant cells: accept the plain `->` token as the connector (line 2 of
/// the corpus answers); drop the lane (RED EXIT2); fold `.` into the
/// connector check (dot template over-answers).
#[test]
fn f66a_6_php_nullsafe_connector_answers_token_exact() {
    assert!(
        native_pattern_answerable(Language::Php, "$O?->$M($$$A)"),
        "php nullsafe must be native (sg 0.45.2 answers it)"
    );
    let php =
        "<?php\n$a1 = a->b();\n$a2 = a?->b();\n$a3 = $cfg?->get(\"k\", 1);\n$a4 = g?->h?->i();\n";
    let hits = match_pattern(Language::Php, php, "$O?->$M($$$A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![3u32, 4, 5],
        "nullsafe faces answer; the plain -> face (line 2) does not: {hits:?}"
    );
    let by_line = |line: u32| {
        hits.iter()
            .find(|h| h.line_start == line)
            .unwrap_or_else(|| panic!("missing hit on line {line}"))
    };
    assert_eq!(by_line(3).captures.get("O").map(String::as_str), Some("a"));
    assert_eq!(by_line(3).captures.get("M").map(String::as_str), Some("b"));
    assert_eq!(
        by_line(4).captures.get("O").map(String::as_str),
        Some("$cfg")
    );
    assert_eq!(
        by_line(4).captures.get("M").map(String::as_str),
        Some("get")
    );
    assert_eq!(
        by_line(5).captures.get("O").map(String::as_str),
        Some("g?->h"),
        "the wildcard head folds across nullsafe links (sg O=g?->h)"
    );

    // Token-exact in the other direction: the plain-arrow spelling never
    // answers nullsafe receivers, and dot patterns stay empty (registered).
    let plain_arrow = match_pattern(Language::Php, php, "$O->$M($$$A)");
    let _ = plain_arrow; // unregistered face (subject refuses today) — no pin.
    let dot = match_pattern(Language::Php, php, "$O.$M($$$A)").unwrap();
    assert!(
        dot.is_empty(),
        "php dot templates must never answer -> or ?-> receivers: {dot:?}"
    );
    let opt_on_plain =
        match_pattern(Language::Php, "<?php\n$p = a->b();\n", "$O?->$M($$$A)").unwrap();
    assert!(
        opt_on_plain.is_empty(),
        "the ?-> template must not answer plain -> receivers (token-exact): {opt_on_plain:?}"
    );
}

/// F66a-8: ruby bare `raise` answers sg's identifier-level set — tree-
/// sitter-ruby has no raise_statement kind, and sg's bare pattern matches
/// every raise (the pure bare statement at line 12 included) exactly like
/// the F64-6 statement-head rule predicts.
/// Mutant cells: kind-collapse to `call` (every ruby call would answer);
/// drop the fix (RED silent-0).
#[test]
fn f66a_8_ruby_bare_raise_answers_identifier_level() {
    let rb = "def risky\n  raise \"boom\"\nrescue ArgumentError => e\n  raise e\nrescue => ex\n  raise ArgumentError, \"wrapped\"\nend\n\ndef call_it\n  risky\nrescue ZeroDivisionError\n  raise\nend\n\nbegin\n  raise CustomError.new(\"x\")\nend\n";
    let hits = match_pattern(Language::Ruby, rb, "raise").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![2u32, 4, 6, 12, 16],
        "ruby bare raise must answer sg's set incl. the pure bare statement: {hits:?}"
    );
    for hit in &hits {
        assert_eq!(
            hit.captures.get("MATCH").map(String::as_str),
            Some("raise"),
            "sg matches at identifier level (row text is the raise token): {hit:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// PASS 69a (r19 remediation): F68a-1 + F26-0182 + F-68c-1.
//
// sg 0.45.2 meta-name grammar, COMPLETELY probed 2026-09-04 (oracle table):
//   CANONICAL META: `$NAME` / `$$$NAME` with NAME ∈ [A-Z_][A-Z0-9_]* —
//     `$A`, `$A1`, `$AB1`, `$A_B`, `$A_`, `$_`, `$_A` all wildcard-match a
//     different identifier in the source (probed in py AND php AND js).
//   NON-META tokens (any lowercase byte in NAME):
//     - js/ts (`$` is identifier syntax): LITERAL code — `const $x = 1`
//       answers line 1 only, never `const y = 1`; `$ABc + 2` answers its
//       own line; `text.$B0x1F(z)` answers its own line.
//     - php (lowercase-LED): LITERAL variable code (`$x = 1` answers its
//       own line, never `$y = 1`).
//     - php (uppercase/underscore-LED with lowercase tail): POISON — the
//       whole pattern parses to an ERROR node (sg warns, rc=0) and answers
//       NOTHING, not even the token's own line (`echo $ABc;` → empty).
//     - py/rust/`$$`-prefixed/garbage: parse error → answers nothing
//       (`greet($a)`, `greet($ABc)`, `greet($_a)`, `foo($$$ABc)`,
//       `$$ABc` all rc=1 empty; `$$ABc` is NOT the universal metavar).
// ---------------------------------------------------------------------------

/// F68a-1 (HIGH): dollar-led LITERAL patterns in `$`-name languages must
/// answer sg's literal hits — the subject silently answered `ok:true`-0 by
/// routing every lowercase-`$`-led pattern to NeverMatches. sg 0.45.2 probed
/// on this exact corpus shape (2026-09-04): each pattern answers ONLY its
/// own line (the `$x` token is literal code, not a metavariable — `const
/// $x = 1` never answers `const y = 1`).
/// Mutant cells: route back to NeverMatches (RED silent-0); treat the
/// lowercase token as a canonical metavariable (`const $x = 1` would
/// answer line 2); treat js mixed-case tokens as non-literal (`$ABc + 2`
/// silent-0).
#[test]
fn f68a_1_dollar_name_language_literal_faces_answer() {
    let js = "const $x = 1;\nconst y = 1;\nlet v = $x + 2;\nlet w = $y + 2;\nlet u = $ABc + 2;\nlet t = $B0x1F + 2;\n";
    for (pattern, want) in [
        ("const $x = 1", vec![1u32]),
        ("$x + 2", vec![3u32]),
        ("$y + 2", vec![4u32]),
        ("$ABc + 2", vec![5u32]),
        ("$B0x1F + 2", vec![6u32]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must stay valid native ingress"
        );
        assert!(
            native_pattern_answerable(Language::JavaScript, pattern),
            "{pattern} must be answerable for js"
        );
        let hits = match_pattern(Language::JavaScript, js, pattern).unwrap();
        assert_eq!(
            lines_of(&hits),
            want,
            "{pattern} must answer sg's literal line set: {hits:?}"
        );
    }
}

/// F68a-1 php half: lowercase-LED variable faces answer literally (the
/// php2/php3 fixture faces, sg-probed), and the MixedCase POISON control
/// (`$ABc` — sg ERROR-node, answers nothing, not even its own line) stays
/// empty — never the canonical-meta wildcard.
/// Mutant cells: route back to NeverMatches (RED silent-0); route php
/// MixedCase tokens to the literal lane (the poison control over-answers).
#[test]
fn f68a_1_php_dollar_literal_faces_answer() {
    let php = "<?php\n$n = strlen($s);\n$x = 1;\n$p4 = $x?->prop;\n$p5 = $x?->prop?->other;\n$p6 = $o?->m1()->m2();\necho $this->get();\n$p7 = $y->a()->b();\n";
    for (pattern, want) in [
        ("$x = 1", vec![3u32]),
        ("strlen($s)", vec![2u32]),
        ("$this->get()", vec![7u32]),
        ("$x?->prop", vec![4u32, 5]),
        ("$o?->m1()", vec![6u32]),
        ("$y->a()->b()", vec![8u32]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must stay valid native ingress"
        );
        let hits = match_pattern(Language::Php, php, pattern).unwrap();
        assert_eq!(
            lines_of(&hits),
            want,
            "{pattern} must answer sg's literal line set: {hits:?}"
        );
    }
    // Poison control (sg: `echo $ABc;` parses to an ERROR-node pattern and
    // answers nothing): line 8 spells `$ABc` verbatim and must NOT answer.
    let poisoned = "<?php\n$ABc = 400;\necho $ABc;\n";
    let hits = match_pattern(Language::Php, poisoned, "$ABc").unwrap();
    assert!(
        hits.is_empty(),
        "php MixedCase $-tokens poison the pattern (sg ERROR node): {hits:?}"
    );
}

/// F26-0182 (P3, grammar-pin correction): sg's meta-name tokenizer rejects
/// a lowercase byte ANYWHERE in the tail — `$ABc`/`$A1b`/`$A_b`/`$_a`/
/// `$$$ABc`/`$$ABc` are NOT canonical metavars. The pass-34/53 pin
/// (`[A-Z_][A-Za-z0-9_]*`) made the subject wildcard through structural
/// lanes where sg answers NOTHING (py/rust: `greet($ABc)` rc=1 empty;
/// `$$ABc` is not the universal metavar) and over-answer js (literal:
/// `foo($ABc)` answers its own line only, never `foo(bar)`).
/// Canonical controls (`$A`, `$A_B`) keep their wildcard rows.
/// Mutant cells: widen the tail back to `[A-Za-z0-9_]` (every empty
/// assertion flips to the wildcard overmatch); reject underscore-led
/// canonical names (`$_`/`$A_B` controls flip).
#[test]
fn f26_0182_meta_name_grammar_rejects_lowercase_anywhere_in_tail() {
    // py/rust: non-canonical `$`-tokens answer nothing (sg rc=1 empty).
    let py = "greet(\"world\")\nt = greet(\"world\")\n";
    for pattern in ["greet($ABc)", "greet($A1b)", "greet($A_b)", "greet($_a)"] {
        let hits = match_pattern(Language::Python, py, pattern).unwrap();
        assert!(
            hits.is_empty(),
            "{pattern} must match nothing (sg: invalid meta name, parse-empty): {hits:?}"
        );
    }
    let rust = "fn main() {\n    foo(1);\n    foo(1, 2);\n}\n";
    let rest = match_pattern(Language::Rust, rust, "foo($$$ABc)").unwrap();
    assert!(
        rest.is_empty(),
        "$$$ABc is not a canonical rest metavar (sg rc=1 empty): {rest:?}"
    );
    let universal = match_pattern(Language::Python, py, "$$ABc").unwrap();
    assert!(
        universal.is_empty(),
        "$$ABc is NOT sg's universal metavar (rc=1 empty, never wildcard): {universal:?}"
    );

    // js: mixed-case tokens are literal code — own line only.
    let js = "foo($ABc);\nfoo(bar);\n";
    let hits = match_pattern(Language::JavaScript, js, "foo($ABc)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1u32],
        "js treats $ABc as a literal identifier: own line only (sg): {hits:?}"
    );

    // Canonical controls keep their wildcard rows (probed sg py {1,2}).
    for pattern in ["greet($A)", "greet($A_B)", "greet($A1)", "greet($_)"] {
        let hits = match_pattern(Language::Python, py, pattern).unwrap();
        assert_eq!(
            lines_of(&hits),
            vec![1u32, 2],
            "{pattern} stays a canonical metavar wildcard (sg): {hits:?}"
        );
    }
}

/// F-68c-1 (MED): the optional-chain matcher must answer literal CALL heads
/// with the plain lane's leaf-text + receiver-walk contract. sg 0.45.2
/// (probed 2026-09-04): `fetch()?.$M($$$A).$M2($$$B)` answers
/// `fetch()?.g().h()` faces (1,2) — never the `other()` head (3), the
/// absorbed-prefix head (4), or the arity mismatch (5) — and the head's own
/// argument template binds (`fetch(1)?...` answers only its arity twin).
/// This is the face pair behind the H-CONF-023 codemod false-alarm: the
/// matcher produced zero edit spans where sg rewrites both files, so the
/// spans must be real (byte-ordered, non-empty).
/// Mutant cells: restore the name-vs-folded-span compare (lines 1/2
/// vanish — the RED state); drop the absorbed-prefix veto (line 4
/// over-answers); drop the head-argument block (line 5 over-answers);
/// drop the leaf compare (line 3 over-answers).
#[test]
fn f68c_1_optional_chain_literal_call_head_answers() {
    let ts = "const a = fetch()?.g().h();\nconst b = fetch()?.g().d();\nconst c = other()?.g().h();\nconst d = x.fetch()?.g().h();\nconst e = fetch(1)?.g().h();\n";
    let pattern = "fetch()?.$M($$$A).$M2($$$B)";
    assert!(
        !needs_ast_grep_fallback(pattern),
        "the literal-call-head optional chain must stay native ingress"
    );
    assert!(native_pattern_answerable(Language::TypeScript, pattern));
    let hits = match_pattern(Language::TypeScript, ts, pattern).unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32, 2],
        "the literal call head must answer sg's two faces only: {hits:?}"
    );
    let first = hits.first().expect("line 1 row");
    assert_eq!(first.captures.get("M").map(String::as_str), Some("g"));
    assert_eq!(first.captures.get("M2").map(String::as_str), Some("h"));
    // Codemod dry-run face: the planner consumes these spans directly
    // (H-CONF-023 fires only when the matcher produces zero of them).
    assert!(
        hits.iter()
            .all(|h| h.byte_start < h.byte_end && h.byte_end > 0),
        "edit spans must be real, non-empty byte ranges: {hits:?}"
    );

    // The head's own argument template is checked through the receiver walk
    // (sg 0.45.2: the meta-arity head answers ONLY the arity twin).
    let meta_arity = "fetch($A)?.$M1($$$B).$M2($$$C)";
    let arity = match_pattern(Language::TypeScript, ts, meta_arity).unwrap();
    assert_eq!(
        klass_lines(&arity),
        vec![5u32],
        "the head arg template binds token-exactly (sg answers only the arity twin): {arity:?}"
    );
    assert_eq!(
        arity.first().unwrap().captures.get("A").map(String::as_str),
        Some("1"),
        "the head's argument capture binds the head call's list"
    );
}

/// F70a-1a (HIGH, pass 71a): the SEARCH lane answers through the index-served
/// short-circuit whenever `index_can_serve_pattern` says a pattern's
/// `pattern_nodes` rows are exact. A member-call CHAIN is never exact there:
/// the index stores (callee path, kind) pairs that cannot express the head's
/// argument template, the segment count, or the connector flags the native
/// matcher enforces — so `fetch()?.$M($$$A)` served `call:fetch` rows and the
/// search lane answered 8 of 9 opchain2 lines (every bare `fetch` call) where
/// sg answers {1,5,6,7,9} and the codemod lane — which calls `match_pattern`
/// directly — planned sg-exact edits on the SAME pattern (the intra-subject
/// search-vs-codemod divergence). The invariant: the index-serve gate must
/// refuse every chain pattern, so the search answer always comes from the
/// same native matcher the codemod lane consumes.
/// Mutant cells: restore the `call:` serving for chains (the gate assertions
/// flip RED); drop the single-call controls (the exact `call:`/`call-name:`
/// cells would silently stop being index-complete).
#[test]
fn f70a_1a_chain_patterns_never_short_circuit_to_the_index() {
    use ast_sgrep_lang::{cached_pattern_signatures, index_can_serve_pattern};
    // The exact finding faces (opchain2 / plainchain / opchain_js patterns).
    for pattern in [
        "fetch()?.$M($$$A)",
        "fetch()?.$M($$$A).$M2($$$B)",
        "fetch().$M($$$A).$M2($$$B)",
        "x.fetch()?.$M($$$A)",
        "$O?.$M($$$A).$M2($$$B)",
    ] {
        let served = cached_pattern_signatures(pattern)
            .map(|sigs| index_can_serve_pattern(pattern, &sigs))
            .unwrap_or(false);
        assert!(
            !served,
            "{pattern} must never be index-served: the search lane would answer from \
             (callee, kind) rows and diverge from the codemod lane's match_pattern answers"
        );
    }
    // Controls: the registered EXACT single-call cells keep their
    // index-complete rows (any-arity rest list = every call of the callee).
    for pattern in ["fetch($$$A)", "greet($$$ARGS)", "conn.open($$$A)"] {
        let sigs = cached_pattern_signatures(pattern).expect("single-call rest shape");
        assert!(
            index_can_serve_pattern(pattern, &sigs),
            "{pattern} is a single call with a pure rest list — its index rows are exact: {sigs:?}"
        );
    }
    // And the matcher itself (the lane both engines must agree through)
    // answers sg's opchain2 line sets on the finding corpus.
    let ts = "const q1 = fetch()?.g();\nconst q2 = fetch(1)?.g();\nconst q3 = fetch(\"k\")?.g();\nconst q4 = fetch(a, b)?.g();\nconst q5 = fetch()?.g(1).h();\nconst q6 = fetch()?.g()?.h();\nconst q7 = fetch()?.g().h()?.i();\nconst q8 = x.fetch()?.g();\nconst q9 = fetch()?.g();\n";
    let two_seg = match_pattern(Language::TypeScript, ts, "fetch()?.$M($$$A)").unwrap();
    assert_eq!(
        klass_lines(&two_seg),
        vec![1u32, 5, 6, 7, 9],
        "2-seg literal-head chain: sg line set (head args veto q2/q3/q4): {two_seg:?}"
    );
    let three_seg = match_pattern(Language::TypeScript, ts, "fetch()?.$M($$$A).$M2($$$B)").unwrap();
    assert_eq!(
        klass_lines(&three_seg),
        vec![5u32, 7],
        "3-seg literal-head chain: sg line set (segment count + mid-connector flags veto the rest): {three_seg:?}"
    );
}

/// F70a-1b (MED, pass 71a): the 3-seg optional meta-head chain must bind
/// NON-EMPTY mid-segment rest args. `fetch()?.g(1).h()` was silently missed
/// by `$O?.$M($$$A).$M2($$$B)` (sg answers {5,7} with A=[1]): the legacy
/// pattern-level argument capture extracted the FIRST call segment's
/// `$$$A` and bound it against the OUTER node's argument list (the LAST
/// segment's), colliding with the receiver walk's correct A=[1] binding and
/// vetoing the match. Codemod under-planned the same face (1 edit vs sg's 2).
/// Mutant cells: restore the pattern-level capture_arguments call (line 5
/// vanishes — the RED state); bind A in the single namespace (the
/// namespace assertion flips).
#[test]
fn f70a_1b_optional_chain_mid_segment_rest_args_bind() {
    let ts = "const q1 = fetch()?.g();\nconst q5 = fetch()?.g(1).h();\nconst q7 = fetch()?.g().h()?.i();\n";
    let pattern = "$O?.$M($$$A).$M2($$$B)";
    assert!(
        !needs_ast_grep_fallback(pattern),
        "the meta-head optional chain must stay native ingress"
    );
    let hits = match_pattern(Language::TypeScript, ts, pattern).unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![2u32, 3],
        "sg answers the non-empty mid-args face (line 2) AND the empty one (line 3): {hits:?}"
    );
    let mid = hits
        .iter()
        .find(|h| h.line_start == 2)
        .expect("line 2 row must answer");
    assert_eq!(mid.captures.get("O").map(String::as_str), Some("fetch()"));
    assert_eq!(mid.captures.get("M").map(String::as_str), Some("g"));
    assert_eq!(mid.captures.get("M2").map(String::as_str), Some("h"));
    assert_eq!(
        mid.captures.get("$$$A").map(String::as_str),
        Some("1"),
        "the mid segment's rest args bind in the MULTI namespace (H-CONF-026)"
    );
    assert_eq!(
        mid.captures.get("$$$B").map(String::as_str),
        Some(""),
        "the last segment's empty rest list binds empty"
    );
    // 2-seg control (registered AGREE face): every optional chain answers.
    let two = match_pattern(Language::TypeScript, ts, "$O?.$M($$$A)").unwrap();
    assert_eq!(
        klass_lines(&two),
        vec![1u32, 2, 3],
        "the 2-seg meta-head lane is unchanged: {two:?}"
    );
}

/// 70c-F1 (MED, destructive surface, pass 71a): sg 0.45.2 REJECTS
/// `$$$`-led LOWERCASE patterns in php (rc=1 — probed 2026-09-06 with the
/// literal face in the fixture, so rc=1 is refusal, not no-match), but the
/// 3-dollar scan classified the name by the 1-dollar table and the php arm
/// of the literal lane admitted `$$$u + 2` — the codemod dry-run PLANNED an
/// edit where sg refuses the pattern. The corrected table: `$$$lowercase` is
/// its own class, php-literal only for 1- and 2-dollar runs.
/// Mutant cells: classify `$$$u` by the 1-dollar table again (the face
/// over-answers — RED); refuse 2-dollar php faces (the `$$tot` control
/// vanishes).
#[test]
fn f70c_f1_php_three_dollar_lowercase_refused_on_the_destructive_surface() {
    let php = "<?php\n$t = $$$u + 2;\n$w = $$$x + 1;\n";
    for pattern in ["$$$u + 2", "$$$x + 1"] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} stays admitted ingress (classify → NeverMatches), so the planner \
             sees the pattern and plans zero edits instead of bailing"
        );
        let hits = match_pattern(Language::Php, php, pattern).unwrap();
        assert!(
            hits.is_empty(),
            "{pattern}: sg refuses the pattern (rc=1); the subject must answer nothing — \
             a planned edit here is a silent-wrong rewrite: {hits:?}"
        );
    }
}

/// 70c-F3 (pass 71a): the corrected `$$`/`$$$` cell table (sg 0.45.2 probed
/// 2026-09-06, faces present in every fixture). `$$`-led NON-canonical tokens
/// are LITERAL CODE where sg answers: js/ts identifiers (`$$x`, `$$ABc`,
/// `$$_x`, `$$$x`, `$$$ABc` — own line only) and php variable-variables
/// (`$$tot`, `$$x` — lowercase-led 2-dollar only). Everywhere sg refuses the
/// pattern the subject answers nothing: php `$$ABc` / `$$$x` / `$$$u`,
/// python and rust `$$`/`$$$` of any non-canonical name (py rc=8, rust
/// ERROR-node). Canonical `$$A`/`$$_` keep the registered universal lane.
/// Mutant cells: skip 2-dollar tokens in the class scan (the js/php answer
/// assertions flip RED); admit 3-dollar php faces (the refusal cells
/// over-answer); let `$$A` enter the literal lane (universal control fails).
#[test]
fn f70c_f3_two_and_three_dollar_literal_cells_match_sg() {
    // sg ANSWERS these literally (own line only).
    let js = "const r1 = $$x + 2;\nconst r2 = $$ABc + 2;\nconst r3 = $$_x + 2;\nconst r4 = $$$x + 2;\nconst r5 = $$$ABc + 2;\n";
    for (pattern, want) in [
        ("$$x + 2", vec![1u32]),
        ("$$ABc + 2", vec![2u32]),
        ("$$_x + 2", vec![3u32]),
        ("$$$x + 2", vec![4u32]),
        ("$$$ABc + 2", vec![5u32]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must stay admitted ingress (sg answers it in js)"
        );
        assert!(native_pattern_answerable(Language::JavaScript, pattern));
        let hits = match_pattern(Language::JavaScript, js, pattern).unwrap();
        assert_eq!(
            lines_of(&hits),
            want,
            "{pattern}: sg answers the literal face, own line only: {hits:?}"
        );
    }
    let php = "<?php\n$t = $$tot + 1;\n$u = $$x + 2;\n";
    for (pattern, want) in [("$$tot + 1", vec![2u32]), ("$$x + 2", vec![3u32])] {
        assert!(!needs_ast_grep_fallback(pattern));
        let hits = match_pattern(Language::Php, php, pattern).unwrap();
        assert_eq!(
            lines_of(&hits),
            want,
            "{pattern}: php variable-variables answer literally (sg, face present): {hits:?}"
        );
    }
    // sg REFUSES these — the subject answers nothing (NeverMatches class).
    let poisoned = "<?php\n$ab = $$ABc + 2;\n$c = $$$x + 1;\n$d = $$$u + 1;\n";
    for pattern in ["$$ABc + 2", "$$$x + 1", "$$$u + 1"] {
        let hits = match_pattern(Language::Php, poisoned, pattern).unwrap();
        assert!(
            hits.is_empty(),
            "{pattern}: sg refuses in php (rc=1) — must stay empty: {hits:?}"
        );
    }
    let py = "r1 = $$x + 2\nr2 = $$ABc + 2\nr3 = $$$x + 2\n";
    for pattern in ["$$x + 2", "$$ABc + 2", "$$$x + 2"] {
        let hits = match_pattern(Language::Python, py, pattern).unwrap();
        assert!(
            hits.is_empty(),
            "{pattern}: sg refuses in python (rc=8) — must stay empty: {hits:?}"
        );
    }
    let rust = "fn main() {\n    let r1 = $$x + 2;\n    let r2 = $$$x + 2;\n}\n";
    for pattern in ["$$x + 2", "$$$x + 2"] {
        let hits = match_pattern(Language::Rust, rust, pattern).unwrap();
        assert!(
            hits.is_empty(),
            "{pattern}: sg parses to an ERROR-node pattern in rust — must stay empty: {hits:?}"
        );
    }
    // Canonical controls: the registered universal lane is untouched.
    assert!(!needs_ast_grep_fallback("$$A"));
    let universal_src = "x = 1\n";
    let universal = match_pattern(Language::Python, universal_src, "$$A").unwrap();
    assert!(
        !universal.is_empty(),
        "$$A must keep its registered universal lane"
    );
}

// ---------------------------------------------------------------------------
// PASS 73 (r23 remediation): F72a-1, F72a-2, F-72c-1.
// sg = ast-grep 0.45.2; every line set and capture below was probed live
// (2026-09-06, pass-73 scratch corpora p73php/p73ts) BEFORE the fix — the
// RED state of tests 1–2 against the r22 binary is the finding evidence.
// ---------------------------------------------------------------------------

/// F72a-1 (HIGH, pass 72a): php static `::` calls split the callee across the
/// `scope` + `name` fields of `scoped_call_expression`; the native matcher
/// resolved only the `name` field, so meta-arg patterns (`Foo::bar($A)`,
/// `Foo::bar($$$A)`, `$A::bar($B)`, `Foo::$M($A)`, `self::bar($A)`) answered
/// silent `ok:true []` where sg answers, AND the mirror face over-matched:
/// `bar($$$A)` index/native-matched the `Foo::bar(1)` lines (sg answers []).
/// sg 0.45.2 probes (p73php): scope/name match positionally, meta scope binds
/// the RAW scope text (`Foo`, `self`, `$inst`), meta name binds the RAW name
/// text (`bar`, `$dyn` with the dollar), and the lone-metavar `$F($$$A)` veto
/// holds (only the PLAIN `helper(...)` call answers).
/// Mutant cells: drop the `scoped_call_expression` arm in `call_callee` /
/// `call_target_path_faithful` (every set assertion flips RED — the finding
/// state); drop the `call_target` raw-callee arm (the `bar($$$A)` over-match
/// assertion flips); drop the `capture_call_path` `::` normalization (the
/// `$A::bar($B)` scope-capture assertion flips).
#[test]
fn f72a_1_php_static_call_meta_args_answer_sg_exact() {
    let php = "<?php\nFoo::bar(1);\nFoo::bar(1, 2);\nFoo::$dyn(3);\nself::bar(4);\n$inst::bar(5);\nFoo::bar($w);\n$inst::other($w);\nFoo::nested(Foo::bar(9));\nhelper(Foo::bar(10));\n";
    // (pattern, sg line set)
    for (pattern, want) in [
        ("Foo::bar($A)", vec![2u32, 7, 9, 10]),
        ("Foo::bar($$$A)", vec![2u32, 3, 7, 9, 10]),
        ("$A::bar($B)", vec![2u32, 5, 6, 7, 9, 10]),
        ("Foo::$M($A)", vec![2u32, 4, 7, 9, 10]),
        ("self::bar($A)", vec![5u32]),
        ("Foo::bar($A, $B)", vec![3u32]),
    ] {
        assert!(!needs_ast_grep_fallback(pattern), "{pattern} classifies");
        let hits = match_pattern(Language::Php, php, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the p73php faces (F72a-1): {hits:?}"
        );
    }
    // Captures: meta scope binds the RAW scope text; meta name binds the RAW
    // name text (incl. the `$` of a dynamic name); args bind as usual.
    let scope_hits = match_pattern(Language::Php, php, "$A::bar($B)").unwrap();
    let inst = scope_hits
        .iter()
        .find(|h| h.line_start == 6)
        .expect("line 6 ($inst::bar(5)) must answer");
    assert_eq!(inst.captures.get("A").map(String::as_str), Some("$inst"));
    assert_eq!(inst.captures.get("B").map(String::as_str), Some("5"));
    let self_hit = scope_hits
        .iter()
        .find(|h| h.line_start == 5)
        .expect("line 5 (self::bar(4)) must answer");
    assert_eq!(self_hit.captures.get("A").map(String::as_str), Some("self"));
    let dyn_hits = match_pattern(Language::Php, php, "Foo::$M($A)").unwrap();
    let dyn_hit = dyn_hits
        .iter()
        .find(|h| h.line_start == 4)
        .expect("line 4 (Foo::$dyn(3)) must answer");
    assert_eq!(dyn_hit.captures.get("M").map(String::as_str), Some("$dyn"));
    let rest_hits = match_pattern(Language::Php, php, "Foo::bar($$$A)").unwrap();
    let multi = rest_hits
        .iter()
        .find(|h| h.line_start == 3)
        .expect("line 3 (Foo::bar(1, 2)) must answer");
    assert_eq!(
        multi.captures.get("$$$A").map(String::as_str),
        Some("1, 2"),
        "rest args bind in the MULTI namespace (H-CONF-026)"
    );
    // Controls: literal / empty-args `::` faces keep the literal lane; a
    // literal scope does not cross dynamic names; the lone-metavar veto
    // holds for split-field callees (only the PLAIN helper call answers);
    // and the trailing-name over-match face is gone (sg answers []).
    let lit = match_pattern(Language::Php, php, "Foo::bar(1)").unwrap();
    assert_eq!(
        klass_lines(&lit),
        vec![2u32],
        "literal lane control: {lit:?}"
    );
    // Empty-args `::` face (the idxgate_php line-3 shape, sg {3}): the
    // literal lane keeps answering it after the fix.
    let empty_php = "<?php\nFoo::bar(1);\nFoo::bar();\n";
    let empty = match_pattern(Language::Php, empty_php, "Foo::bar()").unwrap();
    assert_eq!(
        klass_lines(&empty),
        vec![3u32],
        "empty-args control: {empty:?}"
    );
    let lone = match_pattern(Language::Php, php, "$F($$$A)").unwrap();
    assert_eq!(
        klass_lines(&lone),
        vec![10u32],
        "$F($$$A): sg answers only the plain helper(Foo::bar(10)) call: {lone:?}"
    );
    let trailing = match_pattern(Language::Php, php, "bar($$$A)").unwrap();
    assert!(
        trailing.is_empty(),
        "bar($$$A): sg answers [] on a static-call-only corpus — the old \
         trailing-name resolution over-matched here: {trailing:?}"
    );
    // Index rows: static-call `call:` rows key on the exact source callee
    // bytes (`Foo::bar`) — never the bare trailing name (`call:bar` was the
    // silent over-match row, hit by every `bar(...)` pattern query).
    use ast_sgrep_lang::ParserRegistry;
    let rows = ParserRegistry::new()
        .parse(Language::Php, php)
        .unwrap()
        .pattern_nodes;
    let call_sigs: Vec<&str> = rows
        .iter()
        .map(|n| n.signature.as_str())
        .filter(|s| s.starts_with("call:") || s.starts_with("call-name:"))
        .collect();
    assert!(
        call_sigs.contains(&"call:Foo::bar"),
        "static-call rows must key on the raw callee bytes: {call_sigs:?}"
    );
    assert!(
        !call_sigs.contains(&"call:bar"),
        "the bare `call:bar` row over-matched Foo::bar lines for bar(...) patterns: {call_sigs:?}"
    );
}

/// F72a-2 (MED, pass 72a), fixed half: nested-call-in-args patterns whose
/// argument lists carry `$$$` rests. The general lane refused EVERY pattern
/// containing `$$$` (pass-51 blanket), so `g(fetch($$$A))` and friends died
/// loud where sg 0.45.2 answers (probed on p73ts). The family gate admits
/// `$$$` ONLY as the SOLE argument of its call, only alongside metavars and
/// nested calls — literal atoms keep the registered H-CONF-002/H-CONF-013
/// mixed-concrete loud class.
/// Mutant cells: drop the sole-rest arm in `general_eq` (every `$$$` nested
/// face goes empty — RED); widen the family gate to literal atoms (the loud
/// controls flip to admitted — RED).
#[test]
fn f72a_2_nested_call_rest_args_answer_sg_exact() {
    let ts = "fetch(\"u\");\ng(fetch(a));\ng(fetch(a), 2);\ng(1, fetch(a));\nfetch(fetch(a));\nfetch(g(fetch(a)));\ng(o.fetch(a));\ng(o.p.fetch(a));\nh(fetch(a), send(b));\nh($v, fetch(a));\ng(fetch(a, b), send(c));\nouter(fetch(g(fetch(d))));\nwrap.x(fetch(a), 2);\n";
    for (pattern, want) in [
        ("g(fetch($$$A))", vec![2u32, 6, 12]),
        ("fetch(fetch($$$A))", vec![5u32]),
        ("g(fetch($$$A), send($$$B))", vec![11u32]),
        ("g(o.fetch($$$A))", vec![7u32]),
        ("fetch(g(fetch($$$A)))", vec![6u32, 12]),
        ("outer(fetch(g(fetch($$$A))))", vec![12u32]),
        ("g(fetch($$$A), $B)", vec![3u32, 11]),
        ("g($A, fetch($$$B))", vec![4u32]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must be admitted: sg answers the nested-rest face"
        );
        assert!(native_pattern_answerable(Language::TypeScript, pattern));
        let hits = match_pattern(Language::TypeScript, ts, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the p73ts faces (F72a-2): {hits:?}"
        );
    }
    // Captures: the inner rest binds the multi namespace; the sibling single
    // metavariable binds any expression node, nested calls included.
    let hits = match_pattern(Language::TypeScript, ts, "g(fetch($$$A), $B)").unwrap();
    let eleven = hits
        .iter()
        .find(|h| h.line_start == 11)
        .expect("line 11 must answer");
    assert_eq!(
        eleven.captures.get("$$$A").map(String::as_str),
        Some("a, b")
    );
    assert_eq!(
        eleven.captures.get("B").map(String::as_str),
        Some("send(c)")
    );
    let three = hits
        .iter()
        .find(|h| h.line_start == 3)
        .expect("line 3 must answer");
    assert_eq!(three.captures.get("$$$A").map(String::as_str), Some("a"));
    assert_eq!(three.captures.get("B").map(String::as_str), Some("2"));
    // Registered LOUD cells (never patched toward the oracle): a literal
    // atom in any argument list keeps the fail-closed ingress.
    for pattern in [
        "g(fetch($$$A), 2)",
        "wrap.x(fetch($$$A), 2)",
        "fetch($$$A, 2)",
        "add(1, $$$A)",
    ] {
        assert!(
            needs_ast_grep_fallback(pattern),
            "{pattern} must stay loud (registered mixed-concrete class)"
        );
    }
    // The no-rest nested twin keeps its pre-existing general-lane answer.
    let twin = match_pattern(Language::TypeScript, ts, "g(fetch($A))").unwrap();
    assert_eq!(
        klass_lines(&twin),
        vec![2u32, 6, 12],
        "twin control: {twin:?}"
    );
}

/// F-72c-1 (LOW, pass 72c): the index-serve gate (`has_multiple_call_segments`)
/// and the chain classifier (`classify_call_chain`) must keep byte-identical
/// depth-0 paren/subscript scanning. Quote/comment paren-skew shapes are loud
/// today through that LOCKSTEP plus the segment/args validation locks; this
/// test pins the loud posture of the documented skew class so a future edit to
/// EITHER scanner (or a new multi-call classify shape) must re-prove it.
/// The CNR §20.1 rider records the coupling.
/// Mutant cell (documented skew direction): make `has_multiple_call_segments`'
/// scanner quote/comment-aware INDEPENDENTLY of `classify_call_chain` — the
/// signatures-layer assertion below flips on the under-counted face (RED),
/// proving the pin watches the gate's scan, not just the ingress posture.
#[test]
fn f72c_1_quote_comment_paren_skew_stays_loud() {
    // The two 72c-named examples + the unbalanced-comment under-count face.
    for pattern in [
        "foo(\"(\").bar($$$X)",
        "foo(/* ( */ $$$A).bar($$$B)",
        "f($$$A /* ().g($$$B)",
        "f(\"(\").g($$$X)",
        "a($$$X) /* ( */.b($$$Y)",
    ] {
        assert!(
            needs_ast_grep_fallback(pattern),
            "{pattern} must stay loud: paren-skew shapes have no native contract"
        );
        // The gate must refuse a signature for every skew shape — an exact
        // (callee, kind) row can never be complete for a pattern whose
        // segment count the classifier cannot even trust.
        assert!(
            ast_sgrep_lang::cached_pattern_signatures(pattern).is_none(),
            "{pattern} must never be index-servable"
        );
        // And the native answer is empty, never silent-wrong.
        let hits = match_pattern(Language::TypeScript, "g(1);\n", pattern).unwrap();
        assert!(hits.is_empty(), "{pattern} must answer nothing: {hits:?}");
    }
    // Balanced-quote control: a well-formed chain with a plain string
    // argument still refuses (args purity), and a quote-free chain admits —
    // the skew class is about PAREN SKEW, not strings per se.
    assert!(needs_ast_grep_fallback("fetch(\"u\").g($$$X)"));
    assert!(!needs_ast_grep_fallback("fetch($$$A).g($$$B)"));
    // Gate-side cell of the lockstep: a chain the classifier ADMITS must
    // still be gate-refused at the signatures layer (the under-count
    // direction of the drift — the gate seeing fewer segments than the
    // classifier — is the HIGH over-match class 72c named).
    assert!(
        ast_sgrep_lang::cached_pattern_signatures("fetch($$$A).g($$$B)").is_none(),
        "an admitted 2-call chain must never be index-servable"
    );
}

// ---------------------------------------------------------------------------
// PASS 75a (r25 remediation): F74a-1, F74a-2, F74a-3, F74a-4, 74c-F3, 74c-F4,
// 74c-F2 pins. sg = ast-grep 0.45.2; every line set and capture below was
// probed live (2026-09-06, /tmp/p75a fixtures byte-identical to the inline
// sources) BEFORE the fix — the RED state of tests 1–5 against the r24 binary
// (sha16 beaeaf8248e285bc) is the finding evidence. Sources mirror the pass-74a
// fixtures (`php_matrix/a.php`) and the pass-74c scratch corpora so the
// adjudicated sg line sets transfer 1:1.
// ---------------------------------------------------------------------------

/// F74a-1 (HIGH, pass 74a): php `::` DYNAMIC-receiver calls with meta args
/// answered silent `ok:true []`. Root cause: a lowercase-led `$name` token
/// (`$obj`, `$dyn`) mixed with canonical metas tripped the pass-54
/// non-canonical gate into `NativeKind::NeverMatches` (silent empty). sg
/// 0.45.2 treats such tokens as LITERAL php variable text: `$obj::bar($A)`
/// answers only the `$obj` scope line (`$anything::bar($A)` answers []), and
/// `Foo::$dyn($A)` answers only the literal `$dyn` name line
/// (`Foo::$other($A)` answers []).
/// Mutant cells: drop the `literal_variable_callee_admitted` carve (every set
/// assertion here flips RED — the silent-miss state); drop the
/// LowercaseLed arm of `call_path_segment` (same faces flip).
#[test]
fn f74a_1_php_dynamic_scope_calls_answer_sg_exact() {
    let php = "<?php\nFoo::bar($u);\n\\Foo::bar($u);\nself::bar($u);\nstatic::bar($u);\nparent::bar($u);\n$obj::bar($u);\nApp\\Models\\User::find($u);\nFoo::bar();\nFoo::bar(1);\nFoo::bar($u, $v);\nFoo::bar($u)->baz();\nFoo::$dyn($u);\nFoo::bar(\n    $u\n);\nFoo::bar($v);\n$svc->run($u);\necho strtoupper($w);\n";
    for (pattern, want) in [
        // The finding faces: literal variable scope / name text + meta arg.
        ("$obj::bar($A)", vec![7u32]),
        ("$obj::bar($B)", vec![7u32]),
        ("Foo::$dyn($A)", vec![13u32]),
        // Literal-text vetoes (sg answers [] — rc=1): the scope/name match is
        // whole-node text, never a wildcard.
        ("$anything::bar($A)", vec![]),
        ("Foo::$other($A)", vec![]),
        ("obj::$M($A)", vec![]),
        // Pass-73 guards re-pinned on this corpus (zero unflip).
        ("$A::bar($B)", vec![2u32, 3, 4, 5, 6, 7, 10, 12, 14, 17]),
        ("Foo::$M($A)", vec![2u32, 10, 12, 13, 14, 17]),
        ("Foo::bar($A, $B)", vec![11u32]),
        ("$F($$$A)", vec![19u32]),
        ("bar($$$A)", vec![]),
        ("User::find($A)", vec![]),
    ] {
        assert!(!needs_ast_grep_fallback(pattern), "{pattern} classifies");
        let hits = match_pattern(Language::Php, php, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on php_matrix (F74a-1): {hits:?}"
        );
    }
    // Capture: the meta arg binds sg's way (single namespace, `A`).
    let hits = match_pattern(Language::Php, php, "$obj::bar($A)").unwrap();
    let hit = hits.iter().find(|h| h.line_start == 7).expect("line 7");
    assert_eq!(hit.captures.get("A").map(String::as_str), Some("$u"));
}

/// F74a-2 (HIGH, pass 74a): php `->` member calls with meta args answered
/// silent `ok:true []` (and codemod planned 0 edits). Same gate shape as
/// F74a-1 (`$svc` tripped NeverMatches) plus a missing member-call callee
/// arm: `call_callee` keyed only `scoped_call_expression`. sg 0.45.2 answers
/// `$svc->run($A)` on the object->name chain, keeps the nullsafe `?->`
/// connector token-exact (`$o->m($A)` answers [] where only `?->` exists),
/// and never answers trailing-name patterns (`run($A)` → []).
/// Mutant cells: drop the `member_call_expression` arm in `call_callee` /
/// `call_target_path_faithful` (the `$svc` faces and the row pins flip RED);
/// drop the LowercaseLed head arm of `parse_optional_call_path` (the `$o?->m`
/// face flips RED).
#[test]
fn f74a_2_php_member_call_meta_args_answer_sg_exact() {
    let php = "<?php\nFoo::bar($u);\n\\Foo::bar($u);\nself::bar($u);\nstatic::bar($u);\nparent::bar($u);\n$obj::bar($u);\nApp\\Models\\User::find($u);\nFoo::bar();\nFoo::bar(1);\nFoo::bar($u, $v);\nFoo::bar($u)->baz();\nFoo::$dyn($u);\nFoo::bar(\n    $u\n);\nFoo::bar($v);\n$svc->run($u);\necho strtoupper($w);\n";
    for (pattern, want) in [
        ("$svc->run($A)", vec![18u32]),
        ("$svc->run($$$A)", vec![18u32]),
        // The lone-metavar synthetic veto: `$F($$$A)` stays on the PLAIN call.
        ("$F($$$A)", vec![19u32]),
    ] {
        assert!(!needs_ast_grep_fallback(pattern), "{pattern} classifies");
        let hits = match_pattern(Language::Php, php, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on php_matrix (F74a-2): {hits:?}"
        );
    }
    let hits = match_pattern(Language::Php, php, "$svc->run($A)").unwrap();
    let hit = hits.iter().find(|h| h.line_start == 18).expect("line 18");
    assert_eq!(hit.captures.get("A").map(String::as_str), Some("$u"));
    // Chain + nullsafe + trailing-name faces (php2 corpus, sg-probed).
    let php2 = "<?php\n$x::$x(1);\nFoo::bar(1);\n$o?->m($u);\n$a->b->c($u);\n$y->a()->b();\nFoo\\Bar::baz(1);\n\\Foo::bar(2);\nobj::$dyn(3);\n";
    for (pattern, want) in [
        ("$a->b->c($A)", vec![5u32]),
        ("$a->b->d($A)", vec![]),
        ("$o?->m($A)", vec![4u32]),
        ("$o->m($A)", vec![]),
        ("run($A)", vec![]),
        ("run($$$A)", vec![]),
        ("m($A)", vec![]),
        ("c($A)", vec![]),
    ] {
        let hits = match_pattern(Language::Php, php2, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on php2 (F74a-2): {hits:?}"
        );
    }
    // Index rows: php member calls key the reassembled `object.name` callee
    // (sg's own callee spelling), never the bare trailing name — the pass-22
    // `call:run` row was the silent over-match key for run(...) patterns.
    use ast_sgrep_lang::ParserRegistry;
    let rows = ParserRegistry::new()
        .parse(Language::Php, php)
        .unwrap()
        .pattern_nodes;
    let call_sigs: Vec<&str> = rows
        .iter()
        .map(|n| n.signature.as_str())
        .filter(|s| s.starts_with("call:") || s.starts_with("call-name:"))
        .collect();
    assert!(
        call_sigs.contains(&"call:$svc.run"),
        "member-call rows must key on the object.name callee: {call_sigs:?}"
    );
    assert!(
        !call_sigs.contains(&"call:run"),
        "the bare `call:run` row over-matched $svc->run lines for run(...) patterns: {call_sigs:?}"
    );
}

/// F74a-3 (MEDIUM, pass 74a): canonical 2-dollar `$$A` in argument slots
/// refused EXIT2 where sg 0.45.2 answers. sg probes (2026-09-06): `$$A` in
/// an argument slot behaves EXACTLY like `$A` — same line sets, same capture
/// key `A`, siblings and literals included (`g($$A, $B)` ≡ `g($A, $B)`,
/// `g($$A, 2)` answers). The pass-73 `$$$`-only family predicate narrows to
/// its registered core: a canonical `$$NAME` joins the sole-rest family as a
/// SINGLE-capture member, substitution, and capture seams.
/// Mutant cells: drop the `$$` arm of `is_pure_metavariable` (every flat face
/// flips RED); drop the `$$` arm of `capture_name` (the capture pin flips);
/// drop the `dollars == 2` arm of `substitute_general_metavariables` and the
/// 2-dollar arm of `parse_family_call` (the nested faces flip).
#[test]
fn f74a_3_two_dollar_args_answer_sg_exact() {
    let ts = "fetch(\"u\");\ng(fetch(a));\ng(fetch(a), 2);\ng(1, fetch(a));\nfetch(fetch(a));\nfetch(g(fetch(a)));\ng(o.fetch(a));\ng(o.p.fetch(a));\nh(fetch(a), send(b));\nh($v, fetch(a));\ng(fetch(a, b), send(c));\nouter(fetch(g(fetch(d))));\nwrap.x(fetch(a), 2);\ng(send(a));\nx.y(1);\nh(i(j(1)));\nfetch(a).g(b);\n";
    for (pattern, want) in [
        ("g($$A)", vec![2u32, 6, 7, 8, 12, 14]),
        ("fetch($$A)", vec![1u32, 2, 3, 4, 5, 6, 9, 10, 12, 13, 17]),
        ("g(fetch($$A))", vec![2u32, 6, 12]),
        ("fetch(fetch($$A))", vec![5u32]),
        // sg answers [] here — the pin is the no-over-match direction.
        ("h(g(fetch($$A)))", vec![]),
        ("obj.wrap(fetch($$A))", vec![]),
        ("g(send($$A))", vec![14u32]),
        ("x.y($$A)", vec![15u32]),
        ("h(i(j($$A)))", vec![16u32]),
        ("fetch($$A).g($$B)", vec![17u32]),
        // Sibling rows of the boundary table: `$$A` ≡ `$A` with siblings.
        ("g($$A, $B)", vec![3u32, 4, 11]),
        ("g(fetch($$A), $B)", vec![3u32]),
        ("g($$A, 2)", vec![3u32]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must be admitted: sg answers the 2-dollar face"
        );
        let hits = match_pattern(Language::TypeScript, ts, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the p75ts faces (F74a-3): {hits:?}"
        );
    }
    // Capture: `$$A` binds under `A` (sg capture key), single namespace.
    let hits = match_pattern(Language::TypeScript, ts, "g($$A)").unwrap();
    let two = hits.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(two.captures.get("A").map(String::as_str), Some("fetch(a)"));
    // php face: `Foo::bar($$A)` answers exactly the single-arg static sites;
    // the same-name unify veto holds (`$A::bar($$A)` → [] like sg) and the
    // removed `bar($$A)` over-match stays gone.
    let php = "<?php\nFoo::bar($u);\n\\Foo::bar($u);\nself::bar($u);\nstatic::bar($u);\nparent::bar($u);\n$obj::bar($u);\nApp\\Models\\User::find($u);\nFoo::bar();\nFoo::bar(1);\nFoo::bar($u, $v);\nFoo::bar($u)->baz();\nFoo::$dyn($u);\nFoo::bar(\n    $u\n);\nFoo::bar($v);\n$svc->run($u);\necho strtoupper($w);\n";
    for (pattern, want) in [
        ("Foo::bar($$A)", vec![2u32, 10, 12, 14, 17]),
        ("$A::bar($$A)", vec![]),
        ("bar($$A)", vec![]),
    ] {
        let hits = match_pattern(Language::Php, php, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on php_matrix (F74a-3): {hits:?}"
        );
    }
    // Registered loud cells keep their class: bare `$$` (malformed dollar run)
    // stays loud.
    assert!(
        needs_ast_grep_fallback("g($$)"),
        "bare `$$` keeps the registered loud residual"
    );
}

/// F74a-4 (MEDIUM, pass 74a) + 74c-F3 (LOW, pass 74c): php `::` scope-text
/// variants refused EXIT2 where sg answers — backslash-led scopes
/// (`\Foo::bar($A)`), namespace-qualified scopes (`App\Models\User::find`),
/// and `::`-headed chains (`Foo::bar($A)->baz()`). The re-keyed index already
/// emits `call:\Foo::bar` / `call:Foo\Bar::baz` rows (74c direct sqlite
/// dump); the pattern-side gates (`parse_call_path`, `is_pattern_path`) must
/// accept the same spellings so the rows are addressable.
/// Mutant cells: drop the namespace arm of `call_path_segment` (the `\Foo` /
/// `App\Models` faces flip RED); drop the php `<?php ` wrapper retry for
/// `->`-carrying patterns (the chain face flips); drop the
/// `is_pattern_path` extension (the signature pins flip).
#[test]
fn f74a_4_php_scope_text_variants_answer_sg_exact() {
    let php = "<?php\nFoo::bar($u);\n\\Foo::bar($u);\nself::bar($u);\nstatic::bar($u);\nparent::bar($u);\n$obj::bar($u);\nApp\\Models\\User::find($u);\nFoo::bar();\nFoo::bar(1);\nFoo::bar($u, $v);\nFoo::bar($u)->baz();\nFoo::$dyn($u);\nFoo::bar(\n    $u\n);\nFoo::bar($v);\n$svc->run($u);\necho strtoupper($w);\n";
    for (pattern, want) in [
        ("\\Foo::bar($A)", vec![3u32]),
        ("App\\Models\\User::find($A)", vec![8u32]),
        ("Foo::bar($A)->baz()", vec![12u32]),
        // Suffix-scope veto (§74a-7.3): scope matching is whole-node.
        ("User::find($A)", vec![]),
        ("Other\\Models\\User::find($A)", vec![]),
    ] {
        assert!(!needs_ast_grep_fallback(pattern), "{pattern} classifies");
        assert!(native_pattern_answerable(Language::Php, pattern));
        let hits = match_pattern(Language::Php, php, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on php_matrix (F74a-4): {hits:?}"
        );
    }
    let php2 = "<?php\n$x::$x(1);\nFoo::bar(1);\n$o?->m($u);\n$a->b->c($u);\n$y->a()->b();\nFoo\\Bar::baz(1);\n\\Foo::bar(2);\nobj::$dyn(3);\n";
    for (pattern, want) in [
        ("Foo\\Bar::baz($A)", vec![7u32]),
        ("\\Foo::bar($$$A)", vec![8u32]),
    ] {
        let hits = match_pattern(Language::Php, php2, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on php2 (F74a-4): {hits:?}"
        );
    }
    // 74c-F3: the pattern-side signature derivation must address the re-keyed
    // namespace rows (row emission was verified by 74c's sqlite dump).
    assert_eq!(
        ast_sgrep_lang::cached_pattern_signatures("\\Foo::bar($$$A)"),
        Some(vec!["call:\\Foo::bar".to_string()]),
        "backslash-qualified callees must derive their exact call: signature"
    );
    assert_eq!(
        ast_sgrep_lang::cached_pattern_signatures("Foo\\Bar::baz($$$A)"),
        Some(vec!["call:Foo\\Bar::baz".to_string()]),
        "namespace-qualified callees must derive their exact call: signature"
    );
    use ast_sgrep_lang::ParserRegistry;
    let rows = ParserRegistry::new()
        .parse(Language::Php, php2)
        .unwrap()
        .pattern_nodes;
    let call_sigs: Vec<&str> = rows
        .iter()
        .map(|n| n.signature.as_str())
        .filter(|s| s.starts_with("call:"))
        .collect();
    assert!(
        call_sigs.contains(&"call:\\Foo::bar") && call_sigs.contains(&"call:Foo\\Bar::baz"),
        "namespace callee rows must be emitted so the pattern side can serve them: {call_sigs:?}"
    );
}

/// 74c-F4 (LOW, pass 74c): all-meta scoped-call spellings `$A::$B(1)` /
/// `$A::$A(1)` loud-fallbacked where sg answers (meta scope AND meta name,
/// concrete args included). sg 0.45.2 (php_matrix / php2): `$A::$B(1)` → the
/// `Foo::bar(1)` line; `$A::$A(1)` unifies the same name across scope and
/// name (`$x::$x(1)` only). The registered §21.2 cell `$A::bar(1)` (meta
/// scope + LITERAL name) stays loud by registration — see
/// `f74c_f2_php_registered_loud_cells_stay_loud`.
/// Mutant cells: drop the all-meta `::` arm of the php wrapper gate (both
/// faces flip RED); widen the gate to any `::` head (the 74c-F2 loud pins
/// flip RED — forbidden direction).
#[test]
fn f74c_f4_all_meta_scoped_call_answers_sg_exact() {
    let php = "<?php\nFoo::bar($u);\n\\Foo::bar($u);\nself::bar($u);\nstatic::bar($u);\nparent::bar($u);\n$obj::bar($u);\nApp\\Models\\User::find($u);\nFoo::bar();\nFoo::bar(1);\nFoo::bar($u, $v);\nFoo::bar($u)->baz();\nFoo::$dyn($u);\nFoo::bar(\n    $u\n);\nFoo::bar($v);\n$svc->run($u);\necho strtoupper($w);\n";
    for (pattern, want) in [
        ("$A::$B(1)", vec![10u32]),
        (
            "$A::$B($C)",
            vec![2u32, 3, 4, 5, 6, 7, 8, 10, 12, 13, 14, 17],
        ),
    ] {
        assert!(!needs_ast_grep_fallback(pattern), "{pattern} classifies");
        assert!(native_pattern_answerable(Language::Php, pattern));
        let hits = match_pattern(Language::Php, php, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on php_matrix (74c-F4): {hits:?}"
        );
    }
    let php2 = "<?php\n$x::$x(1);\nFoo::bar(1);\n$o?->m($u);\n$a->b->c($u);\n$y->a()->b();\nFoo\\Bar::baz(1);\n\\Foo::bar(2);\nobj::$dyn(3);\n";
    let hits = match_pattern(Language::Php, php2, "$A::$A(1)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![2u32],
        "$A::$A(1): sg unifies same-name scope/name (php2): {hits:?}"
    );
    let hits = match_pattern(Language::Php, php, "$A::$B(1)").unwrap();
    let hit = hits.iter().find(|h| h.line_start == 10).expect("line 10");
    assert_eq!(hit.captures.get("A").map(String::as_str), Some("Foo"));
    assert_eq!(hit.captures.get("B").map(String::as_str), Some("bar"));
}

/// 74c-F2 (LOW, pass 74c): the two §21.2 php registered loud cells were NOT
/// pinned anywhere in tests/, so the "every registered cell asserted" claim
/// had a hole — a future silent admission of exactly these cells would not
/// have failed any suite. RED-first is not observable here (behavior is
/// already loud); the pin's teeth are mutant-verified: widening the php
/// wrapper gate to ANY `::` head flips this test RED.
#[test]
fn f74c_f2_php_registered_loud_cells_stay_loud() {
    for pattern in ["$A::bar(1)", "Foo::nested(Foo::bar($A))", "$A + 1"] {
        // The registered loud driver: php cannot template the pattern (the
        // general-lane php build refuses), so core's post-walk gate fails the
        // query closed loudly whenever the native walk answers empty. Widening
        // the php wrapper gate (or the general-lane admission) flips this.
        assert!(
            !native_pattern_answerable(Language::Php, pattern),
            "{pattern} must stay php-unanswerable (registered §21.2 cell)"
        );
        assert!(
            !index_can_serve(pattern),
            "{pattern} must never be index-servable (the stale-row class)"
        );
        let hits = match_pattern(Language::Php, "<?php\nFoo::bar(1);\n", pattern).unwrap();
        assert!(hits.is_empty(), "{pattern} must answer nothing: {hits:?}");
    }
}

/// The index-serve half of the §21.2 loud posture: a pattern that is both
/// php-unanswerable AND index-servable would silently serve stale/wrong rows
/// (the F74c-1 shape). None of the registered cells may take that combination.
fn index_can_serve(pattern: &str) -> bool {
    match ast_sgrep_lang::cached_pattern_signatures(pattern) {
        Some(signatures) => ast_sgrep_lang::index_can_serve_pattern(pattern, &signatures),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// PASS 77b (r26 remediation): F76-1 (php `->` member-call CHAINS with meta
// args answered silent `ok:true []`) and F76-2 (`$$A` in php member args
// answered silent `ok:true []`). sg = ast-grep 0.45.2; every line set and
// capture below was probed live (2026-09-06, fixtures byte-identical to
// pass76a `fixtures/php_member/a.php` + `fixtures/php_member2/b.php`) BEFORE
// the fix — the RED state of this test against the pinned r26 binary
// (sha16 c08cc77412137d64) is the finding evidence. The pass-75 member lane
// (`f74a_2`) pinned only SINGLE-segment faces — exactly the coverage hole
// pass-76a exploited: any chain (≥2 call segments) with a metavariable
// argument fell through the classifier into the silent general lane.
// ---------------------------------------------------------------------------

/// F76-1 + F76-2 (P0, pass 76a): php member-call chains answer sg-exact, and
/// the member lane binds canonical `$$A` argument captures like every other
/// call lane (F74a-3 semantics).
/// Mutant cells (post-fix): drop the `->`-chain classifier arm or the chain
/// walk (every chain face flips RED — silent []); drop the per-segment arity
/// check (the `m1($A)->m2($A)` / `touch($C)` refusal faces flip); drop the
/// same-name unify veto (`$obj->m($A)->n($A)` flips); bind chain-arg captures
/// in the wrong namespace (the capture pins flip); route `$$A` member args
/// through a single-dollar-only binding (the `$$weird` capture pin flips).
#[test]
fn f76_php_member_chains_and_member_two_dollar_answer_sg_exact() {
    // Byte-identical to pass76a fixtures/php_member/a.php (sg-probed 2026-09-06).
    let a = "<?php\n$svc->run($u);\n$obj->m1()->m2($u);\n$obj->m($w)->n($u);\n$a?->b($u);\nclass C {\n  public function f($x) {\n    $this->m($x);\n    return $this->m($x)->chain();\n  }\n}\n$obj->p($u);\n$a->b()->c()->d($u);\n$svc->run($u, $v);\n$obj->m1()->m2($w);\necho $svc->run($u);\n$obj->m($u, $v, $w);\n$svc->run($$weird);\n$x = $svc->run($u);\n";
    for (pattern, want) in [
        // F76-1 chain faces (sg {3,15} / {4} / {13}).
        ("$obj->m1()->m2($A)", vec![3u32, 15]),
        ("$obj->m($A)->n($B)", vec![4u32]),
        ("$a->b()->c()->d($A)", vec![13u32]),
        // F76-1 × 2-dollar: the chain lane binds `$$A` like the flat member lane.
        ("$obj->m1()->m2($$A)", vec![3u32, 15]),
        // F76-2: `$$A` in flat member args answers exactly `$A`'s line set.
        ("$svc->run($$A)", vec![2u32, 16, 18, 19]),
        // Refusal faces (sg []): wrong name / wrong arity per segment, and the
        // same-name unify veto across chain segments ($w != $u on line 4).
        ("$obj->m2($A)", vec![]),
        ("$obj->m1()->m3($A)", vec![]),
        ("$obj->m1()->m2($A, $B)", vec![]),
        ("$obj->m1($A)->m2($A)", vec![]),
        ("$obj->m($A)->n($A)", vec![]),
        // Connector token-exactness keeps its registered posture: the plain
        // `->` template never answers the nullsafe site, and a bare trailing
        // name never answers a member call.
        ("$a->b($A)", vec![]),
        ("run($A)", vec![]),
        // Single-segment lane regression guards (f74a_2 posture, this corpus).
        ("$svc->run($A)", vec![2u32, 16, 18, 19]),
        ("$this->m($A)", vec![8u32, 9]),
        ("$obj->p($A)", vec![12u32]),
        ("$svc->run($A, $B)", vec![14u32]),
        ("$obj->m($A, $B, $C)", vec![17u32]),
        ("$obj->m($A)", vec![4u32]),
        ("$a?->b($A)", vec![5u32]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must be admitted: sg answers the php member face"
        );
        let hits = match_pattern(Language::Php, a, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on php_member/a.php (F76-1/F76-2): {hits:?}"
        );
    }
    // Captures: chain argument metavars bind sg's capture key `A`; `$$A`
    // binds the single namespace too — the arg node's exact text, including
    // the `$$weird` variable-variable face sg binds verbatim.
    let chain = match_pattern(Language::Php, a, "$obj->m1()->m2($A)").unwrap();
    let three = chain.iter().find(|h| h.line_start == 3).expect("line 3");
    assert_eq!(three.captures.get("A").map(String::as_str), Some("$u"));
    let fifteen = chain.iter().find(|h| h.line_start == 15).expect("line 15");
    assert_eq!(fifteen.captures.get("A").map(String::as_str), Some("$w"));
    let two_dollar = match_pattern(Language::Php, a, "$svc->run($$A)").unwrap();
    let two = two_dollar
        .iter()
        .find(|h| h.line_start == 2)
        .expect("line 2");
    assert_eq!(two.captures.get("A").map(String::as_str), Some("$u"));
    let weird = two_dollar
        .iter()
        .find(|h| h.line_start == 18)
        .expect("line 18");
    assert_eq!(weird.captures.get("A").map(String::as_str), Some("$$weird"));

    // Independent fixture (different names/spacing), sg-probed on
    // pass76a fixtures/php_member2/b.php: prefix-subnode answers, deep chains,
    // and per-segment arity refusals. The trailing receiver-class lines are
    // sg-probed 2026-09-06 (/tmp/p77b_link): a lowercase-led `$aBc` receiver
    // is literal php variable text — sg ANSWERS the identical source line —
    // while a MixedCase `$ABc` receiver poisons the whole sg pattern (rc=1,
    // answers NOTHING even with the literal line present), so the
    // classifier's receiver class check is load-bearing in BOTH directions.
    let b = "<?php\n$repo->find($id)->hydrate($row);\n$repo->find($id)->hydrate($row)->touch();\n$svc->run($$tok);\n$log->write($msg, $lvl);\n$obj->a($x)->b($y)->c($z);\n$a->b->c($u);\n$aBc->m1()->m2($u);\n$ABc->m1()->m2($u);\n";
    for (pattern, want) in [
        ("$repo->find($A)->hydrate($B)", vec![2u32, 3]),
        ("$repo->find($A)->hydrate($B)->touch()", vec![3u32]),
        ("$obj->a($A)->b($B)->c($C)", vec![6u32]),
        ("$svc->run($$A)", vec![4u32]),
        ("$log->write($A, $B)", vec![5u32]),
        ("$repo->find($A)->touch()", vec![]),
        ("$repo->find($A, $B)->hydrate($C)", vec![]),
        ("$repo->find($A)->hydrate($B)->touch($C)", vec![]),
        // sg-probed (rc=1): a property link is not a call — the empty-arg
        // mid segment must not fold onto it.
        ("$a->b()->c($A)", vec![]),
        // Receiver-class kill cells: literal answer for lowercase-led,
        // poisoned refusal for MixedCase (the mutant that literal-matches
        // `$ABc` answers line 8 where sg answers nothing).
        ("$aBc->m1()->m2($A)", vec![8u32]),
        ("$ABc->m1()->m2($A)", vec![]),
    ] {
        let hits = match_pattern(Language::Php, b, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on php_member2/b.php (F76-1): {hits:?}"
        );
    }
}

/// PASS 77b reconciliation pins (pass-76a §2 FLIP-1/FLIP-2, positive
/// direction): the pass-75a 2-dollar family widened these §21.2-registered
/// loud cells to sg-exact answers, so the register rows narrow to the
/// remaining loud residuals (literal-arg mixed-concrete cells stay loud —
/// still pinned by `f72a_2`'s 3-dollar cells). sg 0.45.2 line sets probed
/// live 2026-09-06 on byte-identical sources.
/// Mutant cell: revert the `parse_family_call` 2-dollar arm (every face here
/// flips RED — the registered pre-75 posture).
#[test]
fn f76_two_dollar_flip_cells_answer_sg_exact() {
    // FLIP-1: `g(1, $$A)` / `add(1, $$A)` — trailing 2-dollar rest sibling;
    // sg binds `$$A` to ANY single argument (literal `2` included).
    let ts = "g(1, fetch(a));\nadd(1, 2);\ng(1, 2);\nadd(1, fetch(a));\n";
    for (pattern, want) in [("g(1, $$A)", vec![1u32, 3]), ("add(1, $$A)", vec![2u32, 4])] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must be admitted (FLIP-1: sg answers the face)"
        );
        let hits = match_pattern(Language::TypeScript, ts, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set (FLIP-1): {hits:?}"
        );
    }
    // FLIP-2: `wrap.x(fetch($$A), 2)` answers {line 1} like sg; the
    // mixed-trailing-literal twin `fetch($$A, 2)` also answers its face —
    // `$$A` ≡ `$A` per F74a-3, literals included.
    let ts2 = "wrap.x(fetch(a), 2);\nfetch(a, 2);\n";
    for (pattern, want) in [
        ("wrap.x(fetch($$A), 2)", vec![1u32]),
        ("fetch($$A, 2)", vec![2u32]),
    ] {
        let hits = match_pattern(Language::TypeScript, ts2, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set (FLIP-2): {hits:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// PASS 79 (r28 remediation): F78-1 (php `->` member chain with a PROPERTY
// segment between calls × meta arg answered silent `ok:true []`) and F78-2
// (`?->` nullsafe ANYWHERE in a chain × meta arg answered silent `ok:true []`
// — the registered `?`-refusal fell through to the NeverMatches gate). sg =
// ast-grep 0.45.2; every line set and capture below was probed live
// (2026-09-07, /tmp/p79_probes fixtures byte-identical to the strings here)
// BEFORE the fix — the RED state of these tests against the pinned r28
// binary (sha16 08188baa4c391bb2) is the finding evidence. Cross-connector
// cells pin sg's per-link token exactness: a `->` link never answers a
// `?->` site and vice versa, and a property pattern segment never answers
// a call-position link (probed both directions).
// ---------------------------------------------------------------------------

/// F78-1 (P0, pass 78a): property-segment member chains answer sg-exact,
/// including the canonical metavariable property name (`$P` binds the
/// property text) and prefix subnodes of deeper property chains.
/// Mutant cells (post-fix): drop the mid-chain property admission in
/// `classify_member_call_chain` (every face here flips RED — silent []);
/// let a pattern property segment match a call-position link (the b-fixture
/// property/call cross cells flip); drop the property-name capture bind
/// (the `$P` capture pin flips); drop the last-segment-must-be-call
/// discipline (property-tail faces re-route off their serving lane).
#[test]
fn f78_property_segment_member_chains_answer_sg_exact() {
    // Byte-identical to the /tmp/p79_probes/fix/a.php sg probe fixture.
    let a = "<?php\n$svc->c1()->prop->c2($u);\n$svc->c1()->prop->c2();\n$svc->c1()->prop->c2($u, $v);\n$svc->c1()->prop->c2($w);\n$guard?->g1($u)->g2($v);\n$chain->h1($u)?->h2($v);\n$both?->m1($u)?->m2($v);\n$guard->g1($u)->g2($v);\n$mix?->p1->q1($u);\n$deep->d1($u)->dprop->d2($w)->d3();\n$x->prop->m($u);\n$xp?->pp->mm($u);\n";
    for (pattern, want) in [
        // F78-1 faces (sg {2,5} / {3} / {2,3,4,5} / {4}).
        ("$svc->c1()->prop->c2($A)", vec![2u32, 5]),
        ("$svc->c1()->prop->c2()", vec![3u32]),
        ("$svc->c1()->prop->c2($$$A)", vec![2u32, 3, 4, 5]),
        ("$svc->c1()->prop->c2($A, $B)", vec![4u32]),
        // Deep property chain (sg {11}) and its prefix subnode (sg answers
        // the inner `$deep->d1($u)->dprop->d2($w)` call node too).
        ("$deep->d1($A)->dprop->d2($B)->d3()", vec![11u32]),
        ("$deep->d1($A)->dprop->d2($B)", vec![11u32]),
        // Canonical metavariable property name: sg binds `P` to the property
        // text (`prop`) — probed `P={text:"prop"}` on both lines.
        ("$svc->c1()->$P->c2($A)", vec![2u32, 5]),
        // Receiver + property + single call (sg {12}); the flat two-segment
        // member-call shape is too shallow, so the chain lane owns the face.
        ("$x->prop->m($A)", vec![12u32]),
        // Wrong-name / wrong-arity refusal faces (sg []).
        ("$svc->c1()->prop->c3($A)", vec![]),
        ("$svc->c1()->prop->c2($A, $B, $C)", vec![]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must be admitted: sg answers the property-chain face"
        );
        let hits = match_pattern(Language::Php, a, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the property-chain fixture (F78-1): {hits:?}"
        );
    }
    // Captures: the chain arg binds the arg node text; the property
    // metavariable binds the property name text.
    let chain = match_pattern(Language::Php, a, "$svc->c1()->prop->c2($A)").unwrap();
    let two = chain.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(two.captures.get("A").map(String::as_str), Some("$u"));
    let five = chain.iter().find(|h| h.line_start == 5).expect("line 5");
    assert_eq!(five.captures.get("A").map(String::as_str), Some("$w"));
    let meta_prop = match_pattern(Language::Php, a, "$svc->c1()->$P->c2($A)").unwrap();
    let prop = meta_prop
        .iter()
        .find(|h| h.line_start == 2)
        .expect("line 2");
    assert_eq!(prop.captures.get("P").map(String::as_str), Some("prop"));

    // Independent fixture (byte-identical to /tmp/p79_probes/fix/b.php):
    // property-vs-call position discriminators in BOTH directions — a
    // property pattern segment answers only the property site, a call
    // pattern segment only the call site (sg {3} / {2}).
    let b = "<?php\n$svc->c1()->c2()->c3($u);\n$svc->c1()->c2->c3($u);\n$n?->m1($u)?->m2($v)?->m3($w);\n$a->m1($u)?->m2($v)?->m3($w);\n";
    for (pattern, want) in [
        ("$svc->c1()->c2()->c3($A)", vec![2u32]),
        ("$svc->c1()->c2->c3($A)", vec![3u32]),
    ] {
        let hits = match_pattern(Language::Php, b, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the property/call cross fixture (F78-1): {hits:?}"
        );
    }
}

/// F78-2 (P0, pass 78a): `?->` nullsafe links anywhere in a member chain
/// answer sg-exact — head, mid, and both-links faces — with per-link
/// connector token exactness (plain `->` patterns never answer nullsafe
/// sites and vice versa), `$$A` riding the nullsafe args, prefix subnodes,
/// and the previously-LOUD single-call nullsafe+property faces.
/// Mutant cells (post-fix): drop the `?->` split (every answering face here
/// flips RED — the `?` refusal falls back to the NeverMatches gate); drop
/// the per-link flag comparison (the three cross-connector [] cells
/// over-answer RED); drop the nullsafe candidate kinds from the walk (the
/// answering faces flip RED); bind `$P`-style property text through the
/// wrong arm (the mix-face capture pin flips).
#[test]
fn f78_nullsafe_chain_links_answer_sg_exact() {
    // Byte-identical to the /tmp/p79_probes/fix/a.php sg probe fixture.
    let a = "<?php\n$svc->c1()->prop->c2($u);\n$svc->c1()->prop->c2();\n$svc->c1()->prop->c2($u, $v);\n$svc->c1()->prop->c2($w);\n$guard?->g1($u)->g2($v);\n$chain->h1($u)?->h2($v);\n$both?->m1($u)?->m2($v);\n$guard->g1($u)->g2($v);\n$mix?->p1->q1($u);\n$deep->d1($u)->dprop->d2($w)->d3();\n$x->prop->m($u);\n$xp?->pp->mm($u);\n";
    for (pattern, want) in [
        // F78-2 faces: nullsafe head (sg {6}), mid (sg {7}), both links
        // (sg {8}).
        ("$guard?->g1($A)->g2($B)", vec![6u32]),
        ("$chain->h1($A)?->h2($B)", vec![7u32]),
        ("$both?->m1($A)?->m2($B)", vec![8u32]),
        // `$$A` rides the nullsafe chain args like `$A` (sg {6}).
        ("$guard?->g1($$A)->g2($B)", vec![6u32]),
        // Nullsafe head + property mid + single call: sg answers {10}; the
        // pre-79 subject answered this face LOUD (structural fallback).
        ("$mix?->p1->q1($A)", vec![10u32]),
        ("$xp?->pp->mm($A)", vec![13u32]),
        // Arity refusal through the nullsafe+property chain (sg []).
        ("$mix?->p1->q1($A, $B)", vec![]),
        // Connector token exactness (sg [] on the crossed cells): a plain
        // link pattern never answers a nullsafe site, and a nullsafe-head
        // pattern never answers a plain-head site. The all-plain pattern
        // answers exactly the plain line (sg {9}).
        ("$both?->m1($A)->m2($B)", vec![]),
        ("$guard?->g1($A)->g2($B)", vec![6u32]),
        ("$guard->g1($A)->g2($B)", vec![9u32]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must be admitted: sg answers the nullsafe-chain face"
        );
        let hits = match_pattern(Language::Php, a, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the nullsafe-chain fixture (F78-2): {hits:?}"
        );
    }
    // Captures: nullsafe chain args bind per segment (A from the nullsafe
    // g1 link, B from the plain g2 link); the mix chain binds the property
    // mid's trailing call arg.
    let head = match_pattern(Language::Php, a, "$guard?->g1($A)->g2($B)").unwrap();
    let six = head.iter().find(|h| h.line_start == 6).expect("line 6");
    assert_eq!(six.captures.get("A").map(String::as_str), Some("$u"));
    assert_eq!(six.captures.get("B").map(String::as_str), Some("$v"));
    let mix = match_pattern(Language::Php, a, "$mix?->p1->q1($A)").unwrap();
    let ten = mix.iter().find(|h| h.line_start == 10).expect("line 10");
    assert_eq!(ten.captures.get("A").map(String::as_str), Some("$u"));

    // Independent fixture (byte-identical to /tmp/p79_probes/fix/b.php):
    // deep nullsafe chains answer outermost AND prefix subnodes (sg {4});
    // per-link token exactness on the mid-nullsafe faces (sg [] crossed).
    let b = "<?php\n$svc->c1()->c2()->c3($u);\n$svc->c1()->c2->c3($u);\n$n?->m1($u)?->m2($v)?->m3($w);\n$a->m1($u)?->m2($v)?->m3($w);\n";
    for (pattern, want) in [
        ("$n?->m1($A)?->m2($B)?->m3($C)", vec![4u32]),
        ("$n?->m1($A)?->m2($B)", vec![4u32]),
        ("$a->m1($A)?->m2($B)", vec![5u32]),
        ("$a?->m1($A)?->m2($B)", vec![]),
        ("$a->m1($A)->m2($B)", vec![]),
    ] {
        let hits = match_pattern(Language::Php, b, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the deep nullsafe fixture (F78-2): {hits:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// PASS 81a (round r31) — FB-80a-01/02/03/05/06 remediation. Every `want`
// below is a live ast-grep 0.45.2 cell (`sg run --pattern <p> --lang <l>`)
// recorded 2026-09-07 against the byte-identical fixtures embedded here
// (gauntlet copies under fixtures/r31_81a/).
// ---------------------------------------------------------------------------

/// FB-80a-01 (HIGH, silent): property-TAIL member chains with meta args
/// answer sg-exact. The r29 admission gate deliberately kept property-LAST
/// chains off the lane; pass-80a proved that fall-through answers silent
/// `[]` whenever the pattern carries a metavariable argument
/// (`$o->c1($A)->tailProp` sg {2,6}). sg answers the whole member-access
/// node AND the depth-equal prefix subnode of a deeper chain, for every
/// argument shape, through plain and nullsafe connectors.
/// Mutant cells (post-fix): re-refuse the last property segment (every
/// answering face flips silent []); drop the member-access candidate kinds
/// from the walk (same flip); let a pattern call segment land on a property
/// link or vice versa (the flat/cross cells flip).
#[test]
fn f80a_property_tail_chains_answer_sg_exact() {
    let pt = "<?php\n$o->c1($u)->tailProp;\n$o->c1($u, $v)->tailProp;\n$o->c1()->tailProp;\n$o?->c1($u)->tailProp;\n$o->c1($u)->tailProp->c2($w);\n$o->c1($u)->tailOther;\n$o->c1($u)->prop->tailProp;\n$o->c1($u)?->tailProp;\n";
    for (pattern, want) in [
        // The FB-80a-01 face (sg {2,6}): the property-tail node itself plus
        // the depth-3 prefix subnode inside line 6's longer chain.
        ("$o->c1($A)->tailProp", vec![2u32, 6]),
        // Argument shapes ride the tail like the call-tail faces (sg {3} /
        // {2,3,4,6} / {4}).
        ("$o->c1($A, $B)->tailProp", vec![3u32]),
        ("$o->c1($$$A)->tailProp", vec![2u32, 3, 4, 6]),
        ("$o->c1()->tailProp", vec![4u32]),
        // Nullsafe connector INTO the call (sg {5}) and INTO the property
        // tail (sg {9}) — per-link token exactness as registered by F78-2.
        ("$o?->c1($A)->tailProp", vec![5u32]),
        ("$o->c1($A)?->tailProp", vec![9u32]),
        // Wrong tail name answers only its own line (sg {7}); the deeper
        // property-mid+property-tail chain answers sg {8}; the call-tail
        // control stays on its r29 lane (sg {6}).
        ("$o->c1($A)->tailOther", vec![7u32]),
        ("$o->c1($A)->prop->tailProp", vec![8u32]),
        ("$o->c1($A)->tailProp->c2($B)", vec![6u32]),
        // A dynamic-property tail spelling never answers a plain property
        // site (sg []): the literal text `$tail` exists nowhere.
        ("$o->c1($A)->$tail", vec![]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must be admitted: sg answers the property-tail face"
        );
        let hits = match_pattern(Language::Php, pt, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the property-tail fixture (FB-80a-01): {hits:?}"
        );
    }
    // The single-call face keeps its flat MemberCall lane (sg answers the
    // call node and its prefix subnodes {2,6,7,8,9} — line 9's inner call
    // is a PLAIN member call, the nullsafe connector rides INTO the tail)
    // — the chain lane must not absorb it.
    let flat = match_pattern(Language::Php, pt, "$o->c1($A)").unwrap();
    assert_eq!(
        klass_lines(&flat),
        vec![2u32, 6, 7, 8, 9],
        "$o->c1($A): flat member-call lane contract unchanged: {flat:?}"
    );
    // Capture: the chain argument binds the argument text (sg -r rewrite
    // `QQ($A)` applies `QQ($u)` on line 2 and `QQ($u)->c2($w)` on line 6).
    let hits = match_pattern(Language::Php, pt, "$o->c1($A)->tailProp").unwrap();
    let two = hits.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(two.captures.get("A").map(String::as_str), Some("$u"));
}

/// FB-80a-02 (HIGH, silent): a lowercase-led dynamic-property link
/// (`$dyn`) answers sg-exact as BYTE-EXACT literal property text — sg
/// answers `$svc->c1()->$dyn->c2($A)` only on the `$dyn` line, a different
/// name only on its own line, a canonical `$DYN` wildcards across ALL
/// property links (binding the name-node text INCLUDING the dollar), and a
/// plain spelling never answers a dynamic site (token exactness).
/// Mutant cells (post-fix): treat `$dyn` as a capture/wildcard (the
/// name-specific cells over-answer); refuse lowercase-led property links
/// again (the FB-80a-02 faces flip silent []).
#[test]
fn f80a_dynamic_property_segments_answer_sg_exact() {
    let dp = "<?php\n$svc->c1()->$dyn->c2($u);\n$svc->c1()->$other->c2($u);\n$svc->c1()->prop->c2($u);\n$svc->c1()->$dyn->c3($u);\n$obj->$key;\n$obj->$key->m($u);\n$svc->c1()->$dyn;\n";
    for (pattern, want) in [
        // The FB-80a-02 face (sg {2}) — and its name-specific control: the
        // literal reading answers only the same-named line (sg {3}).
        ("$svc->c1()->$dyn->c2($A)", vec![2u32]),
        ("$svc->c1()->$other->c2($A)", vec![3u32]),
        // No `$dyn2` site exists (sg []); the canonical property metavar
        // wildcards across every property link (sg {2,3,4}, the F78-1 row).
        ("$svc->c1()->$dyn2", vec![]),
        ("$svc->c1()->$P->c2($A)", vec![2u32, 3, 4]),
        // Dynamic property mid + call tail on the flat shapes (sg {7}); a
        // plain spelling never answers the dynamic site (sg []).
        ("$obj->$key->m($A)", vec![7u32]),
        ("$obj->key->m($A)", vec![]),
        // Property-TAIL spellings: dynamic literal (sg {2,5,8}) and the
        // canonical wildcard across every property text (sg {2,3,4,5,8}).
        ("$svc->c1()->$dyn", vec![2u32, 5, 8]),
        ("$svc->c1()->$DYN", vec![2u32, 3, 4, 5, 8]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must be admitted: sg answers the dynamic-property face"
        );
        let hits = match_pattern(Language::Php, dp, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the dynamic-property fixture (FB-80a-02): {hits:?}"
        );
    }
    // Captures: a canonical property metavar binds the name-node text
    // INCLUDING the dollar for dynamic sites (sg -r `X($DYN)` applies
    // `X($dyn)` / `X($other)`), and the plain property text without.
    let wild = match_pattern(Language::Php, dp, "$svc->c1()->$DYN->c2($A)").unwrap();
    let two = wild.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(two.captures.get("DYN").map(String::as_str), Some("$dyn"));
    let four = wild.iter().find(|h| h.line_start == 4).expect("line 4");
    assert_eq!(four.captures.get("DYN").map(String::as_str), Some("prop"));
}

/// FB-80a-03 (MED, silent): a php assignment whose LHS is a literal
/// lowercase-led `$var` and whose RHS is ONE bare canonical metavariable
/// answers sg-exact — the RHS capture binds the RHS expression text
/// (`"str"` with quotes, `$delta`), the node span excludes the `;`
/// (sg -r `VV=$V` applies `VV=$delta` on line 4), `$$$V` and the
/// semicolon-less spelling answer the same face, and a MIXED RHS
/// (`$V + 1`) keeps the registered NeverMatches empty (sg refuses it too).
/// Mutant cells (post-fix): drop the lane (every face flips silent []);
/// wildcard the LHS (the wrong-head cells over-answer).
#[test]
fn f80a_php_assignment_bare_meta_rhs_answers_sg_exact() {
    let asg = "<?php\n$alpha = \"str\";\n$beta = 7;\n$gamma = $delta;\n$delta = [$x, $y];\n$eps = foo(1);\n$zeta = $alpha + 1;\n";
    for (pattern, want) in [
        // The FB-80a-03 faces: every lowercase-led head answers its own
        // assignment line (sg {2}/{3}/{4}/{5}/{6}/{7}).
        ("$alpha = $V;", vec![2u32]),
        ("$beta = $V;", vec![3u32]),
        ("$gamma = $V;", vec![4u32]),
        ("$delta = $V;", vec![5u32]),
        ("$eps = $V;", vec![6u32]),
        ("$zeta = $V;", vec![7u32]),
        // Rest metavariable RHS (sg {2}) and the semicolon-less spelling
        // (sg {2}).
        ("$alpha = $$$V;", vec![2u32]),
        ("$alpha = $V", vec![2u32]),
        // A canonical metavariable RHS wildcards any LHS expression —
        // including another metavariable (sg {2}).
        ("$alpha = $ALPHA;", vec![2u32]),
        // A fully-canonical assignment template answers NOTHING in sg
        // (probed []) — the lane must not over-serve the canonical LHS.
        ("$ALPHA = $V;", vec![]),
        // A mixed RHS keeps the registered NeverMatches class (sg rc=1 []).
        ("$alpha = $V + 1;", vec![]),
        // All-lowercase faces keep the literal lane (sg {4} byte-exact).
        ("$gamma = $delta;", vec![4u32]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must be admitted: sg answers the assignment face"
        );
        let hits = match_pattern(Language::Php, asg, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the assignment fixture (FB-80a-03): {hits:?}"
        );
    }
    // Captures: V binds the RHS expression text (sg pins `VV="str"` and
    // `VV=$delta`); `$$$V` binds the rest spelling in its own namespace.
    let str_face = match_pattern(Language::Php, asg, "$alpha = $V;").unwrap();
    let two = str_face.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(two.captures.get("V").map(String::as_str), Some("\"str\""));
    let var_face = match_pattern(Language::Php, asg, "$gamma = $V;").unwrap();
    let four = var_face.iter().find(|h| h.line_start == 4).expect("line 4");
    assert_eq!(four.captures.get("V").map(String::as_str), Some("$delta"));
}

/// FB-80a-05 (MED, loud false-bail): chain argument lists spelled as a
/// comma list of metavariables bind EVERY name to its positional candidate
/// argument — the codemod lane's "rewrite references unbound metavariable
/// $A" bail on `$w->q2($A, $B)->r2()` was the missing per-argument capture,
/// not an unbound name (sg binds A=$u, B=$v and applies `QQ($u, $v)`).
/// Mutant cells (post-fix): drop the positional binding (both capture pins
/// flip — the exact false-bail condition); bind only the first name (the B
/// pin flips); single-meta whole-list binding must stay byte-compatible.
#[test]
fn f80a_chain_argument_metas_bind_positionally() {
    let mt = "<?php\n$w->q1($u)->r1();\n$w->q2($u, $v)->r2();\n$w->q3()->r3($u);\n$log->write($u, $v)->flush();\n";
    for (pattern, want) in [
        ("$w->q2($A, $B)->r2()", vec![3u32]),
        ("$log->write($A, $B)->flush()", vec![5u32]),
        // Arity refusal (sg []).
        ("$w->q2($A)->r2()", vec![]),
    ] {
        let hits = match_pattern(Language::Php, mt, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the multi-meta chain fixture (FB-80a-05): {hits:?}"
        );
    }
    let two = match_pattern(Language::Php, mt, "$w->q2($A, $B)->r2()").unwrap();
    let three = two.iter().find(|h| h.line_start == 3).expect("line 3");
    assert_eq!(three.captures.get("A").map(String::as_str), Some("$u"));
    assert_eq!(three.captures.get("B").map(String::as_str), Some("$v"));
    let write = match_pattern(Language::Php, mt, "$log->write($A, $B)->flush()").unwrap();
    let five = write.iter().find(|h| h.line_start == 5).expect("line 5");
    assert_eq!(five.captures.get("A").map(String::as_str), Some("$u"));
    assert_eq!(five.captures.get("B").map(String::as_str), Some("$v"));
    // The single-meta whole-list contract keeps its registered binding
    // (A = the list content text, not a positional slot).
    let single = match_pattern(Language::Php, mt, "$w->q1($A)->r1()").unwrap();
    let line2 = single.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(line2.captures.get("A").map(String::as_str), Some("$u"));

    // The dotted chain lane carries the same positional contract
    // (sg {3}: `r.m1($A, $B).m2()` binds A=1, B=2 — sg -r applies
    // `QQ(1, 2)`; `r.m1($A).m2()` answers only the single-arg line; the
    // TAIL call's `$B, $C` list answers {5} with B=4, C=5 — sg -r applies
    // `QQ(4, 5)`).
    let jch = "class J {\n    void go() {\n        r.m1(1, 2).m2();\n        r.m1(1).m2();\n        r.m1(3).m2(4, 5);\n    }\n}\n";
    let hits = match_pattern(Language::Java, jch, "r.m1($A, $B).m2()").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![3u32],
        "r.m1($A, $B).m2(): sg 0.45.2 line set (FB-80a-05 dotted face): {hits:?}"
    );
    let three = hits.iter().find(|h| h.line_start == 3).expect("line 3");
    assert_eq!(three.captures.get("A").map(String::as_str), Some("1"));
    assert_eq!(three.captures.get("B").map(String::as_str), Some("2"));
    let single = match_pattern(Language::Java, jch, "r.m1($A).m2()").unwrap();
    assert_eq!(
        klass_lines(&single),
        vec![4u32],
        "r.m1($A).m2(): sg 0.45.2 line set: {single:?}"
    );
    let tail = match_pattern(Language::Java, jch, "r.m1($A).m2($B, $C)").unwrap();
    assert_eq!(
        klass_lines(&tail),
        vec![5u32],
        "r.m1($A).m2($B, $C): sg 0.45.2 line set (tail-site positional bind): {tail:?}"
    );
    let five = tail.iter().find(|h| h.line_start == 5).expect("line 5");
    assert_eq!(five.captures.get("B").map(String::as_str), Some("4"));
    assert_eq!(five.captures.get("C").map(String::as_str), Some("5"));
}

/// FB-80a-06 (HIGH, CLI-independent): chain matching tolerates
/// newline/indentation BETWEEN chain links and inside argument lists —
/// sg answers the multiline pattern spellings with the same line set as
/// the single-line spelling, over single-line AND multiline sources
/// (java dot chains probed 0.45.2; the match span covers the full
/// multiline source).
/// Mutant cells (post-fix): drop the per-segment trim in the dotted
/// callee-path parse (the multiline-pattern cells flip silent [] while the
/// single-line control stays green — the exact F26-0148 matcher face).
#[test]
fn f80a_chain_patterns_tolerate_newlines_between_links() {
    // Single-line source with the r30 fixture shape; multiline sources on
    // every link boundary and inside the argument list.
    let jml = "class Main {\n    void go() {\n        cfg.out.reload(7);\n        cfg.out.\n            reload(8);\n        cfg\n            .out.reload(9);\n        cfg.out.reload(\n            10);\n    }\n}\n";
    for (pattern, want) in [
        // Single-line spelling control: every source shape answers (sg
        // rows 3-9 = matches starting at 3, 4, 6, 8).
        ("$O.out.$M($A)", vec![3u32, 4, 6, 8]),
        // Newline between the property link and the call (the FB-80a-06
        // pattern spelling, sg rows 3-9).
        ("$O.out\n.$M($A)", vec![3u32, 4, 6, 8]),
        // Newline between the head and the property link (sg rows 3-9).
        ("$O\n.out.$M($A)", vec![3u32, 4, 6, 8]),
        // Literal head with the newline spelling (sg rows 3-9).
        ("cfg.out\n.$M($A)", vec![3u32, 4, 6, 8]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must be admitted: sg answers the multiline chain face"
        );
        let hits = match_pattern(Language::Java, jml, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the multiline java fixture (FB-80a-06): {hits:?}"
        );
    }
    // php chain faces keep the same tolerance on their serving lane.
    let pt = "<?php\n$o->c1($u)->tailProp;\n$o->c1($u, $v)->tailProp;\n$o->c1()->tailProp;\n$o?->c1($u)->tailProp;\n$o->c1($u)->tailProp->c2($w);\n$o->c1($u)->tailOther;\n$o->c1($u)->prop->tailProp;\n$o->c1($u)?->tailProp;\n";
    for (pattern, want) in [
        ("$o->c1($A)\n->tailProp", vec![2u32, 6]),
        ("$o->c1($A)->prop\n->tailProp", vec![8u32]),
        ("$o->c1($A)->tailProp->\nc2($B)", vec![6u32]),
    ] {
        let hits = match_pattern(Language::Php, pt, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the multiline php faces (FB-80a-06): {hits:?}"
        );
    }
}

/// FB-80a-06 ROOT CAUSE (r31 reconciler, pass 81): the memchr PREFILTER
/// literal, not the matcher. `required_pattern_literal` picked the longest
/// `$`-free dotted callee segment WITHOUT trimming it, so the admitted
/// multiline chain spelling `$O.out\n.$M($A)` yielded the literal `"out\n"`
/// (the callee split keeps the interior newline) and its collapsed-ingress
/// twin `$O.out .$M($A)` (split_in_path_scope rejoins the newline as a
/// space — the registered F26-0148 search face) yielded `"out "` with a
/// trailing space. No source file's `out.` bytes contain either spelling,
/// so the prefilter skipped EVERY file and the green matcher never ran:
/// search answered silent [] and codemod planned ok:true 0 edits. Every
/// returned literal must be whitespace-free bytes (a `None` return is the
/// sound no-prefilter — both consumers, core/pattern.rs and codemod.rs,
/// scan the file when the literal is `None`).
/// Mutant cells (pre-fix): `$O.out\n.$M($A)` → `Some("out\n")`,
/// `$O.out .$M($A)` → `Some("out ")`, `$O.\n.$M($A)` → `Some("\n")` —
/// whitespace-carrying literals, all caught by this test; the single-line
/// control already returned the sound literal pre-fix.
#[test]
fn f80a_prefilter_literal_is_whitespace_free_for_multiline_chains() {
    // The registered FB-80a-06 multiline pattern spelling.
    assert_eq!(
        ast_sgrep_lang::required_pattern_literal("$O.out\n.$M($A)").as_deref(),
        Some("out"),
        "multiline chain prefilter literal must be the trimmed segment"
    );
    // The collapsed-ingress face: same callee with the newline rejoined as
    // a space (the registered F26-0148 search spelling).
    assert_eq!(
        ast_sgrep_lang::required_pattern_literal("$O.out .$M($A)").as_deref(),
        Some("out"),
        "collapsed-spelling prefilter literal must be the trimmed segment"
    );
    // Whitespace-only `$`-free segments leave NO literal: `None` (no
    // prefilter) — never a whitespace-only literal no file can contain.
    assert_eq!(
        ast_sgrep_lang::required_pattern_literal("$O.\n.$M($A)").as_deref(),
        None,
        "whitespace-only segments must yield no prefilter, not an unsound one"
    );
    // Control: the single-line spelling already yields the exact sound
    // literal pre-fix and must not regress.
    assert_eq!(
        ast_sgrep_lang::required_pattern_literal("$O.out.$M($A)").as_deref(),
        Some("out"),
        "single-line control keeps its exact literal"
    );
}

/// FB-82a-01 (MED, silent): a `$$name` LOWERCASE double-dollar token in a
/// dynamic-property LINK position admits as byte-exact literal property
/// text — sg 0.45.2 answers `$svc->c1()->$$dyn->c2($A)` on the `$$dyn`
/// line. The r31 admission covered single-`$` lowercase links and left
/// `$$`+lowercase in the never-silent fall-through. Controls (all probed
/// sg 0.45.2): the `$$dyn` TAIL spelling already answered through the
/// literal lane, a `$$dyn2` site never answers, `$$Dyn` (mixed case)
/// keeps its registered refusal, the canonical `$$DYN` wildcards across
/// every property link binding the text INCLUDING the dollars, and a
/// `$$key` RECEIVER face stays refused (sg []).
/// Mutant cells (post-fix): treat `$$dyn` as a capture (the `$$dyn2`
/// no-site cell over-answers); refuse `$$`+lowercase links again (the
/// mid-link face flips silent []).
#[test]
fn f83a_dollar2_lowercase_property_link_answers_sg_exact() {
    let dyn2 = "<?php\n$svc->c1()->dyn->c2($u);\n$svc->c1()->dyn;\n$svc->$dyn->c2($u);\n$svc?->$dyn->c2($u);\n$svc->c1()->$Dyn->c2($u);\n$svc->c1()->$$dyn->c2($u);\n$obj->$key;\n";
    for (pattern, want) in [
        // The FB-82a-01 face: the `$$dyn` link admits as literal text
        // (sg {7}).
        ("$svc->c1()->$$dyn->c2($A)", vec![7u32]),
        // The tail spelling already answered (literal-lane control, sg {7}).
        ("$svc->c1()->$$dyn", vec![7u32]),
        // Name specificity: no `$$dyn2` site exists (sg []).
        ("$svc->c1()->$$dyn2", vec![]),
        // Mixed-case `$$Dyn` keeps the registered refusal (sg []).
        ("$svc->c1()->$$Dyn->c2($A)", vec![]),
        // Canonical `$$DYN` wildcards every property link (sg {2,6,7}).
        ("$svc->c1()->$$DYN->c2($A)", vec![2u32, 6, 7]),
        // A `$$key` RECEIVER stays refused (sg []).
        ("$obj->$$key", vec![]),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern} must be admitted: sg answers or accepted-empties the face"
        );
        let hits = match_pattern(Language::Php, dyn2, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the $$dyn fixture (FB-82a-01): {hits:?}"
        );
    }
    let wild = match_pattern(Language::Php, dyn2, "$svc->c1()->$$DYN->c2($A)").unwrap();
    let seven = wild.iter().find(|h| h.line_start == 7).expect("line 7");
    assert_eq!(
        seven.captures.get("DYN").map(String::as_str),
        Some("$$dyn"),
        "the canonical property wildcard binds the double-dollar text verbatim"
    );
}

/// FB-82a-02/82c-1 (MED, silent): the php lowercase-literal + canonical-meta
/// OPERAND family answers across sg's actual grammar — every augmented
/// assignment operator (`+= -= *= /= .= ??= |= <<=`), every probed binary
/// operator (`== === < <=> + . ??`), member/dim/variable-variable LHS, and
/// RHS EXPRESSION patterns mixing canonical metas with literal operands
/// (`$v + 1`, `f($v)`, `[$v, $w]`, `-$v`, `$v->p`, `"x" . $v`, `A_CONST`).
/// The registered narrow lane (bare-meta RHS only) silent-emptied every one
/// of these sg-answering faces. Controls (probed sg []): a canonical LHS
/// (`$ALPHA = $V;`), a MixedCase LHS, and a pattern assignment whose LHS is
/// a META (`$alpha = $V = $W;` — sg refuses meta assignment-targets) stay
/// out. HIGH-1 (84c, r35): the `$alpha = $V = $W;` refusal cell used to pass
/// VACUOUSLY — the fixture had no chained-assignment source line, so the
/// inert meta-target veto was never exercised. Lines 31/32 give it teeth:
/// sg 0.45.2 answers BOTH chained lines through the bare-meta wildcard
/// (`$alpha = $V;` → {31,32} probed) and answers the literal-inner-target
/// pattern `$alpha = $bcd = $V;` on its own line (inner target `$bcd` is
/// literal code — sg {32}), while any META assignment-target
/// (`$alpha = $V = $W;`) refuses even against the exact chained source
/// (probed sg [] rc1). Mutant cells (post-fix): wildcard the LHS (the
/// `$ALPHA`/`$Alpha` controls over-answer); drop an operator row (that face
/// flips silent []); slice the veto against the bare RHS instead of the
/// wrapper doc (the `$alpha = $V = $W;` cell re-answers lines 31/32); veto
/// literal inner targets too (the `$alpha = $bcd = $V;` cell flips []).
#[test]
fn f83a_php_binary_operand_faces_answer_sg_exact() {
    let b2 = "<?php\n$alpha = $v;\n$alpha += $v;\n$alpha -= $v;\n$alpha *= $v;\n$alpha /= $v;\n$alpha .= $v;\n$alpha ??= $v;\n$alpha |= $v;\n$alpha <<= $v;\n$alpha == $v;\n$alpha === $v;\n$alpha < $v;\n$alpha <=> $v;\n$alpha + $v;\n$alpha . $v;\n$alpha ?? $v;\n$alpha = $v + 1;\n$alpha = 1 + $v;\n$alpha = $v . $w;\n$alpha = f($v);\n$alpha = [$v, $w];\n$alpha = -$v;\n$alpha = $v->p;\n$alpha = $v[0];\n$alpha = \"x\" . $v;\n$alpha = $$v + 1;\n$this->prop = $v;\n$eps[0] = $v;\n$$zeta = $v;\n$alpha = $b = 1;\n$alpha = $bcd = $V;\n";
    for (pattern, want) in [
        // The r31 registered control (bare-meta RHS, sg {2,18..27} — the
        // meta RHS wildcards every RHS expression shape). HIGH-1 (r35): the
        // appended chained lines 31/32 are part of the same sg answer set
        // (probed: the bare meta wildcards the whole chained RHS).
        (
            "$alpha = $V;",
            vec![2u32, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 31, 32],
        ),
        // Augmented assignment operators (sg rows 3-10).
        ("$alpha += $V;", vec![3u32]),
        ("$alpha -= $V;", vec![4u32]),
        ("$alpha *= $V;", vec![5u32]),
        ("$alpha /= $V;", vec![6u32]),
        ("$alpha .= $V;", vec![7u32]),
        ("$alpha ??= $V;", vec![8u32]),
        ("$alpha |= $V;", vec![9u32]),
        ("$alpha <<= $V;", vec![10u32]),
        // Binary operand faces (sg rows 11-17).
        ("$alpha == $V;", vec![11u32]),
        ("$alpha === $V;", vec![12u32]),
        ("$alpha < $V;", vec![13u32]),
        ("$alpha <=> $V;", vec![14u32]),
        ("$alpha + $V;", vec![15u32]),
        ("$alpha . $V;", vec![16u32]),
        ("$alpha ?? $V;", vec![17u32]),
        // RHS expression patterns: canonical metas bind sub-expression
        // positions, literal operands match byte-exactly (sg rows as shown;
        // `$$V` binds the 2-dollar variable too).
        ("$alpha = $V + 1;", vec![18u32, 27]),
        ("$alpha = $V . $W;", vec![20u32, 26]),
        ("$alpha = $$V + 1;", vec![18u32, 27]),
        ("$alpha = $V->p;", vec![24u32]),
        ("$alpha = \"x\" . $V;", vec![26u32]),
        // Member / dim / variable-variable LHS admit as byte-exact literal
        // targets (sg rows 28/29/30).
        ("$this->prop = $V;", vec![28u32]),
        ("$eps[0] = $V;", vec![29u32]),
        ("$$zeta = $V;", vec![30u32]),
        // Refusal controls (probed sg []): canonical LHS, MixedCase LHS,
        // meta assignment-target inside the RHS. HIGH-1 (r35): the meta-
        // target cell now has TEETH — lines 31/32 are chained-assignment
        // sources the inert veto used to answer (sg refuses the face).
        ("$alpha = $V = $W;", vec![]),
        ("$ALPHA = $V;", vec![]),
        ("$Alpha = $V;", vec![]),
        // HIGH-1 false-veto direction: a LITERAL inner target with a meta
        // inner RHS is sg-answering code (probed sg {32}; the doc-relative
        // mis-slice used to veto it silently).
        ("$alpha = $bcd = $V;", vec![32u32]),
    ] {
        let hits = match_pattern(Language::Php, b2, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the operand fixture (FB-82a-02): {hits:?}"
        );
    }
    // Captures: V and W bind their operand expression texts (line 20
    // `$v . $w`; line 26 `"x" . $v` binds V to the string literal).
    let both = match_pattern(Language::Php, b2, "$alpha = $V . $W;").unwrap();
    let twenty = both.iter().find(|h| h.line_start == 20).expect("line 20");
    assert_eq!(twenty.captures.get("V").map(String::as_str), Some("$v"));
    assert_eq!(twenty.captures.get("W").map(String::as_str), Some("$w"));
    let twentysix = both.iter().find(|h| h.line_start == 26).expect("line 26");
    assert_eq!(
        twentysix.captures.get("V").map(String::as_str),
        Some("\"x\"")
    );
    assert_eq!(twentysix.captures.get("W").map(String::as_str), Some("$v"));
}

/// HIGH-2 (84c, r35 PANIC): `split_php_binary` sliced its operator probe at
/// a byte-scanned index, so a multi-byte character in the pattern's top
/// level (`$α`, `$Ü` — PHP allows non-ASCII identifier bytes) panicked on
/// `p[i..]` ("byte index N is not a char boundary") before any answer was
/// produced. sg 0.45.2 answers BOTH faces [] rc1 (parsed-but-empty, probed
/// on a fixture carrying the exact spellings), so the subject must produce
/// a clean empty answer — never a panic, never a crash. Mutant cell:
/// remove the `is_char_boundary` guard and the first `unwrap()` aborts the
/// test process.
#[test]
fn f85a_php_non_ascii_pattern_answers_clean_empty_without_panicking() {
    let src = "<?php\n$alpha = $v;\n";
    for pattern in ["$α = $V;", "$Ü = $V;"] {
        // RED: this call panicked pre-fix (char-boundary abort).
        let hits = match_pattern(Language::Php, src, pattern).unwrap();
        assert!(
            hits.is_empty(),
            "{pattern}: sg 0.45.2 answers [] rc1 on the non-ASCII spelling (probed); got {hits:?}"
        );
    }
}

/// FB-84a-01 (r34, silent FP): the doubled-sign unary operators collapse —
/// pattern `$a = --$V;` answered the `--$v` sites and `$alpha = $V--;` the
/// postfix sites, while sg 0.45.2 parses its own grammar into an ERROR for
/// every doubled-sign unary face and answers NOTHING (probed [] rc1 even
/// against the byte-identical source line). Token-exactness holds in the
/// single-sign controls: `$a = -$V;` answers {4,5}, `$a = +$V;` answers {7}
/// (probed sg), and the bare-meta wildcard still answers the postfix lines
/// for a plain `$alpha = $V;` pattern (probed sg {10,11}). Mutant cells:
/// drop the doubled-sign refusal and the `--`/`++` cells re-answer their
/// source lines; widen the refusal to single signs and the `-$V`/`+$V`
/// controls flip [].
#[test]
fn f85a_php_doubled_sign_unary_faces_never_answer() {
    let op = "<?php\n$a = ++$v;\n$a = --$v;\n$a = -$v;\n$a = -$w;\n$a = --$w;\n$a = +$v;\n$a = ++$w;\n$b = 1;\n$alpha = $v--;\n$alpha = $w++;\n";
    for (pattern, want) in [
        ("$a = --$V;", vec![]),
        ("$a = ++$V;", vec![]),
        // Single-sign controls stay sg-exact.
        ("$a = -$V;", vec![4u32, 5]),
        ("$a = +$V;", vec![7u32]),
        // Postfix doubled-sign faces refuse too (sg [] rc1 against the
        // exact line 10/11 spellings).
        ("$alpha = $V--;", vec![]),
        ("$alpha = $V++;", vec![]),
        // The bare-meta wildcard keeps answering the postfix lines.
        ("$alpha = $V;", vec![10u32, 11]),
    ] {
        let hits = match_pattern(Language::Php, op, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the doubled-sign fixture (FB-84a-01): {hits:?}"
        );
    }
}

/// FB-84a-02 (r34, silent fail-open): a bare connector token answered EVERY
/// AST connector site through the literal lane (`->` hit both member-link
/// lines, `::` hit the scoped-call line, silent rc0). sg 0.45.2 parses the
/// bare token leniently and answers NOTHING (probed [] rc0, "parsed the
/// pattern but it matched nothing"). Not text containment: the subject
/// answered [] on a connector-free fixture, and `=>` keeps its own sg
/// behavior (it answers array-pair sites). Mutant cell: let the bare token
/// reach the literal lane again and the connector sites re-answer.
#[test]
fn f85a_bare_connector_tokens_never_answer() {
    let conn = "<?php\n$a = 1;\n$o->c1(2);\nFoo::bar(3);\n$x = $y -> $z;\n$s = A::B;\necho \"x => y\";\n$t = [\"k\" => 1];\n";
    for pattern in ["->", "::"] {
        let hits = match_pattern(Language::Php, conn, pattern).unwrap();
        assert!(
            hits.is_empty(),
            "{pattern}: sg 0.45.2 answers [] rc0 on the bare connector token (probed); got {hits:?}"
        );
    }
}

/// FB-84a-03 (r34, silent miss): a php comment between binary operands
/// defeated the structured RHS match (`$a = $v /* = */ + 1;` — sg answers,
/// subject []). sg 0.45.2 aligns expression children with CANDIDATE comment
/// children transparent (probed {3,4} answered; paren spellings {6,9,10}
/// answered by the paren pattern) while a PATTERN-side comment must find
/// its text-exact counterpart (probed: the commented paren pattern answers
/// only the self line 10 — a different comment text does not match). A
/// comment sitting directly inside the matched operator node keeps sg's
/// strict alignment (probed sg refuses `$a = /* h */ $v + 1;` for the
/// uncommented pattern). Mutant cells: strict child-count zip (lines 3/4/6
/// flip silent); skip candidate comments at the operator-node level too
/// (line 5 over-answers); treat pattern comments as transparent (the
/// commented-paren cell over-answers line 6).
#[test]
fn f85a_php_comment_between_binary_operands_answered() {
    let com = "<?php\n$a = $v + 1;\n$a = $v /* = */ + 1;\n$a = $v + /* c */ 1;\n$a = /* h */ $v + 1;\n$a = ($v /* p */ + 1);\n$a = $v + 1; // tail\n$a = $w . /* m */ $x;\n$a = ($v + 1);\n$a = ($v /* q */ + 1);\n";
    for (pattern, want) in [
        ("$a = $V + 1;", vec![2u32, 3, 4, 7]),
        ("$a = ($V + 1);", vec![6u32, 9, 10]),
        ("$a = ($V /* q */ + 1);", vec![10u32]),
        ("$a = $V . $W;", vec![8u32]),
    ] {
        let hits = match_pattern(Language::Php, com, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the comment fixture (FB-84a-03): {hits:?}"
        );
    }
}

/// MED-1 (84c, r35) + flat/chain rest-slot unification: the FLAT
/// `;`-terminated mixed+rest member-call spelling silent-emptied sg's
/// answer set — the classifier set Any for a rest slot but
/// `argument_template` independently derived Exactly(n) from the comma
/// count, and the pre-filter vetoed rest-expanded candidates before the
/// slots ran (subject [] vs sg {2,3,8}). sg 0.45.2 cell table (probed):
/// flat rest faces answer the same sources as the chained spelling; the
/// whole-list rest `$$$A` answers every arity INCLUDING the empty list
/// (`q9()`); a trailing rest sharing the list with an earlier slot must
/// bind at least one argument (`q9(1, $$$A)` refuses `q9(1)`); a
/// non-trailing rest may bind zero (`q9($$$A, 3)` answers `q9(3)`); a
/// `;`-terminated flat pattern binds statement-rooted calls only (the
/// chained lines' inner `q9` subnodes stay out with `;`, answer without).
/// Mutant cells: keep the text-derived Exactly(n) pre-filter (the 3-arg
/// cells flip silent); allow zero-length trailing shared rests (line 4
/// over-answers); drop the `;`-statement gate (lines 5/11 leak into the
/// semi cells); require a literal in a whole-rest list (`$$$A` flips []).
#[test]
fn f85a_php_flat_and_chain_rest_slot_arity_sg_exact() {
    let flat = "<?php\n$w->q9(1, 2, 3);\n$w->q9(1, 2);\n$w->q9(1);\n$w->q9(1, 2, 3)->r9();\n$o->q9(1, 2, 3);\n$w->q8(7, 8);\n$w->q9(1, 2, 3, 4);\n$w->q9();\n$w->q9(3);\n$w->q9(1)->r9();\n";
    for (pattern, want) in [
        // Flat mixed+rest: the MED-1 cells (sg {2,3,5,8} no-semi — the
        // chained line 5's inner q9 subnode answers embedded; {2,3,8} with
        // `;` — statement roots only).
        ("$w->q9(1, $$$A)", vec![2u32, 3, 5, 8]),
        ("$w->q9(1, $$$A);", vec![2u32, 3, 8]),
        // Whole-list rest: every arity incl. the empty list (sg: the q9
        // lines {2,3,4,8} plus the empty-arg line 9 and `q9(3)` line 10).
        ("$w->q9($$$A);", vec![2u32, 3, 4, 8, 9, 10]),
        // Mid rest between literals (zero-length mid stays registered: sg {2}).
        ("$w->q9(1, $$$A, 3);", vec![2u32]),
        // Leading rest before a literal, zero-length included (sg {2,10}).
        ("$w->q9($$$A, 3);", vec![2u32, 10]),
        // Chained spelling unchanged by the flat repair (sg {5} both).
        ("$w->q9(1, $$$A)->r9();", vec![5u32]),
        ("$w->q9(1, $$$A)->r9()", vec![5u32]),
        // Receiver/callee literal mismatches stay out (probed sg: the `$o`
        // pattern answers its own line 6 {6}; no `$zz` receiver site exists
        // [] and the q8 callee mismatch []).
        ("$o->q9(1, $$$A);", vec![6u32]),
        ("$zz->q9(1, $$$A);", vec![]),
        ("$w->q8(1, $$$A);", vec![]),
    ] {
        let hits = match_pattern(Language::Php, flat, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the flat/chain rest fixture (MED-1): {hits:?}"
        );
    }
    // Captures: the flat rest binds the remaining arguments' source bytes
    // under the multi namespace key (H-CONF-026: `$$$NAME` keys `$$$NAME`).
    let caps = match_pattern(Language::Php, flat, "$w->q9(1, $$$A);").unwrap();
    let two = caps.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(two.captures.get("$$$A").map(String::as_str), Some("2, 3"));
    let three = caps.iter().find(|h| h.line_start == 3).expect("line 3");
    assert_eq!(three.captures.get("$$$A").map(String::as_str), Some("2"));
}

/// FB-84a-04 (r34, silent miss): plain-call argument lists carrying a
/// `$$$rest` slot inside an assignment RHS silent-emptied — the ArgSlot
/// machinery covered only the member-call lanes, and the raw `$$$` text
/// never decomposed under the strict child zip. sg 0.45.2 binds the rest
/// (probed): a trailing shared rest needs at least one argument
/// (`f($v, $$$A)` refuses `f($v)`), the whole-list rest answers every
/// arity including empty, and a leading rest binds zero before its
/// literal. Mutant cells: drop the arguments-node rest arm (every cell
/// flips []); allow zero-length trailing shared rests (line 6 over-answers
/// in the first two cells); bind the whole-list rest only non-empty (line
/// 6 drops out of the `$$$A` cell).
#[test]
fn f85a_php_plain_call_rest_slots_bind_sg_exact() {
    let plain = "<?php\n$b = f($v, 1);\n$b = f(1, $v);\n$b = f($v, 1, 2);\n$b = f($v, $u);\n$b = f($v);\n$b = f(1, 2);\n$b = f($v, 1, 2, 3);\n$b = g($v, 1);\n";
    for (pattern, want) in [
        ("$b = f($v, $$$A);", vec![2u32, 4, 5, 8]),
        ("$b = f($A, $$$B);", vec![2u32, 3, 4, 5, 7, 8]),
        ("$b = f($$$A);", vec![2u32, 3, 4, 5, 6, 7, 8]),
        ("$b = f($$$A, 3);", vec![8u32]),
        // LHS mismatch stays out (sg [] — the g line answers only its own
        // callee, probed).
        ("$c = f($A, $$$B);", vec![]),
    ] {
        let hits = match_pattern(Language::Php, plain, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the plain-call rest fixture (FB-84a-04): {hits:?}"
        );
    }
    // Captures: the trailing rest binds the remaining arguments' bytes; the
    // whole-list rest binds the entire list content including empty.
    let caps = match_pattern(Language::Php, plain, "$b = f($v, $$$A);").unwrap();
    let two = caps.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(two.captures.get("$$$A").map(String::as_str), Some("1"));
    let five = caps.iter().find(|h| h.line_start == 5).expect("line 5");
    assert_eq!(five.captures.get("$$$A").map(String::as_str), Some("$u"));
    let whole = match_pattern(Language::Php, plain, "$b = f($$$A);").unwrap();
    let six = whole.iter().find(|h| h.line_start == 6).expect("line 6");
    assert_eq!(six.captures.get("$$$A").map(String::as_str), Some("$v"));
}

/// FB-82a-03 (MED, silent FALSE-POSITIVE): a `;`-terminated assignment or
/// binary pattern binds STATEMENT-level nodes only — sg's `;`-rooted
/// pattern never answers an assignment embedded in an `if`/`while`
/// condition or a call argument, while the `;`-less spelling answers the
/// same embedded nodes (probed sg {3,4} / {5,6} on the embedded lines).
/// The r31 walker bound ANY assignment_expression node and answered the
/// if-condition lines. In-block statements (`function`/`if` bodies) stay
/// statement-level for both spellings. Mutant cells (post-fix): drop the
/// statement-root gate (the `$eta = $V;` cell re-answers the condition
/// lines); apply the gate to the `;`-less spelling too (the `$eta = $V`
/// cell loses the condition lines).
#[test]
fn f83a_php_assignment_binds_statement_roots_only() {
    let b3 = "<?php\n$eta = $v;\nif ($eta = $v) { g(1); }\nwhile ($eta = f($v)) { g(2); }\nif ($alpha = $v) { g(9); }\nfoo($alpha = $v);\n$alpha = $v;\nfunction fb() { $alpha = $v; }\nif (true) { $alpha = $v; }\n$gamma == $v;\nif ($gamma == $v) { h(1); }\n";
    for (pattern, want) in [
        // `;`-terminated: only the statement line (sg {2}; the embedded
        // condition lines 3/4 stay silent-[]).
        ("$eta = $V;", vec![2u32]),
        // `;`-terminated: the statement lines only — top-level 7, in-block
        // 8/9; the condition line 5 and call-arg line 6 stay out (sg).
        ("$alpha = $V;", vec![7u32, 8, 9]),
        // `;`-less: the same embedded nodes DO answer (sg {2,3,4}).
        ("$eta = $V", vec![2u32, 3, 4]),
        ("$alpha = $V", vec![5u32, 6, 7, 8, 9]),
        // Binary faces carry the same statement-root rule (sg {10} with `;`,
        // {10,11} without).
        ("$gamma == $V;", vec![10u32]),
        ("$gamma == $V", vec![10u32, 11]),
    ] {
        let hits = match_pattern(Language::Php, b3, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the statement-root fixture (FB-82a-03): {hits:?}"
        );
    }
}

/// FB-82a-04 (MED, silent): member-call argument lists that MIX literal
/// tokens with canonical metas bind per-position — literal pattern args
/// match the candidate argument text byte-exactly, metas bind their
/// positional argument, and `$$$NAME` rest slots bind the REMAINING
/// arguments' source text (leading, mid, or trailing; zero-length mid-rest
/// matches too). The r31 arg_metas admission covered meta-only lists and
/// fell every mixed list to a silent refusal. Registered meta-only
/// contracts stay byte-compatible: whole-list `$A` keeps binding the list
/// content, `$A, $B` keeps positional binding, the same-name veto holds on
/// equal args, and the dotted java lane is untouched. Mutant cells
/// (post-fix): match literals position-INSENSITIVELY (`$w->q5($A, 1)`
/// over-answers line 8); drop rest binding (the `$$$A` cells flip silent);
/// bind rest including the literal prefix (the `A=` pins flip).
#[test]
fn f83a_member_chain_arg_slots_bind_mixed_lists() {
    let b4 = "<?php\n$w->q9(1, 2, 3)->r9();\n$w->q8(7, 8)->r8();\n$w->q7s('a', 'b')->r7s();\n$w->q0(1, 3)->r0();\n$w->q1($u)->r1();\n$w->q2($u, $v)->r2();\n$w->q5(1, $v)->r5();\n$w->q6(2, 2)->r6();\n";
    for (pattern, want) in [
        // Rest slots: trailing rest after a literal, leading rest before a
        // literal, mid rest between literals, zero-length mid rest (sg {2};
        // the q0 zero-arg row is sg {5}).
        ("$w->q9(1, $$$A)->r9()", vec![2u32]),
        ("$w->q9($$$A, 3)->r9()", vec![2u32]),
        ("$w->q9(1, $B, 3)->r9()", vec![2u32]),
        ("$w->q0(1, $$$A, 3)->r0()", vec![5u32]),
        ("$w->q0($$$A)->r0()", vec![5u32]),
        // Mixed literal/meta lists in chains (sg {3}/{4}/{8}).
        ("$w->q8(7, $B)->r8()", vec![3u32]),
        ("$w->q8($A, 8)->r8()", vec![3u32]),
        ("$w->q7s('a', $B)->r7s()", vec![4u32]),
        ("$w->q5(1, $B)->r5()", vec![8u32]),
        ("$w->q5($A, $v)->r5()", vec![8u32]),
        // The FLAT member-call spelling carries the same per-position
        // contract (sg {3}/{8}).
        ("$w->q8(7, $B)", vec![3u32]),
        ("$w->q5(1, $B)", vec![8u32]),
        // Refusal control: the literal position mismatches (sg []).
        ("$w->q5($A, 1)->r5()", vec![]),
        // Registered meta-only contracts stay byte-compatible.
        ("$w->q6($A, $A)->r6()", vec![9u32]),
        ("$w->q1($A)->r1()", vec![6u32]),
        ("$w->q2($A, $B)->r2()", vec![7u32]),
        ("$w->q1($A)", vec![6u32]),
    ] {
        let hits = match_pattern(Language::Php, b4, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the arg-slot fixture (FB-82a-04): {hits:?}"
        );
    }
    // Captures: rest slots bind the remaining arguments' source text
    // (sg -r pins `QQ(2, 3)` and `QQ(1, 2)`); mixed meta slots bind their
    // positional argument.
    let trailing = match_pattern(Language::Php, b4, "$w->q9(1, $$$A)->r9()").unwrap();
    let two = trailing.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(two.captures.get("$$$A").map(String::as_str), Some("2, 3"));
    let leading = match_pattern(Language::Php, b4, "$w->q9($$$A, 3)->r9()").unwrap();
    let two = leading.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(two.captures.get("$$$A").map(String::as_str), Some("1, 2"));
    let mid = match_pattern(Language::Php, b4, "$w->q9(1, $B, 3)->r9()").unwrap();
    let two = mid.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(two.captures.get("B").map(String::as_str), Some("2"));
    let mixed = match_pattern(Language::Php, b4, "$w->q5(1, $B)->r5()").unwrap();
    let eight = mixed.iter().find(|h| h.line_start == 8).expect("line 8");
    assert_eq!(eight.captures.get("B").map(String::as_str), Some("$v"));
    // Registered whole-list and positional bindings keep their bytes.
    let whole = match_pattern(Language::Php, b4, "$w->q0($$$A)->r0()").unwrap();
    let five = whole.iter().find(|h| h.line_start == 5).expect("line 5");
    assert_eq!(five.captures.get("$$$A").map(String::as_str), Some("1, 3"));
    let single = match_pattern(Language::Php, b4, "$w->q1($A)->r1()").unwrap();
    let six = single.iter().find(|h| h.line_start == 6).expect("line 6");
    assert_eq!(six.captures.get("A").map(String::as_str), Some("$u"));
    let positional = match_pattern(Language::Php, b4, "$w->q2($A, $B)->r2()").unwrap();
    let seven = positional
        .iter()
        .find(|h| h.line_start == 7)
        .expect("line 7");
    assert_eq!(seven.captures.get("A").map(String::as_str), Some("$u"));
    assert_eq!(seven.captures.get("B").map(String::as_str), Some("$v"));
    // Dotted java regression control (served lane untouched, sg {3}).
    let jch = "class J {\n    void go() {\n        r.m1(1, 2).m2();\n    }\n}\n";
    let java = match_pattern(Language::Java, jch, "r.m1(1, $B).m2()").unwrap();
    assert_eq!(klass_lines(&java), vec![3u32]);
    assert_eq!(
        java[0].captures.get("B").map(String::as_str),
        Some("2"),
        "the dotted mixed list keeps its positional binding"
    );
}

/// FB-82a-05 (LOW, silent): a pattern ending in a DANGLING plain arrow
/// (`$o->c1($A)->`) is sg's lenient parse of the flat member-call prefix —
/// sg 0.45.2 repairs the ERROR node to `member_call_expression($o->c1($A))`
/// (debug-query probe) and answers every flat prefix site. The r31
/// classifier refused the empty final segment and fell silent. Controls:
/// a dangling NULLSAFE arrow stays refused (sg []), a nullsafe-head
/// pattern with a dangling plain arrow stays refused (sg []), a garbage
/// `->$` tail stays refused (sg []), and the chain+dangling face
/// `$o->c1($A)->tailProp->` keeps its registered refusal — sg answers it
/// {5} through ERROR-object alignment (rewrites the depth+1 continuation
/// call), a parse-recovery shape no sound structural rule reproduces
/// without over-matching the property-tail lines; registered residual.
/// Mutant cells (post-fix): strip `?->` too (the nullsafe controls
/// over-answer); drop the strip (the prefix face flips silent []).
#[test]
fn f83a_dangling_arrow_repairs_to_flat_prefix() {
    let b5 = "<?php\n$o->c1($u)->tailProp;\n$o->c1($a, $b, $c)->tailProp;\n$o->c1($u)->p1->p2->tailProp;\n$o->c1($u)->tailProp->c2($w)->c3();\n$o->c1($u)->tailProp($w);\n$o->c1($u)->$tail;\n$o->c1($u)->$tail->c2($w);\nFoo::bar($u)->tailProp;\n\\App\\Models\\User::find($u)->hydrate($u)->tailProp;\n$o?->c1($u)?->tailProp;\n$o->c1($u)->prop?->tailProp;\n$o->c1($u);\n$o->c1($u, $v);\n";
    for (pattern, want) in [
        // The FB-82a-05 face: the flat prefix answer set (sg {2,4..8,12}).
        ("$o->c1($A)->", vec![2u32, 4, 5, 6, 7, 8, 12]),
        // Dangling nullsafe arrow: sg refuses (rc=0 []).
        ("$o->c1($A)?->", vec![]),
        // Nullsafe head + dangling plain arrow: sg refuses ([]).
        ("$o?->c1($A)->", vec![]),
        // Garbage `->$` tail: refused ([]).
        ("$o->c1($A)->$", vec![]),
        // Chain+dangling: registered residual (sg answers {5} via ERROR
        // recovery; the sound subset keeps the refusal).
        ("$o->c1($A)->tailProp->", vec![]),
    ] {
        let hits = match_pattern(Language::Php, b5, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the dangling-arrow fixture (FB-82a-05): {hits:?}"
        );
    }
    let prefix = match_pattern(Language::Php, b5, "$o->c1($A)->").unwrap();
    let two = prefix.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(
        two.captures.get("A").map(String::as_str),
        Some("$u"),
        "the repaired flat prefix binds its argument"
    );
}

/// FB-82a-06 (LOW, rewrite TEXT divergence): a `;`-terminated php
/// assignment pattern matches the `;`-rooted statement span — sg -U
/// CONSUMES the trailing `;` (its match node is the expression statement),
/// so the planned span must include it; the `;`-less spelling keeps the
/// registered assignment-node span that preserves the `;` (byte parity,
/// the C1 control). Mutant cells (post-fix): span the statement for the
/// `;`-less pattern too (byte_end 17 flips to 18); keep the r31
/// assignment-node span for the `;`-terminated pattern (byte_end 18 flips
/// to 17).
#[test]
fn f83a_php_assignment_semicolon_span_consumed() {
    let b6 = "<?php\n$alpha = $v;\n";
    let semi = match_pattern(Language::Php, b6, "$alpha = $V;").unwrap();
    assert_eq!(semi.len(), 1);
    assert_eq!(
        (semi[0].byte_start, semi[0].byte_end),
        (6, 18),
        "the ;-terminated pattern's span consumes the trailing ; (sg -U parity)"
    );
    assert_eq!(
        semi[0].captures.get("MATCH").map(String::as_str),
        Some("$alpha = $v;")
    );
    assert_eq!(semi[0].captures.get("V").map(String::as_str), Some("$v"));
    let bare = match_pattern(Language::Php, b6, "$alpha = $V").unwrap();
    assert_eq!(bare.len(), 1);
    assert_eq!(
        (bare[0].byte_start, bare[0].byte_end),
        (6, 17),
        "the ;-less pattern keeps the assignment-node span (; preserved, sg parity)"
    );
    assert_eq!(
        bare[0].captures.get("MATCH").map(String::as_str),
        Some("$alpha = $v")
    );
}

/// 86a-M3 + 86c-L (r36→r37, silent FP): the r34 doubled-sign refusal was
/// END-anchored (rhs starts/ends `--`/`++`), so a TIGHT sign in mid-RHS or
/// paren-wrapped position passed the gate and the subject's own grammar
/// answered the postfix sites where sg 0.45.2 parse-refuses the face
/// (probed sg [] rc1 against byte-compatible source lines): `$a = $V-- + 1;`
/// answered {2}, `$b = $V++ - 2;` {4}, `$i = ($V--) + 1;` {14},
/// `$j = ($V--);` {15}. The refusal must be a WHOLE-RHS TIGHT-token scan:
/// any `--`/`++` tight to an operand on either side refuses, wherever it
/// sits. Tightness-exact, not substring-exact — the SPACED spellings are
/// sg-answering lenient parses both engines share and MUST keep answering
/// (probed sg: `$a = 1 -- $V;` {3}, `$e = 3 -- $V;` {9}; the registered
/// `$x = $y -- $z;` control; meta-left spaced faces `$d = $V -- $W;` and
/// `$f = $V -- 4;` sg-refuse and the subject grammar self-refuses them []
/// — pinned). Start/end anchored cells from the r34 gate keep refusing, and
/// the single-sign controls stay sg-exact ({12}/{13}).
/// Mutant cells (post-fix): drop the scan (the four refusal faces re-answer
/// their source lines); widen it to every doubled sign regardless of
/// tightness (the spaced `{3}`/`{9}` cells and the registered
/// `$x = $y -- $z;` control flip []); key the scan on the rhs ends only
/// (the mid-RHS `{2}`/`{4}` and paren `{14}`/`{15}` faces re-answer).
#[test]
fn f87b_php_doubled_sign_refusal_is_whole_rhs_tight_token_scan() {
    let m3 = "<?php\n$a = $p-- + 1;\n$a = 1 -- $p;\n$b = $q++ - 2;\n$b = 2 + $q++;\n$c = --$r + 3;\n$d = $s -- $t;\n$d = $u -- $v;\n$e = 3 -- $w;\n$f = $x1 -- 4;\n$g = $y1 ++ $z1;\n$h = -$m;\n$h = +$n;\n$i = ($o1--) + 1;\n$j = ($p1--);\n";
    for (pattern, want) in [
        // RED pre-fix: tight postfix mid-RHS / paren-wrapped faces answered
        // their source lines where sg parse-refuses (probed sg [] rc1).
        ("$a = $V-- + 1;", vec![]),
        ("$b = $V++ - 2;", vec![]),
        ("$i = ($V--) + 1;", vec![]),
        ("$j = ($V--);", vec![]),
        // r34 end-anchored cells keep refusing (tight prefix/tight final).
        ("$c = --$V + 3;", vec![]),
        ("$b = 2 + $V++;", vec![]),
        // Spaced-sign lenient parses: sg ANSWERS the literal-left faces
        // (probed {3}/{9}) — a substring-blanket refusal would flip them.
        ("$a = 1 -- $V;", vec![3u32]),
        ("$e = 3 -- $V;", vec![9u32]),
        // Meta-left spaced faces: sg refuses (probed [] rc1; the no-semi
        // spelling is sg's ERROR-node lenient-empty); the subject grammar
        // self-refuses them — pinned so a widened scan cannot hide behind
        // them nor over-answer them.
        ("$d = $V -- $W;", vec![]),
        ("$d = $V -- $W", vec![]),
        ("$f = $V -- 4;", vec![]),
        ("$g = $V ++ $W;", vec![]),
        // Single-sign controls stay sg-exact.
        ("$h = -$V;", vec![12u32]),
        ("$h = +$V;", vec![13u32]),
    ] {
        let hits = match_pattern(Language::Php, m3, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.3 line set on the tight-sign fixture (86a-M3): {hits:?}"
        );
    }
}

/// 86a-M4 (r36→r37, silent FP): bare `&&` and `.` as WHOLE patterns answered
/// their operator sites through the literal lane (probed subject {2}/{5},
/// silent rc0) where sg 0.45.3 answers NOTHING for php (`&&` [] rc0 with an
/// ERROR-node warning — the `->`/`::` lenient-empty class; `.` [] rc1 — the
/// parse-refuse class; answer sets agree, the registered silent-vs-loud
/// genus). The carve is PHP-scoped: sg ANSWERS bare `.` member/attr sites
/// in javascript/python/rust/bash and bare `&&` sites in javascript/bash
/// (probed), so a cross-language carve would over-refuse. The rest of the
/// garbage sweep pins its own sg classes: `||`/`+`/`=>` answer their sites
/// (probed {3}/{4}/{6}), `)`/`...`/`???` agree-empty, `->`/`::` stay on the
/// registered §27.5 carve.
/// Mutant cells (post-fix): drop the php carve (`&&` re-answers {2}, `.`
/// re-answers {5}); make it cross-language (a non-php `.`/`&&` face
/// over-refuses — pinned by the js/bash probe classes in the report);
/// extend it to `||`/`+`/`=>` (those answering cells flip []).
#[test]
fn f87b_php_bare_token_garbage_classes_sg_exact() {
    let garb = "<?php\n$a = 1 && 2;\n$a = 1 || 2;\n$a = 1 + 2;\n$a = 1 . 2;\n$t = [\"k\" => 1];\n$x = $y -> $z;\n$s = A::B;\necho \"x => y\";\n";
    for (pattern, want) in [
        // RED pre-fix: the literal lane matched the token at its sites.
        ("&&", vec![]),
        (".", vec![]),
        // sg-answering classes keep their lanes (probed agreeing).
        ("||", vec![3u32]),
        ("+", vec![4u32]),
        ("=>", vec![6u32]),
        // Agree-empty classes (probed sg [] for each).
        (")", vec![]),
        ("...", vec![]),
        ("???", vec![]),
        // The registered §27.5 carve cells stay silent-empty.
        ("->", vec![]),
        ("::", vec![]),
    ] {
        let hits = match_pattern(Language::Php, garb, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.3 line set on the garbage fixture (86a-M4): {hits:?}"
        );
    }
    // The carve is PHP-scoped: sg ANSWERS bare `.`/`&&` sites in other
    // grammars (probed sg 0.45.3: javascript `.` {1} member site, `&&` {2}
    // site; python/rust `.` {1}; bash `.` {1} source site, `&&` {2}) — a
    // cross-language carve mutant flips these cells to [].
    let js = "let q = a.b;\nlet r = x && y;\n";
    for (pattern, want) in [(".", vec![1u32]), ("&&", vec![2u32])] {
        let hits = match_pattern(Language::JavaScript, js, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.3 line set on the js garbage fixture (86a-M4 scope): {hits:?}"
        );
    }
}

/// 86a-L5 (r36→r37, silent; §27.8-2 residual ACTIVATED): meta-only flat
/// member-call argument lists silent-[] where sg 0.45.3 answers the
/// statement-root sites (probed): `$o->c1($A);` {2}, `$o?->c1($A);` {3},
/// `$o->c1($A, $B);` {4,11}, `$o?->c1($A, $B);` {10}, receiver/callee
/// spelling-exact controls `$p->c1($A);` {8} / `$o->c9($A);` {9}, and the
/// SAME-NAME list `$o->c1($A, $A);` answers ONLY the equal-args line {11}
/// (sg bind semantics: unequal args refuse, equal re-affirm — the slot
/// path must veto the (2, 3) line). The `;`-terminated spelling died in the
/// pre-gate carve's meta-only floor (routed to the post-gate arm, which
/// refuses the `;` tail → gate NeverMatches). Whole-list rests
/// (`$$$A`, both connectors) keep their r35 MED-1 cells, no-semi spellings
/// keep the F74a-1 embedded-subnode discipline ({2,7}), empty args keep
/// their own agreeing arm ({5}), and the chain control stays {7}.
/// Mutant cells (post-fix): restore the meta-only floor (every RED face
/// flips silent [] again); bind same-name slots without the conflict veto
/// (`$o->c1($A, $A);` over-answers {4}); drop the `;` statement gate (the
/// semi cells absorb line 7's chained-embedded c1); widen the carve to
/// empty/`$$$` lists (their registered arms move).
#[test]
fn f87b_php_meta_only_flat_member_calls_answer_sg_exact() {
    let l5 = "<?php\n$o->c1(2);\n$o?->c1(9);\n$o->c1(2, 3);\n$o->c1();\n$o->c1(2, 3, 4);\n$o->c1(2)->t;\n$p->c1(5);\n$o->c9(7);\n$o?->c1(8, 9);\n$o->c1(5, 5);\n";
    for (pattern, want) in [
        // RED pre-fix: silent [] where sg answers the statement roots.
        ("$o->c1($A);", vec![2u32]),
        ("$o?->c1($A);", vec![3u32]),
        // Distinct metas bind independently (probed sg {4,11} on the final
        // fixture — line 11's equal args are two independent binds).
        ("$o->c1($A, $B);", vec![4u32, 11]),
        ("$o?->c1($A, $B);", vec![10u32]),
        // Same-name metas: only the equal-args line (conflict veto).
        ("$o->c1($A, $A);", vec![11u32]),
        // Receiver/callee token-exactness (probed sg {8}/{9}).
        ("$p->c1($A);", vec![8u32]),
        ("$o->c9($A);", vec![9u32]),
        // Whole-list rests keep their registered MED-1 cells (both
        // connectors; semi = statement-rooted only, line 7 excluded).
        ("$o->c1($$$A);", vec![2u32, 4, 5, 6, 11]),
        ("$o?->c1($$$A);", vec![3u32, 10]),
        // No-semi spellings keep the F74a-1 embedded discipline.
        ("$o->c1($A)", vec![2u32, 7]),
        ("$o->c1($A, $B)", vec![4u32, 11]),
        // Empty args and the chain control keep their own agreeing arms.
        ("$o->c1();", vec![5u32]),
        ("$o->c1($A)->t;", vec![7u32]),
    ] {
        let hits = match_pattern(Language::Php, l5, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.3 line set on the meta-only fixture (86a-L5): {hits:?}"
        );
    }
    // Capture: the meta binds the argument text (sg -r parity).
    let hits = match_pattern(Language::Php, l5, "$o->c1($A);").unwrap();
    let two = hits.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(two.captures.get("A").map(String::as_str), Some("2"));
}

/// 88a-M1 (r38→r39, silent miss): the whole-RHS doubled-sign tightness scan
/// was COMMENT-BLIND — `--`/`++` inside a pattern-side `/* */` block comment
/// with non-space byte neighbors was read as a tight doubled sign and the
/// assignment hook refused sg-ANSWERING faces (probed vendored sg 0.45.2,
/// ATTACHED, 2026-09-08): `$a = $V /*--*/ + 1;` {3}, `$a = $V + /*++*/ 1;`
/// {4}, the paren form `$a = ($V /*--*/ + 1);` {5}, tight BETWEEN LETTERS
/// `$a = $V /* x--x */ + 1;` {6}, triple `$a = $V /* --- */ + 1;` {12},
/// `$a = $V /*++*/ - 1;` {13}, and a doubled pair per comment
/// `$a = $V /* a--b--c */ + 1;` {15}. Comment CONTENT is not an operand —
/// §28.3's registered class is a `--`/`++` token tight to an OPERAND.
/// Controls that must NOT move: the spaced-in-comment face
/// `$a = $V /* -- */ + 1;` {7}, single-sign `$a = $V /*-*/ + 1;` {11},
/// the r37 tight/refuse
/// matrix (`$a = $V-- + 1;` [], the REAL tight operator beside a comment
/// `$a = $V -- /* x */ 1;` [] — sg rc1), the string-literal face
/// `$a = "x--y";` {10}, and the spaced/paren answer faces. `//`/`#`
/// comment kinds are NOT comment-transparent here: sg REFUSES those
/// patterns outright (rc8 probed for every trailing spelling), so their
/// sg-agreed empty class rides the unskipped scan bytes.
/// Mutant tooth (post-fix): drop the block-comment skip and every face
/// above with a doubled sign inside `/* */` flips back to [].
#[test]
fn f89b_php_rhs_tight_sign_scan_skips_pattern_side_block_comments() {
    let m1 = "<?php\n$a = $v + 1;\n$a = $v /*--*/ + 1;\n$a = $v + /*++*/ 1;\n$a = ($v /*--*/ + 1);\n$a = $v /* x--x */ + 1;\n$a = $v /* -- */ + 1;\n$a = 1 -- $v;\n$a = $v-- + 1;\n$a = \"x--y\";\n$a = $v /*-*/ + 1;\n$a = $v /* --- */ + 1;\n$a = $v /*++*/ - 1;\n$a = $v -- /* x */ 1;\n$a = $v /* a--b--c */ + 1;\n$a = ($v + 1);\n";
    for (pattern, want) in [
        // RED pre-fix: the scan read comment bytes as a tight doubled sign.
        ("$a = $V /*--*/ + 1;", vec![3u32]),
        ("$a = $V + /*++*/ 1;", vec![4u32]),
        ("$a = ($V /*--*/ + 1);", vec![5u32]),
        ("$a = $V /* x--x */ + 1;", vec![6u32]),
        ("$a = $V /* --- */ + 1;", vec![12u32]),
        ("$a = $V /*++*/ - 1;", vec![13u32]),
        ("$a = $V /* a--b--c */ + 1;", vec![15u32]),
        // Controls (green pre-fix): the comment-FREE pattern's
        // candidate-comment transparency (sg-probed — it holds even for
        // `--`-carrying comments, and `$V` binds the whole update node on
        // line 9), the r37 tight/refuse matrix, string literals, and the
        // answer faces.
        ("$a = $V + 1;", vec![2u32, 3, 4, 6, 7, 9, 11, 12, 15]),
        ("$a = ($V + 1);", vec![5u32, 16]),
        ("$a = $V /* -- */ + 1;", vec![7u32]),
        ("$a = 1 -- $V;", vec![8u32]),
        ("$a = $V-- + 1;", vec![]),
        ("$a = \"x--y\";", vec![10u32]),
        ("$a = $V /*-*/ + 1;", vec![11u32]),
        ("$a = $V -- /* x */ 1;", vec![]),
    ] {
        let hits = match_pattern(Language::Php, m1, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the comment-tight fixture (88a-M1): {hits:?}"
        );
    }
}

/// 88a-M2 (r38→r39, loud): a bare php binary-expression META template — no
/// `=` assignment root, no literal LHS — classified None and failed closed
/// LOUDLY at the census while sg 0.45.2 ANSWERS the whole probed family
/// through expression-level metavariable binds (ATTACHED, 2026-09-08):
/// `$A && $B` {2,6,7,8,9,11,13} (flat, nested-left-assoc outer, call
/// operands, paren site, echo-embedded, bare statement), `$A . $B` {3,12},
/// `$A + $B` {4}, `$A == $B` {5}, `$A || $B` {10}, `$A === $B` {2,5,6},
/// `$A ?? $B`, `$A <=> $B`, the nested `$A && $B && $C` {6} (root-only),
/// the paren spelling `($A && $B)` {9} (kind-exact — only paren-wrapped
/// sites), the `;`-terminated statement-root spellings (`$A && $B;` {13},
/// `$A === $B;` {6} — bare statements only; `$A . $B;`/`$A == $B;` []
/// because no bare statement carries them), over every probed binary
/// operator. The universal 2-dollar spelling
/// `$$A && $B` (sg answers the full `$A && $B` set) and LITERAL-operand
/// faces (`$A === 2` sg {2,5,6}; `$A + 1` the §21.2 registered LOUD cell)
/// stay OUTSIDE the lane — the former keeps its registered variable-only
/// wildcard, the latter its registered loud class; reconcile-register both.
/// The lane must root the walk at
/// the pattern's own comparison node: `binary_expression` /
/// `parenthesized_expression` (no-semi) or the wrapping
/// `expression_statement` (semi). Mixed-case `$A && $b` keeps the
/// registered NeverMatches silent class (sg rc1 []).
/// Riders that must NOT move (probed sg-answering; outside the deduped
/// rootless-binary mechanism — reconcile-register): the assignment-rooted
/// `$A = $B` (sg answers every assignment line) and the bare-meta statement
/// `$A;` stay loud (native_pattern_answerable false) — their roots are not
/// binary expressions.
/// Mutant tooth (post-fix): gate the lane off and every RED face above
/// returns [] again; admit assignment roots and the `$A = $B` rider flips
/// answerable.
#[test]
fn f89b_php_bare_binary_meta_operand_templates_answer_sg_sets() {
    let m2 = "<?php\n$a = 1 && 2;\n$a = 1 . 2;\n$a = 1 + 2;\n$a = 1 == 2;\n$b = 1 && 2 && 3;\n$c = f(1) && g(2);\n$d = 1 && 2;\n$e = ($x && $y);\n$f = 1 || 2;\necho 1 && 2;\n$g = \"s\" . \"t\";\nh(1) && i(2);\n";
    for (pattern, want) in [
        // RED pre-fix: classified None, failed closed loudly (rc2 through
        // the pipeline; match_pattern answered []).
        ("$A && $B", vec![2u32, 6, 7, 8, 9, 11, 13]),
        ("$A . $B", vec![3u32, 12]),
        ("$A + $B", vec![4u32]),
        ("$A == $B", vec![5u32]),
        ("$A || $B", vec![10u32]),
        ("$A && $B && $C", vec![6u32]),
        ("($A && $B)", vec![9u32]),
        // Rider (probed sg answers the full `$A && $B` set for the
        // universal `$$A` spelling): 2-dollar dynamic tokens keep their
        // registered variable-only wildcard outside this lane — loud at
        // the pipeline level (answerable false below). Reconcile-register.
        ("$$A && $B", vec![]),
        // Semi spellings: statement-rooted only (probed).
        ("$A && $B;", vec![13u32]),
        ("$A . $B;", vec![]),
        ("$A == $B;", vec![]),
        // Mixed-case meta: registered NeverMatches silent class (sg rc1 []).
        ("$A && $b", vec![]),
    ] {
        let hits = match_pattern(Language::Php, m2, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the operand-template fixture (88a-M2): {hits:?}"
        );
    }
    // All-meta faces over every probed operator (sg 0.45.2, ATTACHED), and
    // the semi spelling roots at the bare statement line 6.
    let ops = "<?php\n$a = 1 === 2;\n$b = 1 ?? 2;\n$c = 1 <=> 2;\n$d = 1 === 2;\n1 === 2;\n";
    for (pattern, want) in [
        ("$A === $B", vec![2u32, 5, 6]),
        ("$A ?? $B", vec![3u32]),
        ("$A <=> $B", vec![4u32]),
        ("$A === $B;", vec![6u32]),
    ] {
        let hits = match_pattern(Language::Php, ops, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the ops fixture (88a-M2): {hits:?}"
        );
    }
    // Literal-operand faces stay OUTSIDE the lane: `$A + 1` is the §21.2
    // registered LOUD cell (f74c pin — match_pattern answers nothing,
    // answerable false), and the probed sg-answering literal faces
    // (`$A === 2` {2,5,6}) keep the subject-stricter rider class
    // (reconcile-register). The all-meta faces above are unaffected.
    for (pattern, want) in [
        ("$A === 2", Vec::<u32>::new()),
        ("$A == 1", Vec::<u32>::new()),
    ] {
        let hits = match_pattern(Language::Php, ops, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the ops fixture (88a-M2 rider): {hits:?}"
        );
        assert!(
            !native_pattern_answerable(Language::Php, pattern),
            "{pattern} keeps its registered loud class (88a-M2 scope guard)"
        );
    }
    // Answerability flips only for the deduped family: the binary faces
    // become per-file answerable (php files stop skipping), while the
    // bare-meta riders stay loud.
    assert!(native_pattern_answerable(Language::Php, "$A && $B"));
    assert!(native_pattern_answerable(Language::Php, "$A && $B;"));
    assert!(native_pattern_answerable(Language::Php, "($A && $B)"));
    // PASS 127 correction (125A-F5, oracle grid /tmp/phase127/g1): the
    // `$A = $B` row LEFT the unanswerable list — its premise was the §26.2
    // "sg refuses meta assignment-targets anywhere" reading, which the
    // fresh grid REFUTED for the `;`-less spellings (sg answers `$X = $Y` /
    // `$X = $V` / `$ALPHA = $V` n1 each with LHS+RHS bindings; only the
    // `;`-FUL spellings are sg-[]). The assignment hook now classifies it
    // (answerable via the classify_php_assignment arm) and the walk binds
    // sg-exactly (f127a). The `;`-ful class keeps the registered loud
    // contract.
    assert!(native_pattern_answerable(Language::Php, "$A = $B"));
    assert!(!native_pattern_answerable(Language::Php, "$A = $B;"));
    assert!(!native_pattern_answerable(Language::Php, "$A;"));
    assert!(!native_pattern_answerable(Language::Php, "$$A && $B"));
    assert!(!needs_ast_grep_fallback("$A && $B"));
    // Capture parity (sg -r): A binds the left operand, B the right.
    let hits = match_pattern(Language::Php, m2, "$A && $B").unwrap();
    let two = hits.iter().find(|h| h.line_start == 2).expect("line 2");
    assert_eq!(two.captures.get("A").map(String::as_str), Some("1"));
    assert_eq!(two.captures.get("B").map(String::as_str), Some("2"));
}

/// 88a-L3 (r38→r39, overmatch): a `;`-terminated expression meta template
/// on the general lane unwrapped its `expression_statement` root and
/// answered the expression at EVERY site, while sg 0.45.2 keeps a
/// pattern-trailing `;` significant — the pattern root is the STATEMENT and
/// only bare `expr;` statements answer (ATTACHED, 2026-09-08): `f($A);` {1},
/// `$A.foo();` {3}, `$M($A);` {1,6} (the argument binds the whole argument
/// expression), `$A && $B;` {7}, `$A.$B;` {8}, `$A => $B;` {1}, `$A;`
/// {1,3}, `$A + $B;` []. The `;`-less spellings keep the expression-level
/// sets (all AGREE pre-fix — controls). The rule is scoped to the plain
/// span-less builds (js/ts/py): the php `<?php `-wrapped builds keep their
/// registered byte behavior (rider: the semi spelling `Foo::bar($A)->baz();`
/// over-answers the nested line today, sg roots it at the statement {2} —
/// reconcile-register; the no-semi spelling AGREEs {2,3} and is pinned).
/// Gains under the rule (sg-verified answering, loud pre-fix): the bare
/// statement faces `$A;` {1,3} and `$A => $B;` {1}. The no-semi arrow face
/// `$X => $Y` is sg-answering {1,2} and stays loud — outside the deduped
/// mechanism, reconcile-register.
/// Mutant tooth (post-fix): drop the had_semi root stop and every RED face
/// re-absorbs its nested lines ({1,2,5,6}/{3,4}/{1,2,5,6}/…).
#[test]
fn f89b_had_semi_general_templates_root_at_expression_statement() {
    let l3 = "f(1);\nconst a = f(2);\nobj.foo();\nconst b = c.foo();\nif (f(3)) { work(); }\nbar(f(4));\n";
    for (pattern, want) in [
        // RED pre-fix: the expression root matched nested/condition/argument
        // call sites too.
        ("f($A);", vec![1u32]),
        ("$A.foo();", vec![3u32]),
        ("$M($A);", vec![1u32, 6]),
        // `;`-less controls (green pre-fix): expression-level sets.
        ("f($A)", vec![1u32, 2, 5, 6]),
        ("$A.foo()", vec![3u32, 4]),
    ] {
        let hits = match_pattern(Language::JavaScript, l3, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the semi-root fixture (88a-L3): {hits:?}"
        );
    }
    let l3b = "const a = b && c;\nconst d = e.f;\nif (a && d) { work(); }\nconst g = h && i && j;\nfoo(k.l);\nconst m = n && o;\nbare && stmt;\nsolo.t;\n";
    for (pattern, want) in [
        ("$A && $B;", vec![7u32]),
        ("$A.$B;", vec![8u32]),
        ("$A && $B", vec![1u32, 3, 4, 6, 7]),
        ("$A.$B", vec![2u32, 5, 8]),
    ] {
        let hits = match_pattern(Language::JavaScript, l3b, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the semi fixture (88a-L3): {hits:?}"
        );
    }
    // RED (overmatch half, probed on the ops line): the semi pattern
    // absorbed the nested binary site; sg roots the statement — none here.
    let l3d = "const d = e + 1;\n";
    let hits = match_pattern(Language::JavaScript, l3d, "$A + $B;").unwrap();
    assert_eq!(
        klass_lines(&hits),
        Vec::<u32>::new(),
        "$A + $B;: sg 0.45.2 line set on the arithmetic fixture (88a-L3): {hits:?}"
    );
    // Gains under the rule (sg-verified; loud pre-fix): the bare-statement
    // spellings whose root is not an expression kind ride the statement
    // root once the `;` is significant.
    let l3c = "x => y;\nconst a = p => q;\nobj;\nconst b = c;\n";
    for (pattern, want) in [("$A => $B;", vec![1u32]), ("$A;", vec![1u32, 3])] {
        let hits = match_pattern(Language::JavaScript, l3c, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the statement-gain fixture (88a-L3): {hits:?}"
        );
    }
    // The php wrapped builds keep their registered behavior: the no-semi
    // chain face AGREEs everywhere (probed {2,3}); its answerability is
    // untouched by the had_semi rule.
    let chain = "<?php\nFoo::bar(1)->baz();\n$x = Foo::bar(2)->baz();\n";
    let hits = match_pattern(Language::Php, chain, "Foo::bar($A)->baz()").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![2u32, 3],
        "no-semi php chain control: sg 0.45.2 line set (88a-L3 scope guard): {hits:?}"
    );
}

/// F-r40-2 (r41, 90B-89M2-1 HIGH): an all-meta php operand template carrying
/// an embedded assignment/augmented-assignment expression with a META target
/// must take the sibling assignment lane's `validate_no_meta_target` refusal
/// — sg 0.45.2 answers nothing on the whole face family (probed 2026-09-08,
/// ATTACHED: `($A = $B) && $C` rc1 [], `$A && ($B = $C)` rc0 [],
/// `($A .= $B) && $C` rc1 [], `($A += $B) && $C` rc1 [],
/// `$A && ($B += $C)` rc0 [] — the meta assignment target never answers),
/// while the operand lane's walk unified the kind-equal assignment nodes and
/// over-matched ordinary source (`($A = $B) && $C` answered {2,9} on the
/// fixture: the statement and the assignment-embedded spellings). The veto
/// is the EXACT sibling mechanism ([`validate_no_meta_target`] over the
/// parsed doc): the refused faces leave the lane, answerability goes false,
/// and the pipeline census takes the same loud fail-closed class as the
/// registered `$A = $B` rider (sg answers that one — subject-stricter genus;
/// here sg answers nothing, so refusal is exact on the answer sets).
/// Mutant tooth: skip the veto and every RED face re-overmatches ({2,9},
/// {4}, {6}, {7}, {8}); drop only the augmented arm of the veto and the
/// `+=`/`.=` faces flip back to {6}/{7}.
#[test]
fn f91b_php_meta_assignment_operand_templates_refuse_like_sg() {
    let f2 = "<?php\n($x = $y) && $z;\n$q = 1 && 2;\n$m = $a && ($b = $c);\n($x = $y) || $z;\n($x .= $y) && $z;\n($x += $y) && $z;\n$n = $a && ($b += $c);\n$o = ($x = $y) && $z;\n";
    for (pattern, want) in [
        // RED pre-fix: the walk unified the embedded assignment operands.
        ("($A = $B) && $C", vec![]),
        ("$A && ($B = $C)", vec![]),
        ("($A .= $B) && $C", vec![]),
        ("($A += $B) && $C", vec![]),
        ("$A && ($B += $C)", vec![]),
        // Control (green pre-fix): the plain all-meta binary keeps answering
        // every binary line (sg {2,3,4,6,7,8,9}).
        ("$A && $B", vec![2u32, 3, 4, 6, 7, 8, 9]),
    ] {
        let hits = match_pattern(Language::Php, f2, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the meta-operand fixture (F-r40-2): {hits:?}"
        );
    }
    // Refused faces leave the lane: answerability must go false so the
    // census takes the sibling lane's loud class (RED pre-fix: true).
    for pattern in [
        "($A = $B) && $C",
        "$A && ($B = $C)",
        "($A .= $B) && $C",
        "($A += $B) && $C",
        "$A && ($B += $C)",
    ] {
        assert!(
            !native_pattern_answerable(Language::Php, pattern),
            "{pattern}: meta assignment-target operand must be unanswerable (sibling veto class)"
        );
    }
    assert!(native_pattern_answerable(Language::Php, "$A && $B"));
}

/// F-r40-3 (r41, 90A-F5 MEDIUM, option (a)): a php assignment pattern whose
/// RHS HEAD carries a `/* */` block comment is sg-ANSWERING — sg aligns the
/// operator node's children positionally and demands the pattern comment's
/// TEXT-EXACT counterpart in the candidate at the same slot (CST probe:
/// `comment` is a direct child of `assignment_expression` between `=` and
/// `right`). The walk's blanket top-level-comment refusal silenced the whole
/// face family into `ok:true []` where sg answers. sg 0.45.2 probed
/// (2026-09-08, ATTACHED): `$a = /* h */ $V + 1;` {3} (self line only —
/// `$V`+literal `1` text discipline), `$a = /* h */ $V;` {3,4} (bare meta
/// binds the whole RHS node), `$b = /* h */ $V + 1;` {6}; negative controls:
/// `$a = /* nope */ $V + 1;` [] (text mismatch — sg rc1), and the
/// comment-free `$a = $V + 1;` {8,9} keeps the landed FB-84a-03 classes
/// (candidate head comments break a comment-free pattern's alignment; mid
/// candidate comments stay transparent; the mid pattern comment
/// `$a = $V /* h */ + 1;` {8,9} demands its text-exact counterpart).
/// Mutant tooth: drop the head-comment gate and the RED faces re-silence to
/// []; replace exact-text equality with any-comment acceptance and the
/// `/* nope */` negative control over-matches {3,4}.
#[test]
fn f91b_php_head_comment_assignment_faces_answer_sg_text_exact() {
    let f3 = "<?php\n$a = 1 + 2;\n$a = /* h */ $v + 1;\n$a = /* h */ $w + 2;\n$a = /* nope */ $u + 3;\n$b = /* h */ $v + 1;\n$x = $v + 1;\n$a = $v /* h */ + 1;\n$a = ($v) /* h */ + 1;\n";
    for (pattern, want) in [
        // RED pre-fix: blanket head-comment refusal answered [] everywhere.
        ("$a = /* h */ $V + 1;", vec![3u32]),
        ("$a = /* h */ $V;", vec![3u32, 4]),
        ("$b = /* h */ $V + 1;", vec![6u32]),
        // Text-exactness negative control (sg rc1 [] — stays empty).
        ("$a = /* nope */ $V + 1;", vec![]),
        // Comment-free pattern: candidate head comments refuse (sg agrees),
        // candidate mid comments stay transparent, and the MID pattern
        // comment demands its exact counterpart (landed classes).
        ("$a = $V + 1;", vec![8u32, 9]),
        ("$a = $V /* h */ + 1;", vec![8u32, 9]),
    ] {
        let hits = match_pattern(Language::Php, f3, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 line set on the head-comment fixture (F-r40-3): {hits:?}"
        );
    }
}

/// F-r40-5 (r41, 90A-F2 LOW): a degenerate statement-only semicolon pattern
/// (ONLY `;`s and whitespace, two or more semicolons — multiple empty
/// statement roots) is sg's "Multiple AST nodes are detected" refusal in
/// every language whose grammar gives `;` statement weight: probed rc8
/// (2026-09-08, ATTACHED) for javascript/typescript/php/rust/go/c/cpp/
/// csharp/ruby/java on `;;`, `;;;`, `;; ;`. python/swift parse the same
/// spellings as lenient ERROR-node patterns and kotlin finds no nodes —
/// sg answers accepted-empty rc0/rc1 there (probed), so those languages
/// KEEP the silent-empty class (making them loud would diverge from sg in
/// the loud direction). A SINGLE `;` (whitespace-padded or not) is
/// sg-ACCEPTED everywhere (probed: answers the statement lines in
/// js/ts/php/rust/go/c/java, rc1-empty in py/rb) — the subject already
/// agrees, so count == 1 stays answerable. The census turns the refusal
/// into the loud fail-closed class exactly where sg exits 8.
/// Mutant tooth: drop the guard and every multi-semi cell returns to
/// answerable (silent `ok:true []` where sg rc8s); drop the language scope
/// and python/swift/kotlin go loud where sg answers empty.
#[test]
fn f91b_degenerate_semicolon_only_patterns_refuse_where_sg_rejects() {
    // RED pre-fix: multi-semicolon-only patterns answered silent-empty.
    for pattern in [";;", ";;;", ";; ;"] {
        for lang in [
            Language::JavaScript,
            Language::TypeScript,
            Language::Php,
            Language::Rust,
            Language::Go,
            Language::C,
            Language::Cpp,
            Language::CSharp,
            Language::Ruby,
            Language::Java,
        ] {
            assert!(
                !native_pattern_answerable(lang, pattern),
                "{pattern} over {lang:?}: sg 0.45.2 rc8 (Multiple AST nodes) — must refuse"
            );
        }
        // sg lenient-empty languages keep the silent class.
        for lang in [Language::Python, Language::Swift, Language::Kotlin] {
            assert!(
                native_pattern_answerable(lang, pattern),
                "{pattern} over {lang:?}: sg 0.45.2 answers accepted-empty — must stay answerable"
            );
        }
    }
    // A single `;` (whitespace-padded or not) is sg-accepted everywhere.
    for pattern in [";", " ; "] {
        for lang in [
            Language::JavaScript,
            Language::Python,
            Language::Php,
            Language::Ruby,
            Language::Rust,
        ] {
            assert!(
                native_pattern_answerable(lang, pattern),
                "{pattern} over {lang:?}: single semicolon is sg-accepted — must stay answerable"
            );
        }
    }
    // `$`-carrying faces (`$A;`) never enter the degenerate class.
    assert!(native_pattern_answerable(Language::JavaScript, "$A;"));
}

// ---------------------------------------------------------------------------
// PASS 92 (F-r41-2): fuzz face F26-0614 — ts `1_000 ($A) { $_0x1F }` (no
// newline). The pattern classifies NeverMatches (MixedCase `$_0x1F` token)
// and that arm answered answerable-everywhere, so the comment-free
// NeverMatches exemption walked the face into a silent ok:true [] where
// sg 0.45.2 rc8s "Multiple AST nodes are detected" (two-statement
// brace-compound spelling: `)` followed by `{` spawns two roots). Probe
// matrix (gauntlet artifacts/conformance/pass92/probes_run1.jsonl matrix F +
// probes_run2.jsonl matrix F2) fixes the sg-rc8 language scope:
// ts/js/py/rust/go/cpp/csharp refuse; php/c/java/rb/swift/kt accept-empty.
// ---------------------------------------------------------------------------

/// RED (F-r41-2): the brace-compound NeverMatches spellings must be
/// UNANSWERABLE in exactly the sg-rc8 languages, and the registered silent
/// NeverMatches cells must keep their answerability.
#[test]
fn f92_brace_compound_never_matches_unanswerable_where_sg_rejects() {
    // Classification witness (documents the lane): the MixedCase `$_0x1F`
    // token routes the whole pattern to NeverMatches before any shape arm.
    assert!(matches!(
        classify_native("1_000 ($A) { $_0x1F }"),
        Some(NativeKind::NeverMatches)
    ));
    let face = "1_000 ($A) { $_0x1F }";
    // sg 0.45.2 rc8 ("Multiple AST nodes are detected") — must be refused.
    for lang in [
        Language::TypeScript,
        Language::JavaScript,
        Language::Python,
        Language::Rust,
        Language::Go,
        Language::Cpp,
        Language::CSharp,
    ] {
        assert!(
            !native_pattern_answerable(lang, face),
            "{face} over {lang:?}: sg 0.45.2 rc8s the two-statement brace-compound \
             spelling — must not be answerable"
        );
    }
    // sg lenient-accepts (rc0/rc1 accepted-empty) — the silent walk answer is
    // the agreement; these MUST stay answerable.
    for lang in [
        Language::Php,
        Language::C,
        Language::Java,
        Language::Ruby,
        Language::Swift,
        Language::Kotlin,
    ] {
        assert!(
            native_pattern_answerable(lang, face),
            "{face} over {lang:?}: sg 0.45.2 accepts the spelling empty — must stay \
             answerable"
        );
    }
    // Registered silent NeverMatches cells must NOT move (f89a contract).
    for (lang, pattern) in [
        (Language::Python, "$a = 1"),
        (Language::Rust, "$x # note"),
        (Language::JavaScript, "$x # note"),
        (Language::Php, "$A && $b"),
    ] {
        assert!(
            native_pattern_answerable(lang, pattern),
            "{pattern} over {lang:?}: registered silent class must keep its \
             answerability"
        );
    }
}

// ---------------------------------------------------------------------------
// PASS 94b (r44, FB-93A-1/FB-93A-2/F-93B-2/FB-93A-4/FB-93A-5): the
// multi-root / semicolon-root posture moves from the pass-92 TEXTUAL
// `) {`-scan (7-language scope) to a PARSE-BASED sg pattern-acceptance gate
// (sg 0.45.2 PatternBuilder::single: pre-process the pattern, parse under
// the language grammar, require a single root node — no ERROR rejection, no
// descent). Probe matrices: gauntlet artifacts/conformance/pass94b/
// probes_run1.jsonl (230 cells, R/D/C/M, oracle ast-grep 0.45.2 ATTACHED).
// sg classes below are probed 2026-09-08; every number traces to that run.
// ---------------------------------------------------------------------------

/// RED (FB-93A-1 + FB-93A-2): root multiplicity must be decided by the
/// parse. sg 0.45.2 rc8 cells that the subject answered (hits / silent
/// empty) must become unanswerable; sg accepted cells the old textual scan
/// refused loud (IIFE, `$f($A) {`, …) must become answerable. Previously
/// sg-exact cells keep their classes (had_semi hits, php/kotlin loud
/// residuals, registered silent NeverMatches witnesses).
#[test]
fn f94b_parse_gate_root_multiplicity_matches_sg_matrix() {
    // sg rc8 — the subject must refuse (RED pre-fix: answered hits or
    // silent-empty).
    for (lang, pattern) in [
        // had_semi statement-root faces where sg splits the roots (probed
        // rc8 "Multiple AST nodes are detected"): go/py `$A ;` + `$A;`,
        // java `$A ;;` + `$A ; ;`.
        (Language::Go, "$A ;"),
        (Language::Python, "$A ;"),
        (Language::Go, "$A;"),
        (Language::Python, "$A;"),
        (Language::Java, "$A ;;"),
        (Language::Java, "$A ; ;"),
        // multi-root compound spellings probed rc8 where the subject walked
        // a silent ok:true [].
        (Language::CSharp, "$f($A) ;"),
        (Language::Go, "$f($A) ;"),
        (Language::Python, "$f($A) ;"),
        (Language::Ruby, "$f($A) ;"),
        (Language::Swift, "$f($A) ;"),
        (Language::Ruby, "$f($A) ]"),
        (Language::Swift, "$f($A) ]"),
        (Language::Ruby, "$f($A) {"),
        (Language::Swift, "$f($A) {"),
        (Language::Python, "$f($A) $g($B)"),
        (Language::Java, "$f($A) { $B(); }"),
        (Language::JavaScript, "$f($A) }"),
        (Language::Ruby, "$f($A) }"),
        (Language::Swift, "$f($A) }"),
        (Language::TypeScript, "$f($A) }"),
        (Language::Python, "$f(\"( x) { y\") $G($B)"),
        // php tag-less sharpening: sg's php grammar roots each `;`-terminated
        // statement separately (probed rc8), while the workspace php grammar
        // folds the tag-less document into one `text` node — two or more
        // quote-external semicolons refuse.
        (Language::Php, "$A ;;"),
        (Language::Php, "$A ; ;"),
        (Language::Php, ";; $A"),
        (Language::Php, "; $A"),
        (Language::Php, "; ; $A"),
        // NeverMatches-class twin (`$a` lowercase-led routes the whole
        // pattern to NeverMatches): the sharpening is what refuses it —
        // without it the tag-less text-node fold would answer answerable
        // where sg rc8s the statement split.
        (Language::Php, "$a ;;"),
    ] {
        assert!(
            !native_pattern_answerable(lang, pattern),
            "{pattern} over {lang:?}: sg 0.45.2 rc8 (multi-root) — must refuse \
             (probes_run1.jsonl matrix R)"
        );
    }
    // KOTLIN GRAMMAR-DRIFT RESIDUAL (registered; form-1 predicate "adopt
    // sg's pinned fwcd tree-sitter-kotlin grammar"): sg 0.45.2 rc8s these
    // four compounds — its grammar splits the statement roots — while this
    // repo's kotlin-ng grammar folds `;`/braces into the call root and the
    // faithful parse gate accepts. The registered silent class stands
    // (probes_run1.jsonl matrix R, kotlin cells).
    for pattern in [
        "$f($A) ;",
        "$f($A) $g($B)",
        "$f($A) { $B(); }",
        "$f($A) { $_0x1F }",
    ] {
        assert!(
            native_pattern_answerable(Language::Kotlin, pattern),
            "{pattern} over Kotlin: registered grammar-drift silent residual — \
             posture frozen until the fwcd grammar is adopted"
        );
    }
    // sg ACCEPTS these — the pass-92 textual scan over-refused them loud
    // (rc2) where sg answers accepted-empty (probed rc0/rc1). RED pre-fix.
    for (lang, pattern) in [
        (Language::TypeScript, "$f($A) {"),
        (Language::JavaScript, "$f($A) {"),
        (Language::Python, "$f($A) {"),
        (Language::Rust, "$f($A) {"),
        (Language::Go, "$f($A) {"),
        (Language::Cpp, "$f($A) {"),
        (Language::CSharp, "$f($A) {"),
        (Language::Cpp, "$f($A) { $B(); }"),
        (Language::Go, "$f($A) { $B(); }"),
        (Language::Python, "$f($A) { $B(); }"),
        (Language::JavaScript, "(function($a) { z(); })"),
        (Language::JavaScript, "(function($a) { z(); })()"),
        (Language::TypeScript, "(function($a) { z(); })"),
        (Language::TypeScript, "(function($a) { z(); })()"),
        (Language::Go, "func($a) { work() }"),
        (Language::CSharp, "using ($a = $b) { $c(); }"),
    ] {
        assert!(
            native_pattern_answerable(lang, pattern),
            "{pattern} over {lang:?}: sg 0.45.2 accepts the spelling — must stay \
             answerable (probes_run1.jsonl matrix R)"
        );
    }
    // GO LEADING-SEMICOLON RESIDUAL (registered; form-1 predicate "the
    // tree-sitter-go ERROR-root multiplicity at top level matches sg's
    // pinned parse"): sg 0.45.2 accepts `; $A` empty while this repo's go
    // grammar roots the leading `;` and the expression as separate nodes,
    // so the faithful gate refuses — the registered LOUD residual stands
    // (fail-closed direction).
    assert!(!native_pattern_answerable(Language::Go, "; $A"));
    // Registered sg-exact classes must NOT move — had_semi statement-root
    // hits (sg hits, probed agreeing):
    for (lang, pattern) in [
        (Language::JavaScript, "$A;"),
        (Language::TypeScript, "$A;"),
        (Language::Rust, "$A;"),
        (Language::C, "$A;"),
        (Language::C, "$A ;"),
        (Language::Cpp, "$A;"),
        (Language::Cpp, "$A ;"),
        (Language::Java, "$A;"),
    ] {
        assert!(
            native_pattern_answerable(lang, pattern),
            "{pattern} over {lang:?}: sg hits the face — answerability frozen"
        );
    }
    // php/kotlin single-semi faces: sg hits, registered LOUD residual
    // (90B-T4) — the general-lane build failure keeps them unanswerable at
    // the library surface. kotlin `$A;` is the same posture (the binary's
    // hits ride the lang-free ingress fallback; the library answerability
    // was false before and after the pass-94b gate swap, so the binary
    // class is unchanged).
    assert!(!native_pattern_answerable(Language::Php, "$A ;"));
    assert!(!native_pattern_answerable(Language::Php, "$A;"));
    assert!(!native_pattern_answerable(Language::Kotlin, "$A ;"));
    assert!(!native_pattern_answerable(Language::Kotlin, "$A;"));
    // registered silent NeverMatches witnesses (f89a/f92).
    for (lang, pattern) in [
        (Language::Python, "$a = 1"),
        (Language::Rust, "$x # note"),
        (Language::JavaScript, "$x # note"),
        (Language::Php, "$A && $b"),
    ] {
        assert!(
            native_pattern_answerable(lang, pattern),
            "{pattern} over {lang:?}: registered silent class must keep its \
             answerability"
        );
    }
    // the fuzz face keeps its sg-rc8 language scope under the parse gate
    // (ts parse splits the brace compound; the kotlin cell stays lenient —
    // the kotlin-ng fold rides the registered grammar-drift residual).
    assert!(!native_pattern_answerable(
        Language::TypeScript,
        "1_000 ($A) { $_0x1F }"
    ));
    assert!(native_pattern_answerable(
        Language::Kotlin,
        "1_000 ($A) { $_0x1F }"
    ));
}

/// RED (F-93B-2): `//` is python's floor-div operator, not comment syntax —
/// the language-free comment lane refused `$A // $B` (census rc2) where sg
/// 0.45.2 answers hits (probes_run1.jsonl matrix D: `$A // $B` {1,2,2,3},
/// `$A // 2` {3}, `a // b` {1,2}).
#[test]
fn f94b_py_floor_div_faces_answer_where_sg_answers() {
    let src = "q = a // b\nr = a // b // c\ns = a // 2\nt = a / b\n";
    // census-level: the faces must be answerable (RED pre-fix: rc2).
    assert!(native_pattern_answerable(Language::Python, "$A // $B"));
    assert!(native_pattern_answerable(Language::Python, "$A // 2"));
    assert!(native_pattern_answerable(Language::Python, "a // b"));
    // sg-exact line sets on the floor-div fixture (D matrix).
    let hits = match_pattern(Language::Python, src, "$A // $B").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1, 2, 3],
        "$A // $B: sg answers lines 1,2,2,3 (line 2 carries both chain binaries)"
    );
    let hits = match_pattern(Language::Python, src, "$A // 2").unwrap();
    assert_eq!(klass_lines(&hits), vec![3], "$A // 2: sg answers line 3");
    let hits = match_pattern(Language::Python, src, "a // b").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1, 2],
        "a // b: sg answers lines 1,2"
    );
    // the `/*` arm stays refused for python (no block-comment syntax; the
    // registered py comment faces keep their contract).
    assert!(!native_pattern_answerable(
        Language::Python,
        "$A /* c */ $B"
    ));
}

/// RED (FB-93A-4): candidate-side comment transparency gaps — sg answers
/// each face with the exact line set below (probes_run1.jsonl matrix C,
/// corpora c_php/c_go ATTACHED); the subject missed the comment-carrying
/// candidates (php assignment mid/double-head comments + bare-meta RHS head
/// comments; go literal call arg comments via the statement_list template
/// root).
#[test]
fn f94b_candidate_comment_transparency_matches_sg() {
    // php corpus (c_php/cmt.php, pass94b)
    let php = "<?php\n$a = 1 + 2;\n$a = /* h */ $v + 1;\n$a = /* a */ /* b */ $v + 1;\n$x = $v /* mid */ + 1;\n$x = $v + 1;\n$b = 7;\n$a = /* head1 */ $v + 1;\n$a = /* \u{c3}5 \u{d0}94 */ $v + 1;\n$a = ($v) /* h */ + 1;\n";
    for (pattern, want, why) in [
        (
            "$x = $v + 1;",
            vec![5, 6],
            "mid-comment candidate answers (comment is inside the binary)",
        ),
        (
            "$a = $V;",
            vec![2, 3, 4, 8, 9, 10],
            "bare-meta RHS swallows candidate head comments",
        ),
        (
            "$a = /* a */ /* b */ $V + 1;",
            vec![4],
            "double head-comment slot demand",
        ),
        (
            "$a = /* a */ $V + 1;",
            vec![],
            "text-exact slot: no line carries exactly /* a */ (sg rc1)",
        ),
        (
            "$a = $V + 1;",
            vec![10],
            "comment-free expr pattern: head comments refuse, mid stays",
        ),
    ] {
        let hits = match_pattern(Language::Php, php, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: {why} (sg 0.45.2 matrix C)"
        );
    }
    // go corpus (c_go/cmt.go): the literal lane's structural arm must run
    // (template root descends go's statement_list) and container comments
    // stay transparent.
    let go = "package main\n\nfunc main() {\n\tf(1, 2)\n\tf(1, /* n */ 2)\n\tf(3, 4)\n}\n";
    let hits = match_pattern(Language::Go, go, "f(1, 2)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![4, 5],
        "f(1, 2): sg answers the plain and the comment-carrying call (matrix C)"
    );
}

/// RED (FB-93A-5): php flat member faces with a META NAME segment walked to
/// a silent ok:true [] where sg 0.45.2 answers (probes_run1.jsonl matrix M,
/// corpus m_php ATTACHED): `$o->$M();` {2}, `$o?->$M();` {3}, `$this->$P;`
/// {4}, `$o->$M;` {5}, `$o?->$M;` {6}; `$this->$P();` accepted-empty (no
/// candidate carries a `$this->x()` call).
#[test]
fn f94b_php_flat_member_meta_name_faces_answer_sg() {
    let php = "<?php\n$o->m();\n$o?->m();\n$this->prop;\n$o->prop;\n$o?->prop;\n$o->m(1);\n$o?->m(1);\n$this->prop = 1;\n$o->m($a, $b);\n";
    for (pattern, want) in [
        ("$o->$M();", vec![2]),
        ("$o?->$M();", vec![3]),
        ("$this->$P;", vec![4]),
        ("$o->$M;", vec![5]),
        ("$o?->$M;", vec![6]),
        ("$this->$P();", vec![]),
    ] {
        let hits = match_pattern(Language::Php, php, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern}: sg 0.45.2 matrix M line set"
        );
    }
}

/// PASS 96 probe corpora — byte-identical to the pass96 y_* probe corpora
/// (artifacts/conformance/pass96/corpus, oracle 0.45.2 ATTACHED 2026-09-08),
/// so the expected line sets below ARE the recorded oracle truth.
const F96_RUST_SRC: &str = "fn main() {\n    \u{b5}A + 1;\n    x + 1;\n    \u{b5} + 1;\n    f(9);\n    f(y);\n    \u{b5}A;\n    zz;\n    g(\"hw\");\n    g(\"other\");\n    w = v;\n}\n";
const F96_GO_SRC: &str = "package main\n\nfunc main() {\n    \u{b5}A + 1\n    x + 1\n    \u{b5} + 1\n    f(9)\n    f(y)\n    \u{b5}A\n    zz\n    g(\"hw\")\n    g(\"other\")\n    w = v\n}\n";
const F96_PY_SRC: &str =
    "\u{b5}A + 1\nx + 1\n\u{b5} + 1\nf(9)\nf(y)\n\u{b5}A\nzz\ng(\"hw\")\ng(\"other\")\nw = v\n";
const F96_C_SRC: &str = "void main() {\n    \u{10000}A + 1;\n    x + 1;\n    \u{10000} + 1;\n    f(9);\n    f(y);\n    \u{10000}A;\n    zz;\n    g(\"hw\");\n    g(\"other\");\n    w = v;\n}\n";
const F96_JAVA_SRC: &str = "class T {\n    void m() {\n    \u{b5}A + 1;\n    x + 1;\n    \u{b5} + 1;\n    f(9);\n    f(y);\n    \u{b5}A;\n    zz;\n    g(\"hw\");\n    g(\"other\");\n    w = v;\n    }\n}\n";
const F96_JS_SRC: &str =
    "\u{b5}A + 1\nx + 1\n\u{b5} + 1\nf(9)\nf(y)\n\u{b5}A\nzz\ng(\"hw\")\ng(\"other\")\nw = v\n";

/// RED (F-95A-1): sg 0.45.2 treats expando-spelled metavariables
/// (`extract_meta_var(src, expando_char)`, meta_var.rs:235) as METAs: for the
/// 8 µ-languages `µA`=`$A`, `µµA`=`$$A`, `µµµ`=`$$$`, `µµµA`=`$$$A`,
/// `µ_`=`$_`; c/cpp use U+10000. The subject matcher routed every no-`$`
/// spelling to the literal lane (silent miss / literal-only hit sets). The
/// fix normalizes expando meta spelling to `$` spelling at match_pattern
/// ingress, so every µ-spelling must answer EXACTLY its `$` twin's sg-truth
/// set (matrix m1_expando_meta.jsonl, oracle sets recorded 2026-09-08).
/// Lowercase-tail / 4-run / double-expando spellings stay LITERAL (sg: not
/// meta-shaped), java/js/ts have no expando and stay literal.
#[test]
fn f96_expando_meta_spelling_matches_dollar_semantics() {
    for (lang, src, expando_pattern, dollar_pattern, want) in [
        (
            Language::Rust,
            F96_RUST_SRC,
            "µA + 1",
            "$A + 1",
            vec![2, 3, 4],
        ),
        (
            Language::Rust,
            F96_RUST_SRC,
            "µ_ + 1",
            "$_ + 1",
            vec![2, 3, 4],
        ),
        (Language::Rust, F96_RUST_SRC, "f(µA)", "f($A)", vec![5, 6]),
        (Language::Rust, F96_RUST_SRC, "f(µµµ)", "f($$$)", vec![5, 6]),
        (
            Language::Rust,
            F96_RUST_SRC,
            "$A + µB",
            "$A + $B",
            vec![2, 3, 4],
        ),
        (
            Language::Rust,
            F96_RUST_SRC,
            "µµA",
            "$$A",
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        ),
        (Language::Go, F96_GO_SRC, "µA + 1", "$A + 1", vec![4, 5, 6]),
        (Language::Go, F96_GO_SRC, "f(µA)", "f($A)", vec![7, 8]),
        (
            Language::Python,
            F96_PY_SRC,
            "µA + 1",
            "$A + 1",
            vec![1, 2, 3],
        ),
        (Language::Python, F96_PY_SRC, "f(µA)", "f($A)", vec![4, 5]),
        (
            Language::C,
            F96_C_SRC,
            "\u{10000}A + 1",
            "$A + 1",
            vec![2, 3, 4],
        ),
        (
            Language::C,
            F96_C_SRC,
            "\u{10000}_ + 1",
            "$_ + 1",
            vec![2, 3, 4],
        ),
    ] {
        let hits = match_pattern(lang, src, expando_pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{lang:?} {expando_pattern:?}: sg answers the meta reading \
             (matrix m1_expando_meta); must equal the $-twin's set"
        );
        let twin = match_pattern(lang, src, dollar_pattern).unwrap();
        assert_eq!(
            klass_lines(&twin),
            want,
            "{lang:?} {dollar_pattern:?}: the $-twin contract this pin anchors"
        );
    }
    // Root-MultiCapture-inside-expression faces: sg answers [2,3,4], but the
    // subject's $-twin (`$$$A + 1` / `$$$_ + 1`) is PRE-EXISTING registered
    // rc2-loud at the CLI ingress (matrix m1; probed again post-fix) and
    // answers empty at the match_pattern level. F-95A-1's contract is
    // spelling convergence: the µ-spelling must fold to exactly the twin's
    // behavior (the loudness rides the same core ingress path for both
    // spellings — pinned in tests/core/pattern_routing.rs f96 pins).
    for (lang, src, expando_pattern, dollar_pattern) in [
        (Language::Rust, F96_RUST_SRC, "µµµA + 1", "$$$A + 1"),
        (Language::Rust, F96_RUST_SRC, "µµµ_ + 1", "$$$_ + 1"),
    ] {
        assert_eq!(
            klass_lines(&match_pattern(lang, src, expando_pattern).unwrap()),
            klass_lines(&match_pattern(lang, src, dollar_pattern).unwrap()),
            "{lang:?} {expando_pattern:?}: multi-capture-in-expression must \
             converge to its $-twin (both loud at the CLI ingress)"
        );
    }
    // Literal-preserving controls (sg: not meta-shaped → literal faces).
    for (lang, src, pattern, want) in [
        (Language::Rust, F96_RUST_SRC, "µ + 1", vec![4]),
        (Language::Rust, F96_RUST_SRC, "µabc + 1", vec![]),
        (Language::Rust, F96_RUST_SRC, "µµµµA + 1", vec![]),
        (Language::Rust, F96_RUST_SRC, "f(µµ)", vec![]),
        // java/js/ts have NO expando preprocessing in sg — literal both sides.
        (Language::Java, F96_JAVA_SRC, "µA + 1", vec![3]),
        (Language::JavaScript, F96_JS_SRC, "µA + 1", vec![1]),
    ] {
        let hits = match_pattern(lang, src, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{lang:?} {pattern:?}: non-meta expando spelling stays literal \
             (matrix m1 negative controls)"
        );
    }
}

/// RED (F-95A-1 loud half): sg 0.45.2 REFUSES a root multi-meta variable
/// (`PatternError::RootMultiMetaVar`, exit 8 — matrix m1 `µµµ`/`$$$` cells)
/// regardless of spelling; the subject must classify those roots unanswerable
/// (census-loud) in the expando languages, and answerability must be
/// spelling-invariant (`µµµ` ≡ `$$$`). Java/js/ts have no expando: their
/// `µµµ` is a literal identifier and stays answerable.
#[test]
fn f96_expando_root_multi_meta_stays_loud_like_sg() {
    // Root Multi-meta (Multiple / MultiCapture) is sg rc8: unanswerable in
    // every expando language, spelling-invariantly. C/Cpp take the U+10000
    // expando spelling (their `µ` is a literal byte, not the expando).
    for (lang, expando_multi, expando_single) in [
        (Language::Rust, "µµµ", "µA"),
        (Language::Go, "µµµ", "µA"),
        (Language::Python, "µµµ", "µA"),
        (Language::Php, "µµµ", "µA"),
        (Language::Ruby, "µµµ", "µA"),
        (Language::Swift, "µµµ", "µA"),
        (Language::Kotlin, "µµµ", "µA"),
        (Language::CSharp, "µµµ", "µA"),
        (Language::C, "\u{10000}\u{10000}\u{10000}", "\u{10000}A"),
        (Language::Cpp, "\u{10000}\u{10000}\u{10000}", "\u{10000}A"),
    ] {
        for expando_pattern in [expando_multi, &format!("{expando_multi}A")] {
            assert!(
                !native_pattern_answerable(lang, expando_pattern),
                "{lang:?} {expando_pattern:?}: sg rc8 RootMultiMetaVar — must be \
                 unanswerable so the census keeps the face loud"
            );
            assert_eq!(
                native_pattern_answerable(lang, expando_pattern),
                native_pattern_answerable(lang, "$$$"),
                "{lang:?}: spelling must not change answerability"
            );
        }
        // Bare single-capture root is sg-ANSWERED (m1: `µA` sg hits [1..11]) —
        // answerable, and the spelling must not change that.
        assert_eq!(
            native_pattern_answerable(lang, expando_single),
            native_pattern_answerable(lang, "$A"),
            "{lang:?}: bare single-capture root stays answerable under both \
             spellings (sg answers the meta reading)"
        );
    }
    for lang in [Language::Java, Language::JavaScript, Language::TypeScript] {
        assert!(
            native_pattern_answerable(lang, "µµµ"),
            "{lang:?}: no expando preprocessing — literal identifier face stays \
             answerable (matrix m1 java/js/ts controls)"
        );
    }
}

/// RED (FB-95B-2): sg 0.45.2 ACCEPTS the paren-free ERROR-repair bracket
/// fragments ($A ] / $A ) / $A }) and answers rc0-EMPTY (mf_bracket_fragments
/// matrix: 31 accepted-empty cells across rust/go/py/php/kt/cs/c/cpp/java/js/
/// ts; ruby+swift all tails and js/ts `}` are sg rc8 REFUSES). The subject
/// refused them all (ingress rc2). The fix: the fragment class is sg-accepted
/// (walk answers the honest empty) exactly where the BARE gate accepts the
/// parse; sg-rc8 spellings stay census-loud; the registered `$A ;` / `$A +`
/// loud faces are untouched (tail is not a bracket).
#[test]
fn f96_bracket_fragment_faces_accepted_empty_like_sg() {
    // sg accepted-empty: admitted to the walk (needs_ast_grep_fallback false)
    // and answerable per language (the census must not call them unanswerable).
    for (lang, pattern) in [
        (Language::Python, "$A ]"),
        (Language::Python, "$A )"),
        (Language::Python, "$A }"),
        (Language::Java, "$A ]"),
        (Language::Java, "$A )"),
        (Language::Java, "$A }"),
        (Language::JavaScript, "$A ]"),
        (Language::JavaScript, "$A )"),
        (Language::Rust, "$A ]"),
        (Language::Rust, "$A )"),
        (Language::Rust, "$A }"),
        (Language::Go, "$A ]"),
        (Language::Kotlin, "$A ]"),
        (Language::CSharp, "$A }"),
        (Language::C, "$A ]"),
        (Language::Cpp, "$A )"),
        (Language::Php, "$A ]"),
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern:?}: sg 0.45.2 accepts this ERROR-repair fragment empty \
             (matrix mf_bracket_fragments) — the lang-free ingress must admit \
             the walk instead of failing closed"
        );
        assert!(
            native_pattern_answerable(lang, pattern),
            "{lang:?} {pattern:?}: sg accepts-empty — the census must treat the \
             language as answerable (the walk's empty IS the sg agreement)"
        );
    }
    // sg rc8 refuses: LOUD_FOLD preserved (loud stays loud).
    for (lang, pattern) in [
        (Language::Ruby, "$A ]"),
        (Language::Ruby, "$A )"),
        (Language::Ruby, "$A }"),
        (Language::Swift, "$A ]"),
        (Language::Swift, "$A }"),
        (Language::JavaScript, "$A }"),
        (Language::TypeScript, "$A }"),
        (Language::Go, "$A ;"),
        (Language::Python, "$A ;"),
    ] {
        assert!(
            !native_pattern_answerable(lang, pattern),
            "{lang:?} {pattern:?}: sg 0.45.2 rc8 (multi-root) — must stay \
             unanswerable so the census keeps the registered loud fold"
        );
    }
    // Registered loud faces with a non-bracket tail are untouched. (`$A ;`
    // and the `$f($A) …` compounds are served by the classifier/general-lane
    // early returns, not the fragment admission, so they cannot discriminate
    // here; `$A +` DOES reach the fragment arm and must stay refused.)
    {
        let pattern = "$A +";
        assert!(
            needs_ast_grep_fallback(pattern),
            "{pattern:?}: the registered loud class (trailing-operator) must \
             be untouched by the bracket-fragment admission"
        );
    }
}

/// RED (FB-95B-1): sg 0.45.2 binds a metavariable whose ENTIRE text is a
/// plain string literal's content (the parsed `string_content`/
/// `string_fragment` leaf IS a meta token — matrix ms_instring_meta: js
/// `g("$A")` {8,9} answers `g("hw")` AND `g("other")`). The subject refused
/// the whole class (template-build placeholder-in-literal refusal → ingress
/// rc2). The fix admits the build when the placeholder is the WHOLE content
/// leaf; prefix/suffix spellings (`"pre-$A"`) keep the literal-text refusal
/// (sg reads those as literal content too), and non-expando `g("µA")` stays
/// literal (matrix EMPTY_AGREE control).
#[test]
fn f96_string_meta_faces_answer_where_sg_answers() {
    for (lang, src, pattern, want) in [
        (Language::JavaScript, F96_JS_SRC, "g(\"$A\")", vec![8, 9]),
        (Language::JavaScript, F96_JS_SRC, "g(\"$B\")", vec![8, 9]),
        (Language::JavaScript, F96_JS_SRC, "\"$A\"", vec![8, 9]),
        (Language::TypeScript, F96_JS_SRC, "g(\"$A\")", vec![8, 9]),
        (Language::Python, F96_PY_SRC, "g(\"$A\")", vec![8, 9]),
        (Language::Python, F96_PY_SRC, "\"$A\"", vec![8, 9]),
        (Language::Rust, F96_RUST_SRC, "g(\"$A\")", vec![9, 10]),
        (Language::Rust, F96_RUST_SRC, "g(\"µA\")", vec![9, 10]),
        (Language::Go, F96_GO_SRC, "g(\"$A\")", vec![11, 12]),
        (Language::C, F96_C_SRC, "g(\"\u{10000}A\")", vec![9, 10]),
        (Language::Java, F96_JAVA_SRC, "g(\"$A\")", vec![10, 11]),
    ] {
        let hits = match_pattern(lang, src, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{lang:?} {pattern:?}: sg binds the whole-content string meta \
             (matrix ms_instring_meta); the template must build and answer"
        );
    }
    // Over-match guards: a placeholder that is a PROPER SUBSTRING of the
    // string content keeps the literal-text refusal (sg: literal content
    // leaf, answers nothing on clean sources); the two-argument call keeps
    // plain call semantics (sg rc1-empty); js `g("µA")` (no expando in js)
    // stays a literal empty.
    assert!(
        needs_ast_grep_fallback("g(\"pre-$A\")"),
        "prefix/suffix in-string placeholders keep the fail-closed ingress"
    );
    assert!(
        !needs_ast_grep_fallback("\"$A\""),
        "whole-content string meta is a supported structural template \
         (FB-95B-1 named face: py `\"$A\"` sg [8,9]) — the CLI ingress must \
         not rc2 it"
    );
    assert!(
        needs_ast_grep_fallback("\"pre-$A\""),
        "bare-string SUBSTRING spellings keep the fail-closed ingress"
    );
    assert!(
        needs_ast_grep_fallback("\"$A$B\""),
        "multi-meta string content is sg-literal (leaf not a meta) — stays \
         fail-closed"
    );
    assert!(
        match_pattern(Language::JavaScript, F96_JS_SRC, "g(\"pre-$A\")")
            .unwrap()
            .is_empty(),
        "substring in-string placeholder must not answer"
    );
    let hits = match_pattern(Language::JavaScript, F96_JS_SRC, "g(\"$A\", 1)").unwrap();
    assert!(
        hits.is_empty(),
        "two-arg call template answers the one-arg call sites nothing (sg rc1)"
    );
    let hits = match_pattern(Language::JavaScript, F96_JS_SRC, "g(\"µA\")").unwrap();
    assert!(
        hits.is_empty(),
        "js has no expando: µA inside a string is literal content (matrix \
         ms EMPTY_AGREE control)"
    );
}

// ---------------------------------------------------------------------------
// PASS 98 (r48 remediation): F-97A-1 / FB-97B-1 whole-node expando validation,
// F-97A-1 layer-2 non-canonical $-token literals, F97X-0072 µ-before-$
// composition, F-97A-2 $-less unbalanced-}-tail census governance.
// Probe matrices: artifacts/conformance/pass98/matrix/m3a*.jsonl,
// m3b_unbalanced_tails.jsonl, m3c_mu_before_dollar_composition.jsonl
// (oracle 0.45.2 ATTACHED 2026-09-09, subject binary d3b5a048052c4c88).
// ---------------------------------------------------------------------------

/// Byte-identical to the pass98 c98_wholenode corpora (mixed-tail rows appended
/// to the pass-96 shape). sg truth for every pinned face was probed live this
/// round; line numbers below are the probed oracle sets.
const F98_RUST_SRC: &str = "fn main() {\n    \u{b5}A + 1\n    x + 1\n    \u{b5} + 1\n    f(9)\n    f(y)\n    \u{b5}A\n    zz\n    g(\"hw\")\n    g(\"other\")\n    w = v\n    \u{b5}Bx + 1\n    k(\u{b5}Bx)\n    f(\"\u{b5}Ax\")\n    \u{b5}Able\n    \u{b5}Ab\n    \u{b5}\u{b5}Able\n    \u{b5}\u{b5}\u{b5}ABle\n}\n";
const F98_PY_SRC: &str = "\u{b5}A + 1\nx + 1\n\u{b5} + 1\nf(9)\nf(y)\n\u{b5}A\nzz\ng(\"hw\")\ng(\"other\")\nw = v\n\u{b5}Bx + 1\nk(\u{b5}Bx)\nf(\"\u{b5}Ax\")\n\u{b5}Able\n\u{b5}Ab\n\u{b5}\u{b5}Able\n\u{b5}\u{b5}\u{b5}ABle\n";
const F98_GO_SRC: &str = "package main\n\nfunc main() {\n    \u{b5}A + 1\n    x + 1\n    \u{b5} + 1\n    f(9)\n    f(y)\n    \u{b5}A\n    zz\n    g(\"hw\")\n    g(\"other\")\n    w = v\n    \u{b5}Bx + 1\n    k(\u{b5}Bx)\n    f(\"\u{b5}Ax\")\n    \u{b5}Able\n    \u{b5}Ab\n    \u{b5}\u{b5}Able\n    \u{b5}\u{b5}\u{b5}ABle\n}\n";
const F98_C_SRC: &str = "void main() {\n    \u{10000}A + 1;\n    x + 1;\n    \u{10000} + 1;\n    f(9);\n    f(y);\n    \u{10000}A;\n    zz;\n    g(\"hw\");\n    g(\"other\");\n    w = v;\n    \u{10000}Bx + 1;\n    k(\u{10000}Bx);\n    f(\"\u{10000}Ax\");\n    \u{10000}Able;\n    \u{10000}Ab;\n    \u{10000}\u{10000}Able;\n    \u{10000}\u{10000}\u{10000}ABle;\n}\n";
const F98_CPP_SRC: &str = F98_C_SRC;
const F98_PHP_SRC: &str = "<?php\n    \u{b5}A + 1;\n    x + 1;\n    \u{b5} + 1;\n    f(9);\n    f(y);\n    \u{b5}A;\n    zz;\n    g(\"hw\");\n    g(\"other\");\n    w = v;\n    \u{b5}Bx + 1;\n    k(\u{b5}Bx);\n    f(\"\u{b5}Ax\");\n    \u{b5}Able;\n    \u{b5}Ab;\n    \u{b5}\u{b5}Able;\n    \u{b5}\u{b5}\u{b5}ABle;\n";
const F98_RUBY_SRC: &str = "\u{b5}A + 1\nx + 1\n\u{b5} + 1\nf(9)\nf(y)\n\u{b5}A\nzz\ng(\"hw\")\ng(\"other\")\nw = v\n\u{b5}Bx + 1\nk(\u{b5}Bx)\nf(\"\u{b5}Ax\")\n\u{b5}Able\n\u{b5}Ab\n\u{b5}\u{b5}Able\n\u{b5}\u{b5}\u{b5}ABle\n";
const F98_SWIFT_SRC: &str = "func main() {\n    \u{b5}A + 1\n    x + 1\n    \u{b5} + 1\n    f(9)\n    f(y)\n    \u{b5}A\n    zz\n    g(\"hw\")\n    g(\"other\")\n    w = v\n    \u{b5}Bx + 1\n    k(\u{b5}Bx)\n    f(\"\u{b5}Ax\")\n    \u{b5}Able\n    \u{b5}Ab\n    \u{b5}\u{b5}Able\n    \u{b5}\u{b5}\u{b5}ABle\n}\n";
const F98_KOTLIN_SRC: &str = "fun main() {\n    \u{b5}A + 1\n    x + 1\n    \u{b5} + 1\n    f(9)\n    f(y)\n    \u{b5}A\n    zz\n    g(\"hw\")\n    g(\"other\")\n    w = v\n    \u{b5}Bx + 1\n    k(\u{b5}Bx)\n    f(\"\u{b5}Ax\")\n    \u{b5}Able\n    \u{b5}Ab\n    \u{b5}\u{b5}Able\n    \u{b5}\u{b5}\u{b5}ABle\n}\n";
const F98_CSHARP_SRC: &str = "class T {\n    void M() {\n    \u{b5}A + 1;\n    x + 1;\n    \u{b5} + 1;\n    f(9);\n    f(y);\n    \u{b5}A;\n    zz;\n    g(\"hw\");\n    g(\"other\");\n    w = v;\n    \u{b5}Bx + 1;\n    k(\u{b5}Bx);\n    f(\"\u{b5}Ax\");\n    \u{b5}Able;\n    \u{b5}Ab;\n    \u{b5}\u{b5}Able;\n    \u{b5}\u{b5}\u{b5}ABle;\n    }\n}\n";
/// $-token rows for the no-expando java layer-2 faces ($Bx is literal java
/// identifier text; sg matches these rows literally under the $ spelling).
const F98_JAVA_JNAME_SRC: &str = "class T {\n    void m() {\n    $A + 1;\n    x + 1;\n    $Bx + 1;\n    f(9);\n    f(y);\n    $A;\n    zz;\n    g(\"hw\");\n    g(\"other\");\n    w = v;\n    $Bx + 2;\n    k($Bx);\n    f(\"$Ax\");\n    $Bx;\n    k($Bx, 1);\n    }\n}\n";
/// µ-before-$ composition corpora (F97X-0072): assignment rows.
const F98_COMPOSE_PY: &str = "msg = \"hello \" + name\nw = v\ny = z + 1\n";
const F98_COMPOSE_RS: &str =
    "fn main() {\n    msg = \"hello \" + name;\n    w = v;\n    y = z + 1;\n}\n";

/// RED (F-97A-1 layer 1 / FB-97B-1): sg 0.45.2 validates the meta grammar over
/// the ENTIRE parsed node text (meta_var.rs:260-264 — any invalid char in the
/// tail ⇒ None ⇒ literal). `µAble`/`µAx`-class tokens are therefore literal
/// IDENTIFIERS / literal string content and sg answers the verbatim rows (m3a
/// oracle sets). The r46 normalizer's tail scan was PREFIX-GREEDY: it read
/// `µAble` as meta `µA` + junk `ble`, rewrote to `$Able`, and the
/// non-canonical `$`-token routed to a silent empty — a regression against the
/// normalizer's own verbatim contract. The fix: whole-node validation (an
/// ident continuation after the [A-Z_0-9] prefix ⇒ literal, verbatim).
#[test]
fn f98_expando_whole_node_literal_stays_verbatim() {
    for (lang, src, pattern, want) in [
        (Language::Rust, F98_RUST_SRC, "k(µBx)", vec![13]),
        (Language::Rust, F98_RUST_SRC, "f(\"µAx\")", vec![14]),
        (Language::Rust, F98_RUST_SRC, "µAble", vec![15]),
        (Language::Rust, F98_RUST_SRC, "µAb", vec![16]),
        (Language::Rust, F98_RUST_SRC, "µµAble", vec![17]),
        (Language::Rust, F98_RUST_SRC, "µµµABle", vec![18]),
        (Language::Python, F98_PY_SRC, "µBx + 1", vec![11]),
        (Language::Python, F98_PY_SRC, "k(µBx)", vec![12]),
        (Language::Python, F98_PY_SRC, "f(\"µAx\")", vec![13]),
        (Language::Python, F98_PY_SRC, "µAble", vec![14]),
        (Language::Python, F98_PY_SRC, "µµAble", vec![16]),
        (Language::Python, F98_PY_SRC, "µµµABle", vec![17]),
        (Language::Go, F98_GO_SRC, "k(µBx)", vec![15]),
        (Language::Go, F98_GO_SRC, "µAble", vec![17]),
        (Language::C, F98_C_SRC, "\u{10000}Bx + 1", vec![12]),
        (Language::C, F98_C_SRC, "f(\"\u{10000}Ax\")", vec![14]),
        (Language::C, F98_C_SRC, "\u{10000}Able", vec![15]),
        (Language::C, F98_C_SRC, "\u{10000}\u{10000}Able", vec![17]),
        (Language::Cpp, F98_CPP_SRC, "k(\u{10000}Bx)", vec![13]),
        (Language::Php, F98_PHP_SRC, "k(µBx)", vec![13]),
        (Language::Php, F98_PHP_SRC, "µAble", vec![15]),
        (Language::Ruby, F98_RUBY_SRC, "k(µBx)", vec![12]),
        (Language::Swift, F98_SWIFT_SRC, "k(µBx)", vec![13]),
        (Language::Kotlin, F98_KOTLIN_SRC, "k(µBx)", vec![13]),
    ] {
        let hits = match_pattern(lang, src, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{lang:?} {pattern:?}: sg answers the literal verbatim row (m3a \
             oracle set) — the whole node text fails the meta grammar, so the \
             expando spelling must stay verbatim (FB-97B-1)"
        );
    }
    // Meta controls stay meta (whole-node validation must not over-apply).
    for (lang, src, pattern, want) in [
        (Language::Rust, F98_RUST_SRC, "µA + 1", vec![2]),
        (Language::Python, F98_PY_SRC, "µA + 1", vec![1, 2, 3, 11]),
        (Language::Rust, F98_RUST_SRC, "µ_ + 1", vec![2]),
    ] {
        let hits = match_pattern(lang, src, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{lang:?} {pattern:?}: valid meta tail still normalizes to the $ \
             semantics (m3a control / m1 oracle)"
        );
    }
    // Literal negative controls keep their empty sets (m3a agreement cells).
    for (lang, src, pattern) in [
        (Language::Rust, F98_RUST_SRC, "µBx + 1"),
        (Language::Rust, F98_RUST_SRC, "µabc + 1"),
        (Language::Rust, F98_RUST_SRC, "µµµµA + 1"),
        (Language::Rust, F98_RUST_SRC, "f(µµ)"),
        (Language::Java, F98_JAVA_JNAME_SRC, "µBx + 1"),
    ] {
        let hits = match_pattern(lang, src, pattern).unwrap();
        assert!(
            hits.is_empty(),
            "{lang:?} {pattern:?}: sg answers empty on this corpus (rust \
             literal binaries / no-row corpora) — must stay empty"
        );
    }
}

/// RED (F-97A-1 layer 2): sg answers the literal verbatim row for a 1-run
/// MixedCase $-token (`$Bx`-class) under BOTH spellings in every probed
/// language — the $ spelling preprocesses to the same expando-literal parse
/// (m3a2 oracle sets). The pass-54 literal lane was scoped to $-name languages
/// (js/ts/php-lowercase), so the subject answered silent [] in rust/py/go/c/
/// cpp/php/ruby/swift/kotlin/csharp/java. 2-run/3-run MixedCase (`$$Bx` /
/// `$$$Bx`) are sg-EMPTY everywhere (probed rc1) and lowercase-led runs keep
/// their registered classes — none may start answering.
#[test]
fn f98_dollar_spelled_mixed_tail_literals_answer_like_sg() {
    for (lang, src, pattern, want) in [
        (Language::Rust, F98_RUST_SRC, "k($Bx)", vec![13]),
        (Language::Rust, F98_RUST_SRC, "f(\"$Ax\")", vec![14]),
        (Language::Rust, F98_RUST_SRC, "$Bx", vec![12, 13]),
        (Language::Python, F98_PY_SRC, "$Bx + 1", vec![11]),
        (Language::Python, F98_PY_SRC, "k($Bx)", vec![12]),
        (Language::Python, F98_PY_SRC, "f(\"$Ax\")", vec![13]),
        (Language::Python, F98_PY_SRC, "$Bx", vec![11, 12]),
        (Language::Go, F98_GO_SRC, "k($Bx)", vec![15]),
        (Language::Go, F98_GO_SRC, "$Bx + 1", vec![14]),
        (Language::C, F98_C_SRC, "$Bx + 1", vec![12]),
        (Language::C, F98_C_SRC, "f(\"$Ax\")", vec![14]),
        (Language::C, F98_C_SRC, "$Bx", vec![12, 13]),
        (Language::Cpp, F98_CPP_SRC, "k($Bx)", vec![13]),
        (Language::Cpp, F98_CPP_SRC, "$Bx", vec![12, 13]),
        (Language::Php, F98_PHP_SRC, "k($Bx)", vec![13]),
        (Language::Php, F98_PHP_SRC, "$Bx", vec![12, 13]),
        (Language::Ruby, F98_RUBY_SRC, "k($Bx)", vec![12]),
        (Language::Ruby, F98_RUBY_SRC, "$Bx + 1", vec![11]),
        (Language::Swift, F98_SWIFT_SRC, "k($Bx)", vec![13]),
        (Language::Kotlin, F98_KOTLIN_SRC, "k($Bx)", vec![13]),
        (Language::CSharp, F98_CSHARP_SRC, "k($Bx)", vec![14]),
        (Language::CSharp, F98_CSHARP_SRC, "$Bx + 1", vec![13]),
        // java has NO expando: the $ spelling is the literal text itself.
        (Language::Java, F98_JAVA_JNAME_SRC, "k($Bx)", vec![14]),
        (Language::Java, F98_JAVA_JNAME_SRC, "$Bx + 2", vec![13]),
    ] {
        let hits = match_pattern(lang, src, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{lang:?} {pattern:?}: sg answers the literal verbatim row under \
             the $ spelling too (m3a2 oracle set) — the MixedCase token is \
             sg-literal code, never a silent empty"
        );
    }
    // c's expando-ident CALL doc: sg rc1 (registered §33.4 residual genus —
    // the c pattern gate rejects the expando-ident call doc, probed rc1-empty
    // this round too). The subject's answer here joins the REGISTERED
    // over-answer pair (`f($A)` / `f(𐐀A)`, CNR §33.4): visible hits on the
    // same row set as cpp, never a silent empty. Pinned so the class stays
    // answer-shaped while the reconciler owns the §33.4 refusal port.
    let hits = match_pattern(Language::C, F98_C_SRC, "k($Bx)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![13],
        "c k($Bx): answers the call row like cpp — the registered §33.4 \
         c call-doc over-answer genus (sg rc1; loud/visible direction)"
    );
    // csharp's bare-ident pattern doc is sg-refused (sg answers [] for bare
    // `µBx`/`µAble` — probed m3a cells 100-103). The subject's literal lane
    // has the SAME pre-existing bare-ident genus for PLAIN idents (this
    // round's probe: csharp `zz` sg [] / subject {9} on the same corpus), so
    // the mixed-tail spelling rides the existing lane behavior — visible
    // hits, never silent, consistent with the plain-ident twin.
    let hits = match_pattern(Language::CSharp, F98_CSHARP_SRC, "$Bx").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![13, 14],
        "csharp bare $Bx: rides the pre-existing bare-ident literal-lane \
         genus (ident nodes on the µBx rows; same shape as plain `zz` -> \
         subject line 9 vs sg [])"
    );
    // 2-run/3-run MixedCase are sg-EMPTY everywhere (probed rc1 on every
    // language, m3a2) — the inverse rewrite must not touch them.
    for (lang, src, pattern) in [
        (Language::Rust, F98_RUST_SRC, "$$Bx"),
        (Language::Rust, F98_RUST_SRC, "$$$Bx"),
        (Language::Python, F98_PY_SRC, "$$Bx"),
        (Language::Python, F98_PY_SRC, "$$$Bx"),
    ] {
        let hits = match_pattern(lang, src, pattern).unwrap();
        assert!(
            hits.is_empty(),
            "{lang:?} {pattern:?}: sg rc1 (no matches, m3a2) — multi-run \
             MixedCase must not start answering"
        );
    }
    // js/ts already answered these through the pass-54/69a lane (m3a2 cells
    // 53-56 SG_EXACT) — spelling-invariance control that must not regress.
    for pattern in ["k($Bx)", "$Bx + 1"] {
        let hits = match_pattern(Language::JavaScript, F98_JAVA_JNAME_SRC, pattern).unwrap();
        assert!(
            !hits.is_empty(),
            "js {pattern:?}: the existing $-name literal lane answers the \
             $Bx rows (m3a2 control) — must keep answering"
        );
    }
}

/// RED (F97X-0072): sg's preprocess maps EVERY `$` of a run to the expando
/// char, so a µ-run immediately followed by `$` composes into ONE combined run
/// in sg's expando space — py `µµ$A` preprocesses to `µµµA` = MultiCapture and
/// sg ANSWERS (m3c: py/rust `$A = µµ$A` oracle sets). The subject read the µµ
/// run and the `$` as separate tokens (tail scan stopped at `$`), routed the
/// mixed spelling silently empty while its `$`-twin `$A = $$$A` fails LOUD
/// (registered census genus) — a twin-convergence break. The fix: compose the
/// combined run (µµ$A → $$$A semantics), so the mixed spelling folds to
/// exactly its twin's classes.
#[test]
fn f98_mu_before_dollar_composes_like_sg() {
    // Bare µ$A composes to the 2-run anonymous capture: sg answers every node
    // line (m3c rust {1..5}); the subject's $$A twin does the same.
    for (lang, src, composed, twin) in [
        (Language::Rust, F98_COMPOSE_RS, "µ$A", "$$A"),
        (Language::Python, F98_COMPOSE_PY, "µ$A", "$$A"),
    ] {
        let hits = match_pattern(lang, src, composed).unwrap();
        assert_eq!(
            klass_lines(&hits),
            klass_lines(&match_pattern(lang, src, twin).unwrap()),
            "{lang:?} {composed:?}: µ-before-$ composes — must equal the \
             combined-run $ twin (m3c: sg answers every node line)"
        );
        assert!(
            !hits.is_empty(),
            "{lang:?} {composed:?}: sg ANSWERS the composed 2-run reading \
             (m3c oracle set) — the composition must not stay silent"
        );
    }
    // Statement composition folds to the twin's CLI class: the µµ$A face must
    // be census-loud EXACTLY like its $$$A twin (twin convergence; sg answers
    // the MultiCapture reading — the registered $$$-loud residual stands).
    for (lang, composed, twin) in [
        (Language::Python, "$A = µµ$A", "$A = $$$A"),
        (Language::Rust, "$A = µµ$A", "$A = $$$A"),
        (Language::Python, "µµ$A = $A", "$$$A = $A"),
    ] {
        assert_eq!(
            native_pattern_answerable(lang, composed),
            native_pattern_answerable(lang, twin),
            "{lang:?} {composed:?}: the composed spelling must be answerable \
             exactly when its $ twin is (both loud at the CLI ingress)"
        );
        assert!(
            !native_pattern_answerable(lang, composed),
            "{lang:?} {composed:?}: root/statement multi-meta is the twin's \
             registered loud class — must not walk silent-empty (F97X-0072)"
        );
    }
    // Expression composition: µ$A + 1 ≡ $$A + 1 (m3c sg {3}/{4} rows).
    for (lang, src) in [
        (Language::Rust, F98_COMPOSE_RS),
        (Language::Python, F98_COMPOSE_PY),
    ] {
        assert_eq!(
            klass_lines(&match_pattern(lang, src, "µ$A + 1").unwrap()),
            klass_lines(&match_pattern(lang, src, "$$A + 1").unwrap()),
            "{lang:?} µ$A + 1: composed 2-run in expression must equal its \
             $$A + 1 twin (m3c sg oracle set)"
        );
    }
    // A µ-run followed by `$` whose COMBINED run is 4+ is NOT meta (sg's
    // n>=4 literal rule, m3c: sg rc1-empty): the composition must not fire.
    // The registered $-spelled 4-run loud genus (97A §4.2) stands unchanged.
    assert!(
        match_pattern(Language::Rust, F98_COMPOSE_RS, "µµµ$A + 1")
            .unwrap()
            .is_empty(),
        "rust µµµ$A + 1: combined 4-run is sg-literal (no composition may \
         fire; m3c sg rc1-empty fold)"
    );
}

/// RED (F-97A-2): sg 0.45.2 rc8-REFUSES the unbalanced `}`-tail patterns in
/// js/ts (and ruby/swift) for EVERY spelling — including `$`-less (`q }`) and
/// µ-spelled (`µA }`, js has no expando) faces (m3b oracle: rc8). The
/// census-loud backstop only consulted the language-aware fragment gate for
/// `$`-carrying patterns, so the $-less faces walked silent ok-empty. The
/// pass-90/91 precedent governs: the census must rule $-less faces too. The
/// `]`/`)` tails and the py/rust/go/java `}` tails are sg-ACCEPTED-empty
/// (probed) and MUST stay answerable.
#[test]
fn f98_dollar_less_bracket_tail_census_governs_like_sg() {
    // sg rc8: unanswerable — the census keeps the whole-query loud fold.
    for (lang, pattern) in [
        (Language::JavaScript, "q }"),
        (Language::JavaScript, "µA }"),
        (Language::TypeScript, "q }"),
        (Language::TypeScript, "µA }"),
        (Language::Ruby, "q }"),
        (Language::Swift, "q }"),
        (Language::JavaScript, "$A }"),
        (Language::TypeScript, "$A }"),
    ] {
        assert!(
            !native_pattern_answerable(lang, pattern),
            "{lang:?} {pattern:?}: sg 0.45.2 rc8 (m3b) — the census must \
             govern this face loud, not walk it silent-empty"
        );
    }
    // sg accepted-empty (lenient rc0/rc1): answerable — the walk's honest
    // empty IS the sg agreement; these MUST NOT move to loud.
    for (lang, pattern) in [
        (Language::JavaScript, "q ]"),
        (Language::JavaScript, "q )"),
        (Language::JavaScript, "µA ]"),
        (Language::TypeScript, "q ]"),
        (Language::Python, "q }"),
        (Language::Python, "q ]"),
        (Language::Rust, "q }"),
        (Language::Rust, "µA }"),
        (Language::Go, "q }"),
        (Language::Go, "µA }"),
        (Language::Java, "q }"),
        (Language::C, "q }"),
        (Language::Cpp, "q }"),
        (Language::Php, "q }"),
        (Language::Kotlin, "q }"),
        (Language::CSharp, "q }"),
    ] {
        assert!(
            native_pattern_answerable(lang, pattern),
            "{lang:?} {pattern:?}: sg accepts this tail empty (m3b) — must \
             stay answerable so the walk answers the honest empty"
        );
    }
}

// ---------------------------------------------------------------------------
// PASS 100 (r50) — FB-99A-1 / FB-99A-2 / FB-99B-1 pins. Oracle sets are the
// artifacts/conformance/pass100/matrix/*.jsonl cells (ast-grep 0.45.2,
// probed 2026-09-09, `--limit 1000` pinned on the subject side).
// ---------------------------------------------------------------------------

const F100_GLU_PY: &str = "\u{b5}A_ + 1\n\u{b5}A1 + 1\ny + 1\n\u{b5}\u{b5}\u{b5}$A + 1\n\u{b5}\u{b5}$A + 1\n$A\u{b5}\u{b5}B\nfoo\u{b5}\u{b5}\u{b5}$A\n$$$A + 1\n$$A + 1\n$A$$B\nfoo$$$A\n\u{b5}Ax\n";
const F100_GLU_GO: &str = "package main\n\nfunc main() {\n    \u{b5}A_ + 1\n    \u{b5}A1 + 1\n    y + 1\n    \u{b5}\u{b5}\u{b5}$A + 1\n    \u{b5}\u{b5}$A + 1\n    $A\u{b5}\u{b5}B\n    foo\u{b5}\u{b5}\u{b5}$A\n    $$$A + 1\n    $$A + 1\n    $A$$B\n    foo$$$A\n    \u{b5}Ax\n}\n";
const F100_GLU_SWIFT: &str = "func main() {\n    \u{b5}A_ + 1\n    \u{b5}A1 + 1\n    y + 1\n    \u{b5}\u{b5}\u{b5}$A + 1\n    \u{b5}\u{b5}$A + 1\n    $A\u{b5}\u{b5}B\n    foo\u{b5}\u{b5}\u{b5}$A\n    $$$A + 1\n    $$A + 1\n    $A$$B\n    foo$$$A\n    \u{b5}Ax\n}\n";
const F100_GLU_JS: &str = "    \u{b5}A_ + 1\n    \u{b5}A1 + 1\n    y + 1\n    \u{b5}\u{b5}\u{b5}$A + 1\n    \u{b5}\u{b5}$A + 1\n    $A\u{b5}\u{b5}B\n    foo\u{b5}\u{b5}\u{b5}$A\n    $$$A + 1\n    $$A + 1\n    $A$$B\n    foo$$$A\n    \u{b5}Ax\n";
const F100_GLU_JAVA: &str = "class T {\n    void m() {\n    \u{b5}A_ + 1;\n    \u{b5}A1 + 1;\n    y + 1;\n    \u{b5}\u{b5}\u{b5}$A + 1;\n    \u{b5}\u{b5}$A + 1;\n    $A\u{b5}\u{b5}B;\n    foo\u{b5}\u{b5}\u{b5}$A;\n    $$$A + 1;\n    $$A + 1;\n    $A$$B;\n    foo$$$A;\n    \u{b5}Ax;\n    }\n}\n";
const F100_RUBY_SRC: &str = "\u{b5}A?\n\u{b5}A!\n\u{b5}Ab?\n\u{b5}Ab!\nx = \u{b5}A?\ndef \u{b5}A?\nend\ndef \u{b5}A!\nend\ny = 1\n";

/// RED (FB-99A-1, pass 100): the error-glued `µµµ$A` / `µµ$A` token rows.
/// sg 0.45.2 parses the row's binary LHS as the named identifier FRAGMENT
/// (glued with the unnamed `$A` ERROR text) and its meta binds that fragment
/// (`metaVariables.single.A_ = "µµµ"` probed first-hand), so the meta
/// patterns answer the glued rows too. The subject's general lane dropped
/// these rows: the newer error recovery surfaces the `$A` ERROR as an extra
/// (is_extra, probed true) child of the binary, and `general_eq`'s child
/// alignment required exact child-count equality. sg's Smart strictness
/// skips candidate extras (comments AND error nodes) during alignment —
/// mirror that skip so the fragment binds sg-exactly.
#[test]
fn f100_error_glued_fragment_metas_bind_like_sg() {
    for (lang, src, want) in [
        (Language::Python, F100_GLU_PY, vec![1, 2, 3, 4, 5, 8, 9]),
        (Language::Go, F100_GLU_GO, vec![4, 5, 6, 7, 8, 11, 12]),
        (Language::Swift, F100_GLU_SWIFT, vec![2, 3, 4, 5, 6, 9, 10]),
    ] {
        for pattern in [
            "\u{b5}A_ + 1",
            "\u{b5}A1 + 1",
            "\u{b5}_AB + 1",
            "\u{b5}$A + 1",
            "$$A + 1",
        ] {
            let hits = match_pattern(lang, src, pattern).unwrap();
            assert_eq!(
                klass_lines(&hits),
                want,
                "{lang:?} {pattern:?}: sg answers the clean `+ 1` rows AND the \
                 error-glued rows (m1 oracle set) — the binary LHS fragment is \
                 the named node the meta binds, never a silent drop"
            );
        }
    }
}

/// RED (FB-99A-2, pass 100): in the NO-EXPANDO languages (js/ts/java — `$`
/// and µ are ordinary identifier characters) sg's whole-token semantics read
/// `µµµ$A`, `$AµµB`, `foo$$$A`-class tokens as ONE literal identifier: the
/// node text fails `extract_meta_var`, so sg answers those verbatim rows.
/// The subject mis-classified the embedded `$A`/`$$B` as metavariables —
/// substitution glued a placeholder INSIDE the identifier leaf (text compare
/// could never match) or the language-free ingress gate bailed loud.
#[test]
fn f100_no_expando_glued_idents_answer_literal_rows() {
    for (lang, src, cases) in [
        (
            Language::JavaScript,
            F100_GLU_JS,
            [
                ("\u{b5}\u{b5}\u{b5}$A + 1", vec![4u32]),
                ("\u{b5}\u{b5}$A + 1", vec![5]),
                ("\u{b5}\u{b5}\u{b5}$A", vec![4]),
                ("$A\u{b5}\u{b5}B", vec![6]),
                ("$A$$B", vec![10]),
                ("foo\u{b5}\u{b5}\u{b5}$A", vec![7]),
                ("foo$$$A", vec![11]),
            ],
        ),
        (
            Language::TypeScript,
            F100_GLU_JS,
            [
                ("\u{b5}\u{b5}\u{b5}$A + 1", vec![4u32]),
                ("\u{b5}\u{b5}$A + 1", vec![5]),
                ("\u{b5}\u{b5}\u{b5}$A", vec![4]),
                ("$A\u{b5}\u{b5}B", vec![6]),
                ("$A$$B", vec![10]),
                ("foo\u{b5}\u{b5}\u{b5}$A", vec![7]),
                ("foo$$$A", vec![11]),
            ],
        ),
        (
            Language::Java,
            F100_GLU_JAVA,
            [
                ("\u{b5}\u{b5}\u{b5}$A + 1", vec![6u32]),
                ("\u{b5}\u{b5}$A + 1", vec![7]),
                ("\u{b5}\u{b5}\u{b5}$A", vec![6]),
                ("$A\u{b5}\u{b5}B", vec![8]),
                ("$A$$B", vec![12]),
                ("foo\u{b5}\u{b5}\u{b5}$A", vec![9]),
                ("foo$$$A", vec![13]),
            ],
        ),
    ] {
        for (pattern, want) in cases {
            let hits = match_pattern(lang, src, pattern).unwrap();
            assert_eq!(
                klass_lines(&hits),
                want,
                "{lang:?} {pattern:?}: sg answers the whole-token literal row \
                 (m1 oracle set) — the token is one ordinary identifier in \
                 no-expando languages, never a metavariable"
            );
        }
    }
    // Routing: the whole-token literal class must not ingress-bail loud (sg
    // answers these faces) and must be answerable through the literal lane;
    // the multi-root spelling sg rc8s (`q µµµ$A`) stays census-loud.
    for pattern in [
        "\u{b5}\u{b5}\u{b5}$A",
        "$A\u{b5}\u{b5}B",
        "foo\u{b5}\u{b5}\u{b5}$A",
        "$A$$B",
        "foo$$$A",
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern:?}: sg 0.45.2 answers the js/ts/java literal faces — the \
             language-free gate must not fail closed (m1 oracle sets)"
        );
        assert!(
            native_pattern_answerable(Language::JavaScript, pattern),
            "{pattern:?}: single-identifier root parses — the literal lane must \
             be answerable in js"
        );
    }
    assert!(
        !native_pattern_answerable(Language::JavaScript, "q \u{b5}\u{b5}\u{b5}$A"),
        "q µµµ$A js: sg 0.45.2 rc8 (multi-root parse, m1) — must stay \
         census-loud, not walk silent-empty"
    );
}

/// RED (FB-99B-1, pass 100): tree-sitter-ruby folds the `?` method-call
/// suffix INTO the identifier node (`µA?` is one `call` whose identifier
/// text includes `?`), so sg's whole-node validation reads `µA?` as LITERAL
/// and answers the verbatim rows {1,5,6}. The subject's scan stopped at `?`
/// and folded `µA` into a meta + `?` junk → silent empty. The `!` spelling
/// is NOT a suffix in expression position (sg parses `µA!` as µA + `!` and
/// rc8s), so the `!` twin must keep its loud fold — never literal-answer.
#[test]
fn f100_ruby_question_suffix_literal_like_sg() {
    // sg answers the `?` faces verbatim (m2 oracle sets).
    for (pattern, want) in [
        ("\u{b5}A?", vec![1u32, 5, 6]),
        ("$A?", vec![1, 5, 6]),
        ("x = \u{b5}A?", vec![5]),
    ] {
        let hits = match_pattern(Language::Ruby, F100_RUBY_SRC, pattern).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{pattern:?}: sg answers the ruby `?`-suffix literal rows (m2 \
             oracle set) — the node text includes the suffix, so the token \
             must never fold to meta + junk"
        );
    }
    // Guards: lowercase-tail tokens stay literal today and must stay exact
    // (`µAb?`); the `!` twin keeps its sg-rc8-equivalent loud fold (the
    // search layer surfaces it — here the meta fold must answer nothing).
    let hits = match_pattern(Language::Ruby, F100_RUBY_SRC, "\u{b5}Ab?").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![3u32],
        "µAb?: pre-existing exact literal face must not move"
    );
    let hits = match_pattern(Language::Ruby, F100_RUBY_SRC, "\u{b5}A!").unwrap();
    assert!(
        klass_lines(&hits).is_empty(),
        "µA! ruby: sg rc8 (parse refuses, m2) — the fold must not start \
         literal-answering the `!` rows"
    );
    assert!(
        !native_pattern_answerable(Language::Ruby, "\u{b5}A!"),
        "µA! ruby: sg rc8 — the loud fold class must be kept"
    );
}

// ---------------------------------------------------------------------------
// PASS 102 (r52) — F-101A-1..4 pins. Oracle sets:
// artifacts/conformance/pass102/matrix/*.jsonl (ast-grep 0.45.2, probed
// 2026-09-08, `--limit 1000` pinned, faithful-shape fixtures: one candidate
// row per file so sg's cross-line error merges cannot contaminate a set).
// ---------------------------------------------------------------------------

/// One candidate row per fixture (pass-101a faithful-shape discipline).
fn f102_call_src(lang: Language, shape: &str) -> String {
    let row = match shape {
        "glued" => "q(µµµ$A)",
        "plain" => "q(b)",
        "glued2" => "q(µµµ$A, 1)",
        "empty" => "q()",
        other => panic!("unknown shape {other}"),
    };
    match lang {
        // go needs the package clause; the row lands on line 2.
        Language::Go => format!("package main\nfunc f() {{ {row} }}\n"),
        Language::Rust => format!("fn main() {{ {row}; }}\n"),
        Language::Java => format!("class T {{ void m() {{ {row}; }} }}\n"),
        Language::JavaScript | Language::TypeScript => format!("{row};\n"),
        _ => format!("{row}\n"),
    }
}

/// RED (F-101A-1): call-ARGUMENT meta-fragment binding. sg 0.45.2 parses a
/// glued argument row `q(µµµ$A)` (py/go/swift/rust/kotlin) as
/// `call > argument_list > (, identifier µµµ, ERROR($A) [extra], )` and its
/// Smart strictness skips the candidate extra during alignment — the meta
/// binds the identifier FRAGMENT (`metaVariables.single.A_ = "µµµ"`; m1
/// oracle). The subject's Call lane counted the extra in the argument list
/// (arity Exactly(1) vs 2 named children) and the row went silently
/// unanswered. The r50 is_extra skip lives in `general_eq`'s alignment only
/// (the binary `1 + µA_` face — pinned working below); the argument
/// collector needs the same candidate-extra rule.
#[test]
fn f102_call_argument_fragment_metas_bind_like_sg() {
    // NOTE (R-100-5 continuity): kotlin is deliberately ABSENT here. The
    // workspace tree-sitter-kotlin grammar recovers the whole
    // `fun main() { q(µµµ$A) }` line as ONE flat ERROR node (no call
    // container at all — probed this pass), so no sound structural rule can
    // bind the fragment until the fwcd grammar is adopted — the registered
    // kotlin drift genus (CNR 32.1 / R-100-5) extends to the call-argument
    // faces. sg 0.45.2 answers them {1}; recorded as a residual.
    for lang in [
        Language::Python,
        Language::Go,
        Language::Swift,
        Language::Rust,
    ] {
        // Glued argument: sg answers every meta spelling on the row (m1).
        let src = f102_call_src(lang, "glued");
        let want = vec![if lang == Language::Go { 2u32 } else { 1u32 }];
        for pattern in ["q(µA_)", "q($A_)", "q(µA)", "q($$A_)"] {
            let hits = match_pattern(lang, &src, pattern).unwrap();
            assert_eq!(
                klass_lines(&hits),
                want,
                "{lang:?} {pattern:?} on glued argument: sg answers the row \
                 with the fragment bound (m1 oracle) — never a silent drop"
            );
        }
        // Plain argument: sg binds A_ = "b" (m1 oracle: the meta answers the
        // clean row too).
        let src = f102_call_src(lang, "plain");
        let hits = match_pattern(lang, &src, "q(µA_)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{lang:?} q(µA_) on plain argument: sg binds the meta (m1 oracle)"
        );
        // Two-argument glued row: arity counts non-extra children (m1 oracle:
        // py `q(µA_, 1)` answers; kotlin too).
        let src = f102_call_src(lang, "glued2");
        let hits = match_pattern(lang, &src, "q(µA_, 1)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{lang:?} q(µA_, 1) on `q(µµµ$A, 1)`: sg answers (m1 oracle) — \
             the extra never takes an arity slot"
        );
    }
    // No-expando controls (m1): js/ts read `µµµ$A` as ONE identifier, so
    // sg's `q(µA_)` (a literal `µA_` callee-arg there) answers nothing while
    // the canonical meta spelling binds the identifier.
    for lang in [Language::JavaScript, Language::TypeScript] {
        let src = f102_call_src(lang, "glued");
        let hits = match_pattern(lang, &src, "q(µA_)").unwrap();
        assert!(
            klass_lines(&hits).is_empty(),
            "{lang:?} q(µA_): sg answers [] (literal `µA_` != `µµµ$A`, m1)"
        );
        let hits = match_pattern(lang, &src, "q($A_)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "{lang:?} q($A_): sg binds the whole identifier (m1 SG_EXACT)"
        );
    }
    // Binary-position control: the r50 general_eq extra-skip face must hold.
    let hits = match_pattern(Language::Python, "1 + µµµ$A\n", "1 + µA_").unwrap();
    assert_eq!(klass_lines(&hits), vec![1u32], "py `1 + µA_`: r50 fix face");
}

/// RED (F-101A-3): the Call-lane argument binding must distinguish
/// PARENTHESIZED calls from ruby's paren-less command calls. sg 0.45.2
/// answers `q(µA_)` ONLY on the parenthesized shape — on `q µµµ$A` the
/// pattern's `(` token has no candidate counterpart (the command call's
/// argument list has no paren children) and sg answers []. The subject
/// zipped the meta onto the command call's argument list and over-answered
/// (m3 oracle: subject {1} vs sg [] on `q µµµ$A` and `q x`).
#[test]
fn f102_ruby_paren_call_gate_like_sg() {
    // Paren-less rows: sg answers [] (m3 oracle) — the subject must too.
    for (src, row) in [("q µµµ$A\n", "glued"), ("q x\n", "plain")] {
        for pattern in ["q(µA_)", "q($A_)", "q(µA)", "q($$$A)", "q($$A)"] {
            let hits = match_pattern(Language::Ruby, src, pattern).unwrap();
            assert!(
                klass_lines(&hits).is_empty(),
                "ruby {pattern:?} on `{row}`: sg answers [] (m3 oracle) — the \
                 paren-less command call is a different node shape, never a \
                 meta binding site"
            );
        }
    }
    // Parenthesized rows: sg answers (m3 oracle SG_EXACT cells) — must keep
    // answering after the gate.
    for (src, want) in [("q(µµµ$A)\n", vec![1u32]), ("q(x)\n", vec![1u32])] {
        for pattern in ["q(µA_)", "q($A_)", "q(µA)"] {
            let hits = match_pattern(Language::Ruby, src, pattern).unwrap();
            assert_eq!(
                klass_lines(&hits),
                want,
                "ruby {pattern:?} on {src:?}: sg answers the parenthesized \
                 row (m3 oracle) — the gate must not over-refuse"
            );
        }
    }
    // Literal-arg control (m3): `q(x)` answers only the exact parenthesized
    // row — both sides agree today.
    let hits = match_pattern(Language::Ruby, "q x\n", "q(x)").unwrap();
    assert!(klass_lines(&hits).is_empty(), "ruby q(x) on `q x`: sg []");
}

/// RED (F-101A-2): multi-suffix spellings (`µA??`, `µA?!`, `$A??`, `$A?!`,
/// plain `zz??`) are sg-rc8 patterns in ruby ("Multiple AST nodes are
/// detected" — m2 oracle) but the `$`-less literal route literal-answered
/// the matching rows (µ≡$ twins converged INTO the fail-open). The
/// census/literal route must consult sg's parse gate: whatever sg refuses
/// per language goes loud; whatever sg accepts stays answerable — js
/// `µA??` (nullish-coalescing recovery, m2 SG_EXACT) keeps answering.
#[test]
fn f102_ruby_multi_suffix_gate_refuses_like_sg() {
    for pattern in ["µA??", "µA?!", "$A??", "$A?!", "zz??"] {
        assert!(
            !native_pattern_answerable(Language::Ruby, pattern),
            "ruby {pattern:?}: sg 0.45.2 rc8 (m2 oracle) — the multi-suffix \
             spelling must fold to the loud class, not literal-answer the row"
        );
    }
    // The single-suffix twins stay answerable (m2: `µA?`/`$A?`/`zz?` are
    // sg-accepted faces — the r50 fix must hold).
    for pattern in ["µA?", "$A?", "zz?"] {
        assert!(
            native_pattern_answerable(Language::Ruby, pattern),
            "ruby {pattern:?}: sg accepts the single-suffix face (m2) — the \
             gate must not over-refuse it"
        );
    }
    // Registered loud twins keep their class (m2 rc2-vs-rc8 registered).
    for pattern in ["µA!", "$A!?"] {
        assert!(
            !native_pattern_answerable(Language::Ruby, pattern),
            "ruby {pattern:?}: sg rc8 — registered loud fold must hold"
        );
    }
    // Language scoping: js `µA??` is sg-ANSWERED (m2 SG_EXACT — nullish
    // recovery) and must stay answerable; the gate is per-language truth.
    assert!(
        native_pattern_answerable(Language::JavaScript, "µA??"),
        "js µA??: sg answers the row (m2 SG_EXACT) — answerable"
    );
    // The bang-method spelling `zz!` is plain ruby syntax — sg parses it as
    // ONE call node and answers the row (probed 0.45.2 --debug-query: `call`
    // root) — so it stays answerable. The `µA!` rc8 comes from the meta
    // fold, not the `!` itself (asserted above via the registered twins).
    assert!(
        native_pattern_answerable(Language::Ruby, "zz!"),
        "ruby zz!: sg parses the bang-method spelling and answers — answerable"
    );
}

/// RED (F-101A-4): comment transparency on the whole-token literal route.
/// sg 0.45.2 parses a trailing line comment in the pattern as a real child
/// aligned text-exactly (Smart skips a trailing candidate comment the
/// pattern lacks), so `q(µAble) // c` answers the comment-carrying row (m4
/// oracle: js/ts hits, including the two-space whitespace variant where the
/// comment NODE text is equal). The subject rc2'd the whole class at the
/// census (placement gate) and the R3 comparator blocked every pattern-side
/// comment outside an argument container, so even the walk never saw the
/// face. The walk must align a pattern-trailing line comment text-exactly.
#[test]
fn f102_literal_trailing_comment_face_answers_like_sg() {
    let row = "q(µAble) // c\n";
    // Byte-identical row: sg hits {1} (m4 trail_line oracle).
    let hits = match_pattern(Language::JavaScript, row, "q(µAble) // c").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js q(µAble) // c: sg answers the comment-carrying row (m4 oracle)"
    );
    // Whitespace variant: sg hits (m4 `q(µAble)  // c` on the one-space row)
    // because the comment NODE text is equal — the alignment is node-level,
    // not byte-level.
    let hits = match_pattern(Language::JavaScript, row, "q(µAble)  // c").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js two-space pattern variant: sg answers (m4 oracle) — comment node \
         text alignment, never whole-statement bytes"
    );
    // Comment-free pattern keeps answering the comment-carrying row (m4
    // SG_EXACT control — Smart's trailing skip).
    let hits = match_pattern(Language::JavaScript, row, "q(µAble)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js q(µAble) on the comment row: pre-existing exact face must hold"
    );
    // Comment-free row: the required comment slot is absent — sg answers []
    // (m4 bare-row oracle).
    let hits = match_pattern(Language::JavaScript, "q(µAble)\n", "q(µAble) // c").unwrap();
    assert!(
        klass_lines(&hits).is_empty(),
        "js comment-slot pattern on bare row: sg answers [] (m4 oracle)"
    );
    // Bare-ident trailing-comment face: sg answers the identical row (m4
    // ident_trail_line oracle: hits {1} in js/ts).
    let hits = match_pattern(Language::JavaScript, "µAble // c\n", "µAble // c").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js µAble // c on the identical row: sg answers (m4 oracle)"
    );
}

// ---------------------------------------------------------------------------
// PASS 105 (r55 remediation) — FB-104A-1 / FB-104A-2 / FB-104A-3 / FB-104A-4
// at the match_pattern / ingress level. Probe matrices:
// artifacts/conformance/pass105/matrix/m{1..4}.jsonl (oracle 0.45.2 ATTACHED,
// subject f85c58e7fc0bb604 pre-fix, both service lanes, --limit 1000).
// ---------------------------------------------------------------------------

/// RED (FB-104A-1): sg 0.45.2's swift grammar spells `LHS <binop> q(args)` as
/// ONE call whose CALLEE is the additive/multiplicative compound — the inner
/// `q(...)` is never a standalone matchable call, so clean call patterns
/// answer [] on every arithmetic-binary row (m2 oracle: int/float/string LHS
/// x +,-,*,/ — INCLUDING the clean rows `1 + q(1)` / `1.5 + q(1)`). The
/// subject's call walk recovered the rhs call and over-answered (80 fail-open
/// cells). Equality keeps a real nested call child and a call on the LEFT of
/// the operator is the matchable call — both controls must keep answering.
/// The python control pins the language scope (py keeps a normal binary
/// expression with a matchable call child).
#[test]
fn f105_swift_arithmetic_binary_compound_callee_call_not_matchable() {
    let binop_rows = [
        "1 + q(\u{b5}\u{b5}\u{b5}$A)",
        "1 - q(\u{b5}\u{b5}\u{b5}$A)",
        "1 * q(\u{b5}\u{b5}\u{b5}$A)",
        "1 / q(\u{b5}\u{b5}\u{b5}$A)",
        "let y = 1 + q(\u{b5}\u{b5}\u{b5}$A)",
        "1 + q(1)",
        "1.5 + q(1)",
        "\"s\" + q(\u{b5}\u{b5}\u{b5}$A)",
    ];
    for row in binop_rows {
        let hits = match_pattern(Language::Swift, &format!("{row}\n"), "q($A)").unwrap();
        assert!(
            hits.is_empty(),
            "swift {row:?} x q($A): sg answers [] — the compound callee never \
             presents a matchable inner call (m2 oracle), got {:?}",
            klass_lines(&hits)
        );
    }
    // Equality keeps the real nested call child (m2 eq rows AGREE pre-fix).
    let hits = match_pattern(Language::Swift, "x == q(\u{b5}\u{b5}\u{b5}$A)\n", "q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "swift equality row keeps the matchable call child (m2 oracle)"
    );
    // A call on the LEFT of the operator is the matchable call (m2 oracle H).
    let hits = match_pattern(Language::Swift, "q(\u{b5}\u{b5}\u{b5}$A) - 1\n", "q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "swift call-left row keeps answering (m2 oracle)"
    );
    // Python control: normal binary parse — the call child answers (m2 H).
    let hits = match_pattern(Language::Python, "1 + q(\u{b5}\u{b5}\u{b5}$A)\n", "q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "python binary row keeps the matchable call child (m2 oracle)"
    );
}

/// RED (FB-104A-3): the general lane's bare-keyword guard refused EVERY
/// bare-identifier-led binary pattern (`x * q($A)` — first token alphabetic,
/// continuation operator-led, not `=`), so `needs_ast_grep_fallback` rc2'd
/// faces sg 0.45.2 answers on every ident-LHS row (m2: 6 languages x
/// ident rows, subject rc2 while sg answers H). The aligned compound-callee
/// face must be served (needs=false), the lane must bind the aligned leaves
/// (ident + operator + call), and leaf mismatches must stay honest empty.
#[test]
fn f105_bare_ident_lhs_binary_pattern_answers_sg_aligned() {
    // The ingress refusal is FB-104A-3's root: pre-fix this rc2'd the class.
    assert!(
        !needs_ast_grep_fallback("x * q($A)"),
        "x * q($A): sg answers ident-LHS binary faces (m2 oracle) — the bare \
         ident + operator continuation must be general-lane eligible, not \
         ingress-loud"
    );
    // Aligned face answers (swift compound-callee alignment, m2 oracle H).
    let hits = match_pattern(
        Language::Swift,
        "x * q(\u{b5}\u{b5}\u{b5}$A)\n",
        "x * q($A)",
    )
    .unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "swift x * q($A) on the aligned ident row: m2 oracle answers"
    );
    let hits = match_pattern(Language::Swift, "x + q(1)\n", "x + q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "swift x + q($A) on the clean ident row: m2 oracle answers"
    );
    // Leaf mismatches stay empty (m2 oracle: op mismatch / LHS mismatch []).
    let hits = match_pattern(
        Language::Swift,
        "x * q(\u{b5}\u{b5}\u{b5}$A)\n",
        "x + q($A)",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "swift op mismatch must answer []: {:?}",
        klass_lines(&hits)
    );
    let hits = match_pattern(
        Language::Swift,
        "1 * q(\u{b5}\u{b5}\u{b5}$A)\n",
        "x * q($A)",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "swift LHS mismatch must answer []: {:?}",
        klass_lines(&hits)
    );
    // The uniform cross-language binary parse (m2 ident rows, oracle H).
    for lang in [
        Language::Python,
        Language::Go,
        Language::Rust,
        Language::TypeScript,
        Language::Java,
    ] {
        let stmt = match lang {
            Language::Python | Language::Ruby => String::new(),
            _ => String::new(),
        };
        let _ = stmt;
        let hits = match_pattern(lang, "x * q(\u{b5}\u{b5}\u{b5}$A)\n", "x * q($A)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "{lang:?} x * q($A) on the aligned ident row: m2 oracle answers"
        );
    }
}

/// RED (FB-104A-2): rest-slot admission for plain calls. sg 0.45.2's rest
/// grid is UNIFORM across go/js/php/py/rb/rs/ts (m3 matrix, 189 cells): a
/// trailing rest sharing the list with singles binds >= 1 argument, a
/// non-trailing rest binds EXACTLY ZERO (the singles anchor the arity to the
/// single count — sg does not backtrack the split), and two rests answer
/// every arity. Pre-fix every mixed-slot spelling classified None and rc2'd.
#[test]
fn f105_plain_call_rest_slots_bind_like_sg() {
    let semi_rows = "q(1);\nq(1, 2);\nq(1, 2, 3);\n";
    let bare_rows = "q(1)\nq(1, 2)\nq(1, 2, 3)\n";
    for (lang, source, base) in [
        (Language::JavaScript, semi_rows, 0u32),
        (Language::TypeScript, semi_rows, 0),
        (Language::Go, semi_rows, 0),
        (Language::Rust, semi_rows, 0),
        (Language::Python, bare_rows, 0),
        (Language::Ruby, bare_rows, 0),
        (Language::Php, &format!("<?php\n{semi_rows}"), 1),
    ] {
        // (trailing rest >= 1, leading rest zero, mid rest zero, two rests
        // any arity, exact-arity controls)
        let cases: &[(&str, Vec<u32>)] = &[
            ("q($A, $$$B)", vec![2, 3]),
            ("q($$$A, $B)", vec![1]),
            ("q($A, $$$B, $C)", vec![2]),
            ("q($$$B, $$$A)", vec![1, 2, 3]),
            ("q($A, $B)", vec![2]),
            ("q($A, $B, $C)", vec![3]),
        ];
        for (pattern, want) in cases {
            let hits = match_pattern(lang, source, pattern).unwrap();
            let want: Vec<u32> = want.iter().map(|l| l + base).collect();
            assert_eq!(
                klass_lines(&hits),
                want,
                "{lang:?} {pattern:?}: sg 0.45.2 rest grid (m3 oracle, uniform \
                 across the 7 probed languages)"
            );
        }
        // Sole rest control: every arity (registered sg-exact face).
        let hits = match_pattern(lang, source, "q($$$A)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1 + base, 2 + base, 3 + base],
            "{lang:?} q($$$A): whole-list rest answers every arity (m3 oracle)"
        );
    }
}

/// The walk's paren-token contract that the index lane must never bypass
/// (FB-104A-4): a bare-rest pattern `q($$$A)` was INDEX-served (the
/// `call:q` signature rows cannot express the paren-token contract), so the
/// r52 paren gate never ran and the over-answer spread to swift brace calls
/// plus ruby brace/do/symbol/string/receiver rows (m4: sg answers [] on
/// every paren-less row, H only on the parenthesized rows). These
/// match_pattern-level assertions pin the walk-side contract the index
/// early-return skip leans on: sg-exact on both sides of the paren line.
#[test]
fn f105_parenless_rest_call_candidates_refused_like_sg() {
    // Ruby paren-less command/brace/do/symbol/string/receiver rows: sg [].
    for row in [
        "q x",
        "q { |a| a }",
        "q do |a|\n  a\nend",
        "q :x",
        "q \"s\"",
    ] {
        for pattern in ["q($$$A)", "obj.w($$$A)"] {
            let hits = match_pattern(Language::Ruby, &format!("{row}\n"), pattern).unwrap();
            assert!(
                hits.is_empty(),
                "ruby {row:?} x {pattern:?}: sg answers [] on the paren-less \
                 row (m4 oracle), got {:?}",
                klass_lines(&hits)
            );
        }
    }
    let hits = match_pattern(Language::Ruby, "obj.w x\n", "obj.w($$$A)").unwrap();
    assert!(
        hits.is_empty(),
        "ruby receiver-dot paren-less row: sg [] (m4 oracle)"
    );
    // Swift brace call: sg [] (m4 swbrace row).
    let hits = match_pattern(Language::Swift, "q { x }\n", "q($$$A)").unwrap();
    assert!(hits.is_empty(), "swift brace call row: sg [] (m4 oracle)");
    // Parenthesized controls answer (m4 rbparen/swparen oracle H).
    let hits = match_pattern(Language::Ruby, "q(1)\n", "q($$$A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "ruby parenthesized row answers (m4 oracle)"
    );
    let hits = match_pattern(Language::Swift, "q(1)\n", "q($$$A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "swift parenthesized row answers (m4 oracle)"
    );
}

/// RED (FB-106A-1 / FB-106B-1): a plain-call argument list of TWO `$$$` rests
/// sharing ONE name (`q($$$A, $$$A)`) was admitted by `parse_call_arg_slots`
/// (the pass-105 collision check covered Meta-vs-Rest only), and the two-rest
/// match arm bound BOTH rests to the identical whole-args text — every
/// arity >= 2 over-answered where sg 0.45.2 answers ONLY the 1-arg rows
/// (m1_samespace: 22 over cells across 11 languages). The same-name
/// collision must refuse like the registered rest+single hconf036 contract:
/// classify None -> ingress loud. Distinct-name two-rest lists and the
/// rest+single mirrors keep their registered classes.
#[test]
fn f107_same_name_two_rest_collision_refuses_ingress() {
    // The FB-106A-1 face itself: same-name two rests must classify None and
    // ingress-loud (pre-r55 posture; sg answers only 1-arg rows — m1 oracle).
    assert_eq!(
        classify_native("q($$$A, $$$A)"),
        None,
        "q($$$A, $$$A): two rests sharing one name have no sound binding — \
         the slot admission must refuse (FB-106A-1)"
    );
    assert!(
        needs_ast_grep_fallback("q($$$A, $$$A)"),
        "q($$$A, $$$A): the refused collision face must ingress-loud, never \
         re-namespace into an over-answering slot lane"
    );
    // The registered rest+single collision mirrors keep their refusal.
    assert_eq!(classify_native("q($$$A, $A)"), None, "hconf036 face");
    assert_eq!(classify_native("q($A, $$$A)"), None, "hconf036 mirror");
    // Distinct-name two rests keep the classified slot lane (m3/m1 controls).
    assert!(
        classify_native("q($$$A, $$$B)").is_some(),
        "distinct-name two rests must keep answering (m1: sg answers every \
         arity >= 1)"
    );
    assert!(
        !needs_ast_grep_fallback("q($$$A, $$$B)"),
        "distinct-name control must stay served"
    );
}

/// RED (FB-106A-3): swift's grammar folds a PREFIX-unary compound callee into
/// ONE call node (`-q(1)` parses `call_expression[prefix_expression "-q",
/// call_suffix "(1)"]` — tree-dumped 0.7.3 grammar, pass107) exactly like the
/// registered additive/multiplicative fold of FIX 2. The r55 refusal was
/// scoped to additive/multiplicative children, so prefix rows fell through to
/// `last_identifier_in_chain` and over-answered where sg 0.45.2 answers []
/// (m2_swiftprefix: unminus/unnot/amp x 5 patterns, 19 over cells). The
/// refusal keys on the folded SHAPE (a direct `prefix_expression` callee
/// child), never the operator text. `try q(1)` keeps a REAL inner call node
/// (try_expression wraps it) and must keep answering, exactly like sg.
#[test]
fn f107_swift_prefix_compound_callee_call_not_matchable() {
    for row in [
        "let r = -q(1)",
        "let r = !q(1)",
        "let r = &q(1)",
        "let r = -q(\u{b5}\u{b5}\u{b5}$A)",
    ] {
        for pattern in ["q($A)", "q(\u{b5}A)", "q($A_)", "q($$$A)"] {
            let hits = match_pattern(Language::Swift, &format!("{row}\n"), pattern).unwrap();
            assert!(
                hits.is_empty(),
                "swift {row:?} x {pattern:?}: sg answers [] — the prefix \
                 compound callee never presents a matchable inner call \
                 (m2_swiftprefix oracle), got {:?}",
                klass_lines(&hits)
            );
        }
    }
    // `try q(1)` keeps a real call child — sg ANSWERS it (m2/m106a trycall).
    let hits = match_pattern(Language::Swift, "let r = try q(1)\n", "q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "swift try-call row keeps the matchable inner call (oracle H)"
    );
    // A call on the LEFT of an operator keeps its real call node (m2 oracle H).
    let hits = match_pattern(Language::Swift, "let r = q(1) - 1\n", "q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "swift call-left row keeps answering (m2 oracle)"
    );
    // Plain-call control: untouched.
    let hits = match_pattern(Language::Swift, "let r = q(1)\n", "q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "swift plain call control must keep answering"
    );
    // Python control: `-q(1)` keeps a normal unary+call parse — answers.
    let hits = match_pattern(Language::Python, "y = -q(1)\n", "q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "python prefix-unary row keeps the matchable call (m2_swiftprefix \
         control languages all agree with sg)"
    );
}

/// RED (FB-106A-4): swift member-chain PATTERNS silently answered empty
/// (`a.b($A)` on `a.b(1)` — rc0 ok:true [] where sg 0.45.2 answers {1}).
/// Root cause: swift/kotlin grammars wrap each member link in a
/// `navigation_suffix` node (`navigation_expression[a, navigation_suffix
/// [. b]]`), which is absent from the member-kind table — the FAITHFUL
/// resolver vetoed the whole chain and `chain_tail_identifier` could not
/// find the tail, so `captures_for_node` returned None and push_match
/// dropped every candidate (m3_chains: chain2/chain3/chain4/bin_member x
/// meta patterns, sg answers each aligned row). sg is CONNECTOR token-exact:
/// `a.b($A)` must never answer a `?.` site and a path-length or head
/// mismatch must stay empty.
#[test]
fn f107_swift_member_chain_pattern_answers_sg_aligned() {
    let hits = match_pattern(Language::Swift, "let r = a.b(1)\n", "a.b($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "swift a.b($A) on the aligned chain row: sg answers {{1}} (m3_chains)"
    );
    let hits = match_pattern(Language::Swift, "let r = a.b.c(1)\n", "a.b.c($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "swift a.b.c($A) on the 3-chain row: sg answers {{1}} (m3_chains)"
    );
    let hits = match_pattern(Language::Swift, "let r = a.b.c.d(1)\n", "a.b.c.d($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "swift a.b.c.d($A) on the 4-chain row: sg answers {{1}} (m3_chains)"
    );
    let hits = match_pattern(Language::Swift, "let r = a.b(1) + q(2)\n", "a.b($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "swift a.b($A) on the binary-member row: sg answers {{1}} (m3_chains)"
    );
    // Connector token-exactness: a `.` template never answers a `?.` site.
    let hits = match_pattern(Language::Swift, "let r = a?.b(1)\n", "a.b($A)").unwrap();
    assert!(
        hits.is_empty(),
        "swift a.b($A) on the ?. row must stay empty (sg token-exact, \
         m3_chains swift_optchain EMPTY_AGREE): {:?}",
        klass_lines(&hits)
    );
    // Connector token-exactness extends to META receivers (oracle probed
    // pass107: $O.$M($$$A) and $O.$M($A) answer only the plain row on the
    // two-row corpus) — the receiver-binding fallback must veto the named-?
    // shape just like the path resolvers, or $O would bind the glued `a?`.
    let hits = match_pattern(
        Language::Swift,
        "let r = a?.b(1)\nlet s = a.b(1)\n",
        "$O.$M($$$A)",
    )
    .unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![2u32],
        "swift $O.$M($$$A) on the ?.+plain corpus answers ONLY the plain row \
         (oracle pass107): {:?}",
        klass_lines(&hits)
    );
    let hits = match_pattern(Language::Swift, "let r = a?.b(1)\n", "$O.$M($A)").unwrap();
    assert!(
        hits.is_empty(),
        "swift $O.$M($A) on the ?. row must stay empty (oracle pass107): {:?}",
        klass_lines(&hits)
    );
    // Path-length mismatch stays empty (sg: [a,b] never matches [a,b,c]).
    let hits = match_pattern(Language::Swift, "let r = a.b.c(1)\n", "a.b($A)").unwrap();
    assert!(
        hits.is_empty(),
        "swift a.b($A) on the 3-chain row must stay empty (m3_chains)"
    );
    // Head mismatch stays empty.
    let hits = match_pattern(Language::Swift, "let r = a.b(1)\n", "x.y($A)").unwrap();
    assert!(
        hits.is_empty(),
        "swift x.y($A) on the a.b row must stay empty (honest mismatch)"
    );
}

/// RED (FB-106A-7): ruby's binary expression node kind is `binary` — NOT
/// `binary_expression` — so `is_general_root_kind` refused every ruby
/// operator-continuation template and the face composed into the loud census
/// where sg 0.45.2 answers the aligned rows (m5_loudfarm: rb mul/add/member
/// rows LOUD_VS_HITS). The general lane's leaf unification then answers the
/// aligned faces and honestly empties on LHS/operator mismatch, exactly like
/// the other nine languages whose binary roots already end in
/// `_expression`/`_operator`.
#[test]
fn f107_ruby_operator_continuation_answers_sg_aligned() {
    let hits = match_pattern(Language::Ruby, "y = x * q(1)\n", "x * q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "ruby x * q($A) on the aligned mul row: sg answers {{1}} (m5_loudfam)"
    );
    let hits = match_pattern(Language::Ruby, "y = x + q(1)\n", "x + q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "ruby x + q($A) on the aligned add row: sg answers {{1}} (m5_loudfam)"
    );
    let hits = match_pattern(Language::Ruby, "y = x.y * q(1)\n", "x.y * q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "ruby x.y * q($A) on the member-LHS row: sg answers {{1}} (m5_loudfam)"
    );
    let hits = match_pattern(Language::Ruby, "y = x * y * q(1)\n", "x * y * q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "ruby x * y * q($A) on the aligned chain row: sg answers {{1}} \
         (m5_loudfam genus, 106a m4 aligned-chain cell)"
    );
    // Honest mismatches stay empty (m5_loudfam: op/LHS mismatches [] both).
    let hits = match_pattern(Language::Ruby, "y = x * q(1)\n", "x + q($A)").unwrap();
    assert!(
        hits.is_empty(),
        "ruby op mismatch must answer []: {:?}",
        klass_lines(&hits)
    );
    let hits = match_pattern(Language::Ruby, "y = x * q(1)\n", "z * q($A)").unwrap();
    assert!(
        hits.is_empty(),
        "ruby LHS mismatch must answer []: {:?}",
        klass_lines(&hits)
    );
    // JavaScript control: the same face was already sg-exact (registered).
    let hits = match_pattern(Language::JavaScript, "let y = x * q(1);\n", "x * q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "javascript operator-continuation control unchanged"
    );
}

/// RED (FB-106A-10): THREE-rest pure-meta lists (`q($$$A, $$$B, $$$C)`) were
/// refused by the pass-105 admission (which stopped at two rests), so the
/// faces rc2'd where sg 0.45.2 answers the >= 2-argument rows and empties on
/// 0/1-argument rows (m1_samespace: 22 loud cells across 11 languages; sg
/// grid: k rests with no singles answer exactly n >= k-1). The head rests
/// bind zero and the tail binds the whole argument text — the split is
/// unobservable in hit sets (106B INFO-3).
#[test]
fn f107_three_rest_lists_bind_like_sg() {
    let semi_rows = "q();\nq(1);\nq(1, 2);\nq(1, 2, 3);\n";
    let bare_rows = "q()\nq(1)\nq(1, 2)\nq(1, 2, 3)\n";
    for (lang, source, base) in [
        (Language::JavaScript, semi_rows, 0u32),
        (Language::TypeScript, semi_rows, 0),
        (Language::Python, bare_rows, 0),
        (Language::Ruby, bare_rows, 0),
        (Language::Php, &format!("<?php\n{semi_rows}"), 1),
    ] {
        let hits = match_pattern(lang, source, "q($$$A, $$$B, $$$C)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![3 + base, 4 + base],
            "{lang:?} q($$$A, $$$B, $$$C): sg answers only the 2- and 3-arg \
             rows (m1_samespace oracle: k rests answer n >= k-1)"
        );
        // Two-rest control unchanged: every non-empty arity.
        let hits = match_pattern(lang, source, "q($$$A, $$$B)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![2 + base, 3 + base, 4 + base],
            "{lang:?} q($$$A, $$$B) control: every arity >= 1 (m3 oracle)"
        );
        // Sole-rest control unchanged: every arity incl. zero.
        let hits = match_pattern(lang, source, "q($$$A)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1 + base, 2 + base, 3 + base, 4 + base],
            "{lang:?} q($$$A) control unchanged: every arity incl. zero \
             (m3 oracle)"
        );
    }
}

/// RED (FB-106A-7 php twin): a bare-identifier operator-continuation head in
/// PHP (`x * q($A)`) never reached the general lane — php patterns parse
/// behind sg's own `<?php ` pre-process tag and the plain build folded the
/// pattern into a text node, so the face composed into the loud census where
/// sg 0.45.2 answers the aligned row (m4_php_op phpbare_tag: sg {a.php:2}).
/// The `$`-headed operator faces already answer sg-exactly through the php
/// operand lane (m4_php_op tagged rows: HIT_AGREE) — the bare-ident twin
/// must join them via the tag-wrap build retry.
#[test]
fn f107_php_operator_continuation_answers_sg_aligned() {
    let hits = match_pattern(Language::Php, "<?php\n$y = x * q(1);\n", "x * q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![2u32],
        "php x * q($A) on the bare-ident row: sg answers {{2}} (m4_php_op \
         phpbare_tag oracle)"
    );
    // The $-headed faces keep their operand-lane answers (m4_php_op controls).
    let hits = match_pattern(Language::Php, "<?php\n$y = $x * q(1);\n", "$x * q($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![2u32],
        "php $x * q($A) control keeps the operand-lane answer (m4_php_op \
         phpmul_tag HIT_AGREE)"
    );
    let hits = match_pattern(Language::Php, "<?php\n$y = $x * q(1);\n", "$x + q($A)").unwrap();
    assert!(
        hits.is_empty(),
        "php op mismatch must answer [] (m4_php_op EMPTY_AGREE): {:?}",
        klass_lines(&hits)
    );
}

/// RED (108A-F1): a LITERAL head segment of a two-segment dotted pattern must
/// byte-equal the receiver text slice it is accepted against — sg 0.45.2
/// answers `a.b($X)` with NOTHING on computed-subscript receivers
/// (`a[0].b(1)` / `a["x"].b(1)`, js and ts; oracle grid probed 2026-09-08),
/// while the plain and whitespace-spaced identifier receivers answer. The
/// pre-fix `bind_nonfaithful_receiver` arm bound the literal head through
/// `capture_name(..).unwrap_or_default()` — an empty capture name, so no
/// comparison — and accepted every non-member receiver: a fail-open against
/// the oracle's empty answer set. Mutant killed: dropping the head
/// byte-equality check re-admits lines 2/3 and this test fails.
#[test]
fn f109_literal_head_two_segment_rejects_computed_receiver() {
    // Grid corpus (js): sg answers the plain + spaced rows ONLY ({1,4}).
    let js = "a.b(1);\na[0].b(1);\na[\"x\"].b(1);\na .b(1);\n";
    let hits = match_pattern(Language::JavaScript, js, "a.b($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32, 4],
        "js a.b($A): sg answers the identifier-receiver rows only; \
         subscript receivers are fail-open over-answers: {:?}",
        klass_lines(&hits)
    );
    // Same face on ts (oracle empty on both subscript rows).
    let ts = "a.b(1);\na[0].b(1);\na[\"x\"].b(1);\n";
    let hits = match_pattern(Language::TypeScript, ts, "a.b($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "ts a.b($A): sg answers the plain row only: {:?}",
        klass_lines(&hits)
    );
    // F62-3 wildcard head keeps the whole-text receiver binding (oracle:
    // `$O.b($Y)` answers a[0].b(1) / a["x"].b(1) / y.first().b(1)).
    let mixed = "a.b(1);\na[0].b(1);\na[\"x\"].b(1);\ny.first().b(1);\n";
    let hits = match_pattern(Language::JavaScript, mixed, "$O.b($Y)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32, 2, 3, 4],
        "js $O.b($Y): the wildcard receiver binds every non-member receiver \
         text (F62-3, must not regress): {:?}",
        klass_lines(&hits)
    );
    // Pattern-side subscript literal face stays answered (oracle: {1}).
    let hits = match_pattern(Language::JavaScript, "a[0].b(1);\n", "a[0].b($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js a[0].b($A) on the aligned subscript row: sg answers {{1}}: {:?}",
        klass_lines(&hits)
    );
    // Registered refusals stay refused: `?.` sites (F64-7), 3-segment length
    // mismatch, call receivers, sequence-computed callees.
    for (src, pat) in [
        ("a?.b(1);\n", "a.b($A)"),
        ("a.b.c(1);\n", "a.b($A)"),
        ("a[0].b.c(1);\n", "a.b.c($A)"),
        ("y.first().b(1);\n", "a.b($A)"),
        ("(0, a.b)(1);\n", "a.b($A)"),
    ] {
        let hits = match_pattern(Language::JavaScript, src, pat).unwrap();
        assert!(
            hits.is_empty(),
            "js {pat} on {src:?} must stay empty (oracle EMPTY): {:?}",
            klass_lines(&hits)
        );
    }
}

/// RED (CNR §39.2, agent 109B): a single-segment LITERAL callee must
/// byte-equal the candidate's whole callee text — sg 0.45.2 answers
/// `a($X)` with NOTHING on computed-subscript callees (`a[0](1)` /
/// `a["b"](1)`, js and ts; oracle grid probed 2026-09-08) while the
/// bare-identifier callee answers. The single-segment registered-flattening
/// arm (`call_target_path` → `last_identifier_in_chain`) flattens the
/// subscript callee `a["b"]` to the tail identifier `a`, and the capture
/// layer bound nothing for a literal segment, so the flattened lie was
/// accepted: a fail-open against the oracle's empty answer set (same genus
/// as 108A-F1, different arm). Mutant killed: dropping the whole-callee
/// byte-equality check re-admits lines 3/4 and this test fails.
#[test]
fn f109b_literal_single_segment_rejects_computed_receiver() {
    // Grid corpus (js): sg answers ONLY the bare-identifier callee row {1};
    // the dotted member row stays empty too (2-segment candidate, sg-empty).
    let js = "a(1);\na.b(1);\na[\"b\"](1);\na[0](1);\n";
    let hits = match_pattern(Language::JavaScript, js, "a($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js a($A): sg answers the bare-identifier callee only; member and \
         computed callees are fail-open over-answers: {:?}",
        klass_lines(&hits)
    );
    // Same face on ts (oracle empty on the subscript row).
    let ts = "a(1);\na[\"b\"](1);\n";
    let hits = match_pattern(Language::TypeScript, ts, "a($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "ts a($A): sg answers the bare-identifier callee only: {:?}",
        klass_lines(&hits)
    );
    // Registered refusals stay refused (oracle EMPTY): call receivers,
    // sequence-computed callees, 2-level members, `?.` sites (F64-7).
    for (src, pat) in [
        ("y.first()(1);\n", "a($A)"),
        ("(0, a.b)(1);\n", "a($A)"),
        ("a.b.c(1);\n", "a($A)"),
        ("a?.b(1);\n", "a($A)"),
    ] {
        let hits = match_pattern(Language::JavaScript, src, pat).unwrap();
        assert!(
            hits.is_empty(),
            "js {pat} on {src:?} must stay empty (oracle EMPTY): {:?}",
            klass_lines(&hits)
        );
    }
    // CNR §39.2 meta controls must hold byte-exactly: `$F($X)` binds the
    // flattened tail and answers both the subscript and call-receiver
    // callees; `$F($$$A)` keeps the subscript face (oracle HIT).
    for (src, pat) in [
        ("a[\"b\"](1);\n", "$F($A)"),
        ("y.first()(1);\n", "$F($A)"),
        ("a[\"b\"](1);\n", "$F($$$A)"),
    ] {
        let hits = match_pattern(Language::JavaScript, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "js {pat} on {src:?}: the registered single-segment meta face \
             must keep answering (oracle HIT): {:?}",
            klass_lines(&hits)
        );
    }
    // 110B-F2 pin: a LITERAL tail in the nonfaithful arm must byte-equal the
    // chain-tail identifier — head `$O` binds `a[0]`, literal tail `c` != `b`.
    // sg 0.45.2 answers [] (property is `b`; oracle probed 2026-09-08).
    // NOTE (first-hand mutant run, agent 111): disabling the tail byte-
    // equality does NOT flip this row — the walk-side `path_matches`
    // suffix-zip refuses the candidate ("b" != "c") before captures, and no
    // reachable face reaches the arm with a mismatched literal tail (the
    // suffix-zip subsumes the capture-side equality). The check stays as
    // fail-safe sg-contract documentation; this row pins the oracle face.
    {
        let hits = match_pattern(Language::JavaScript, "a[0].b(1);\n", "$O.c($A)").unwrap();
        assert!(
            hits.is_empty(),
            "js $O.c($A) on a[0].b(1): sg answers [] — the literal tail must \
             byte-equal the chain-tail identifier (110B-F2): {:?}",
            klass_lines(&hits)
        );
    }
    // 110B-F3 pin: the f109b report grid's tail-leak trio + the spaced
    // single-segment face, pinned against re-regression at the capture layer.
    for (src, pat) in [
        ("a.b(1);\n", "b($A)"),
        ("a[\"b\"](1);\n", "b($A)"),
        ("a[0](1);\n", "b($A)"),
    ] {
        let hits = match_pattern(Language::JavaScript, src, pat).unwrap();
        assert!(
            hits.is_empty(),
            "js {pat} on {src:?}: the flattened tail must NOT leak into a \
             mismatched literal (report grid, oracle EMPTY): {:?}",
            klass_lines(&hits)
        );
    }
    {
        let hits = match_pattern(Language::JavaScript, "a(1);\n", "a ($A)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "js pattern-side spacing is tolerated (report grid, oracle HIT): {:?}",
            klass_lines(&hits)
        );
    }
}

/// RED (110A-F1, agent 111): sg 0.45.2 refuses optional-CALL links for a
/// plain-call pattern — the anonymous `?.` token sits between the callee and
/// the argument list as an extra child of the call node, breaking sg's
/// exact-children match. LITERAL and META heads alike refuse (`a($X)`,
/// `$F($X)`, and the rest-slot `$F($$$A)` lane); `a.b?.(1)` and
/// `a?. /*c*/ (1)` refuse too. Oracle grid probed 2026-09-08
/// (/tmp/phase111/corpus). The registered `?.` MEMBER postures are pinned as
/// controls: `a?.b($X)` answers its aligned site; the `$F($X)` non-goal on
/// `a?.b(1)` (subject fail-closed, sg HIT — PASS 65) must NOT move. Mutant
/// killed: disabling the junction veto re-admits the refused rows and this
/// test fails.
#[test]
fn f111a_optional_call_link_refused_for_plain_call_patterns() {
    // `a?.(1)` / `a.b?.(1)` / `a?. /*c*/ (1)`: oracle EMPTY for every plain
    // pattern below (pre-fix the capture layer answered all of them).
    let js = "a?.(1);\na.b?.(1);\na?. /*c*/ (1);\n";
    for pat in ["a($A)", "$F($A)"] {
        let hits = match_pattern(Language::JavaScript, js, pat).unwrap();
        assert!(
            hits.is_empty(),
            "js {pat} on the optional-call corpus: sg answers [] — optional \
             CALL links are exact-children breaks (110A-F1): {:?}",
            klass_lines(&hits)
        );
    }
    // Rest-slot row (H-1 lane correction, 113): `$F($$$A)` is a SOLE rest —
    // `validate_argument_pattern` accepts it, so `arg_slots = None` and the
    // row rides the TEMPLATE lane → `push_match` → `capture_call_path`,
    // where the capture gate kills it (M1). The mixed-slot rows that ride
    // the walk_calls SLOTS arm live in f113c.
    let hits = match_pattern(Language::JavaScript, "a?.(1);\n", "$F($$$A)").unwrap();
    assert!(
        hits.is_empty(),
        "js $F($$$A) on a?.(1): sg answers [] — the junction veto must reach \
         the rest-slot lane: {:?}",
        klass_lines(&hits)
    );
    // Second grammar instance (ts).
    let hits = match_pattern(Language::TypeScript, "a?.(1);\n", "a($A)").unwrap();
    assert!(
        hits.is_empty(),
        "ts a($A) on a?.(1): sg answers []: {:?}",
        klass_lines(&hits)
    );
    // Registered controls that must NOT move:
    // - the `?.`-spelled pattern answers its aligned site (both engines HIT);
    let hits = match_pattern(Language::JavaScript, "a?.(1);\n", "a?.($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js a?.($A) on a?.(1): the optional-spelled pattern must keep \
         answering (oracle HIT): {:?}",
        klass_lines(&hits)
    );
    // - `a?.b($X)` answers `a?.b(1)` (better-than-registered HIT_AGREE);
    let hits = match_pattern(Language::JavaScript, "a?.b(1);\n", "a?.b($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js a?.b($A) on a?.b(1): registered HIT face must hold: {:?}",
        klass_lines(&hits)
    );
    // - `$F($X)` on `a?.b(1)` stays fail-closed (registered PASS-65 non-goal:
    //   sg HIT but the subject's refusal is the registered posture).
    let hits = match_pattern(Language::JavaScript, "a?.b(1);\n", "$F($A)").unwrap();
    assert!(
        hits.is_empty(),
        "js $F($A) on a?.b(1): the registered member-`?.` non-goal must stay \
         fail-closed: {:?}",
        klass_lines(&hits)
    );
}

/// RED (110A-F2, agent 111): a comment extra as a DIRECT child of the call
/// node (in the callee→`(` junction) breaks sg's exact-children match —
/// `a /*c*/ (1)` answers NOTHING for `a($X)` / `$F($X)` / `$F($$$A)`, and
/// `a.b /*c*/ (1)` refuses for `a.b($X)` / `$A.b($X)`. Comments OUTSIDE the
/// call node (`/*c*/ a(1)`, `a(1); // c`), INSIDE the argument list
/// (`a( /*c*/ 1)`, `a(1 /*c*/)`, `a(1 /*c*/, 2)`), and plain newlines
/// (`a\n(1)`) are transparent trivia both engines answer — pinned as
/// must-keep controls. Oracle grid probed 2026-09-08. Mutant killed:
/// disabling the junction veto re-admits the refused rows.
#[test]
fn f111b_call_junction_comment_extra_refused() {
    let js = "a /*c*/ (1);\na.b /*c*/ (1);\n";
    for pat in ["a($A)", "$F($A)", "a.b($A)", "$A.b($B)"] {
        let hits = match_pattern(Language::JavaScript, js, pat).unwrap();
        assert!(
            hits.is_empty(),
            "js {pat} on the junction-comment corpus: sg answers [] — a call \
             node's direct comment extra breaks the exact-children match \
             (110A-F2): {:?}",
            klass_lines(&hits)
        );
    }
    let hits = match_pattern(Language::JavaScript, "a /*c*/ (1);\n", "$F($$$A)").unwrap();
    assert!(
        hits.is_empty(),
        "js $F($$$A) on a /*c*/ (1): sg answers [] — rest-slot lane included: {:?}",
        klass_lines(&hits)
    );
    // Mixed slot list through the walk_calls SLOTS arm (H-2 label made
    // honest by 113): pre-113 this row held only via rest-arity masking
    // (n=1 refuses `a($A, $$$B)` before any junction rule); since PASS 113
    // the slots arm consults `call_junction_exact` directly (F-B1), so the
    // refusal is now the gate's — multi-arg kill rows live in f113c.
    let hits = match_pattern(Language::JavaScript, "a /*c*/ (1);\n", "a($A, $$$B)").unwrap();
    assert!(
        hits.is_empty(),
        "js a($A, $$$B) on a /*c*/ (1): sg answers [] — the slots-lane \
         junction guard must hold: {:?}",
        klass_lines(&hits)
    );
    let hits = match_pattern(Language::TypeScript, "a /*c*/ (1);\n", "a($A)").unwrap();
    assert!(
        hits.is_empty(),
        "ts a($A) on a /*c*/ (1): sg answers []: {:?}",
        klass_lines(&hits)
    );
    // Must-keep transparency controls (oracle HIT on every one).
    for (src, pat) in [
        ("/*c*/ a(1);\n", "a($A)"),
        ("a(1); // c\n", "a($A)"),
        ("a\n(1);\n", "a($A)"),
        ("a( /*c*/ 1);\n", "a($A)"),
        ("a(1 /*c*/);\n", "a($A)"),
        ("a(1 /*c*/, 2);\n", "a($A, $B)"),
        ("a(1, /*c*/ 2);\n", "a($A, $B)"),
    ] {
        let hits = match_pattern(Language::JavaScript, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "js {pat} on {src:?}: trivia the oracle tolerates must keep \
             answering (oracle HIT): {:?}",
            klass_lines(&hits)
        );
    }
    // The parenthesized callee is NOT the bare identifier: sg refuses
    // `(a /*c*/)(1)` for `a($X)` (callee text mismatch, oracle EMPTY).
    let hits = match_pattern(Language::JavaScript, "(a /*c*/)(1);\n", "a($A)").unwrap();
    assert!(
        hits.is_empty(),
        "js a($A) on (a /*c*/)(1): the parenthesized callee is not the bare \
         identifier — sg answers []: {:?}",
        klass_lines(&hits)
    );
}

/// RED (110A-F3, agent 111): a ts `type_arguments` SIBLING between the callee
/// and the argument list is a named extra child of the call node — sg
/// 0.45.2 refuses `a<number>(1)` for `a($X)` / `$F($X)` / `a($$A)` /
/// `$F($$$A)` and refuses `a.foo<number>(1)` for `a.foo($X)` / `$F($X)`.
/// Pattern-side type arguments answer their aligned sites in BOTH engines
/// (`a<number>($X)` on `a<number>(1)` and on `a<number>(1).b(2)`) — pinned
/// controls. Oracle grid probed 2026-09-08. Mutant killed: disabling the
/// junction veto re-admits the refused rows.
#[test]
fn f111c_type_arguments_sibling_refused_for_plain_call_patterns() {
    let ts = "a<number>(1);\na.foo<number>(1);\n";
    for pat in ["a($A)", "$F($A)", "a.foo($A)"] {
        let hits = match_pattern(Language::TypeScript, ts, pat).unwrap();
        assert!(
            hits.is_empty(),
            "ts {pat} on the type_arguments corpus: sg answers [] — the \
             type_arguments sibling breaks the exact-children match (110A-F3): {:?}",
            klass_lines(&hits)
        );
    }
    let hits = match_pattern(Language::TypeScript, "a<number>(1);\n", "a($$A)").unwrap();
    assert!(
        hits.is_empty(),
        "ts a($$A) on a<number>(1): sg answers [] (meta lane included): {:?}",
        klass_lines(&hits)
    );
    let hits = match_pattern(Language::TypeScript, "a<number>(1);\n", "$F($$$A)").unwrap();
    assert!(
        hits.is_empty(),
        "ts $F($$$A) on a<number>(1): sg answers [] — rest-slot lane included: {:?}",
        klass_lines(&hits)
    );
    // Pattern-side type arguments keep answering (oracle HIT both).
    for (src, pat) in [
        ("a<number>(1);\n", "a<number>($A)"),
        ("a<number>(1).b(2);\n", "a<number>($A)"),
    ] {
        let hits = match_pattern(Language::TypeScript, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "ts {pat} on {src:?}: the pattern-side type-args face must keep \
             answering (oracle HIT): {:?}",
            klass_lines(&hits)
        );
    }
}

/// RED (110A-F4, agent 111): sg's member-chain decomposition is COMMENT-
/// TRANSPARENT — `a /*c*/ .b(1)`, `a. /*c*/ b(1)`, `a /*c*/ . /*d*/ b(1)`
/// and the 3-link `a /*c*/ .b /*d*/ .c(1)` all ANSWER the plain dotted
/// patterns, and a meta head binds the CLEAN identifier text (sg binds
/// `$A` = `a` at the identifier's byte range, probed via --json
/// metaVariables). The faithful chain resolver vetoed on the comment child
/// and routed to the nonfaithful arm, whose raw receiver slice (`a /*c*/`)
/// failed the PASS 109 byte-equality — a silent under-answer (js AND ts).
/// Refusals that must NOT move: subscript receivers with comments
/// (`a /*c*/ [0].b(1)` literal head; sg []), `?.` links with comments
/// (`a?. /*c*/ b(1)`; sg []), and the F62-3 raw-text meta binding on the
/// subscript+comment shape (`$O.b($X)` answers with O = the raw slice,
/// oracle-matched). Mutant killed: disabling the trivia skip re-emptying
/// the answered rows fails this test.
#[test]
fn f111d_chain_internal_comments_are_faithful_trivia() {
    // sg answers ALL FOUR rows for `a.b($A)` ({1,2,3,4}).
    let js = "a /*c*/ .b(1);\na. /*c*/ b(1);\na /*c*/ . /*d*/ b(1);\na.b(1);\n";
    let hits = match_pattern(Language::JavaScript, js, "a.b($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32, 2, 3, 4],
        "js a.b($A): sg is comment-tolerant BETWEEN chain links; the \
         comment-bearing rows are silent under-answers (110A-F4): {:?}",
        klass_lines(&hits)
    );
    let hits = match_pattern(Language::TypeScript, "a /*c*/ .b(1);\n", "a.b($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "ts a.b($A) on a /*c*/ .b(1): sg answers (same genus, 2nd grammar): {:?}",
        klass_lines(&hits)
    );
    // 3-link chain, literal and meta heads: sg answers both (distinct names —
    // `$A.b.c($A)` would self-conflict and sg answers [] for it, probed).
    let js3 = "a /*c*/ .b /*d*/ .c(1);\n";
    for pat in ["a.b.c($A)", "$A.b.c($X)"] {
        let hits = match_pattern(Language::JavaScript, js3, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "js {pat} on {js3:?}: comment-transparent 3-link chain must answer \
             (oracle HIT): {:?}",
            klass_lines(&hits)
        );
    }
    // Meta head binds the CLEAN identifier text (sg metaVariables: A = "a").
    let hits = match_pattern(Language::JavaScript, "a /*c*/ .b(1);\n", "$A.b($B)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js $A.b($B) on a /*c*/ .b(1): meta head must keep answering: {:?}",
        klass_lines(&hits)
    );
    assert_eq!(
        hits[0].captures.get("A").map(String::as_str),
        Some("a"),
        "js $A binding must be the clean identifier text (sg binds `a`, not \
         the raw `a /*c*/` slice)"
    );
    // Refusals that must NOT move (oracle [] for the literal heads):
    for (src, pat) in [
        ("a /*c*/ [0].b(1);\n", "a.b($A)"),
        ("a?. /*c*/ b(1);\n", "a.b($A)"),
    ] {
        let hits = match_pattern(Language::JavaScript, src, pat).unwrap();
        assert!(
            hits.is_empty(),
            "js {pat} on {src:?}: subscript/`?.` links stay refused with \
             comments present (oracle EMPTY): {:?}",
            klass_lines(&hits)
        );
    }
    // F62-3 keep: `$O.b($X)` answers the subscript+comment receiver with the
    // RAW text binding (sg binds O = `a /*c*/ [0]`).
    let hits = match_pattern(Language::JavaScript, "a /*c*/ [0].b(1);\n", "$O.b($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js $O.b($A) on a /*c*/ [0].b(1): F62-3 raw-text meta binding must \
         keep answering (oracle HIT): {:?}",
        klass_lines(&hits)
    );
}

/// RED (112A-F2, agent 113): a junction comment on an OPTIONAL-MEMBER call
/// (`a?.b /*c*/ (1)`) breaks sg's exact-children match for `?.`-SPELLED
/// patterns too. The `simple_call` guard excludes `?.` spellings from
/// `capture_call_path`'s gate, so the optional lanes must consult the SAME
/// junction rule. Oracle grid 2026-09-08 (js+ts, first-hand): `a?.b($X)`,
/// `$A?.b($X)` and `a?.b($$$A)` (n=1 AND the arity-masked-in-$X n=2 row) all
/// answer [] on `a?.b /*c*/ (1)`. Controls that must NOT move: the clean
/// chain answers (`a?.b(1)` / `a?.b($A)`, js+ts) and trivia at the NEXT dot
/// link answers (`a?.b /*c*/ .c(1)` / `a?.b.c($A)` — the comment there is
/// inside the member callee, not the junction). Mutant killed: disabling the
/// optional lanes' junction consultation re-admits the refused rows.
#[test]
fn f113a_optional_spelled_patterns_refuse_junction_comment() {
    for lang in [Language::JavaScript, Language::TypeScript] {
        for pat in ["a?.b($A)", "$A?.b($B)"] {
            let hits = match_pattern(lang, "a?.b /*c*/ (1);\n", pat).unwrap();
            assert!(
                hits.is_empty(),
                "{lang:?} {pat} on a?.b /*c*/ (1): sg answers [] — the junction \
                 comment breaks the exact-children match for `?.`-spelled \
                 patterns too (112A-F2): {:?}",
                klass_lines(&hits)
            );
        }
    }
    // Rest-arg spelling: the n=1 row and the n=2 row (the latter is
    // arity-masked for the exactly-one spellings but live for `$$$A`).
    for src in ["a?.b /*c*/ (1);\n", "a?.b /*c*/ (1, 2);\n"] {
        let hits = match_pattern(Language::JavaScript, src, "a?.b($$$A)").unwrap();
        assert!(
            hits.is_empty(),
            "js a?.b($$$A) on {src:?}: sg answers [] — rest-arg junction face \
             (112A-F2): {:?}",
            klass_lines(&hits)
        );
    }
    // Must-keep controls (oracle HIT).
    for (lang, src, pat) in [
        (Language::JavaScript, "a?.b(1);\n", "a?.b($A)"),
        (Language::TypeScript, "a?.b(1);\n", "a?.b($A)"),
        (Language::JavaScript, "a?.b /*c*/ .c(1);\n", "a?.b.c($A)"),
    ] {
        let hits = match_pattern(lang, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "{lang:?} {pat} on {src:?}: the comment is chain-internal trivia \
             here, NOT the junction — sg answers (must keep): {:?}",
            klass_lines(&hits)
        );
    }
}

/// RED (112A-F1, agent 113): sg's `?.`-chain decomposition is comment-
/// transparent on the ANONYMOUS-token grammars — js `a /*c*/ ?.b(1)`,
/// `a /*c1*/ ?. /*c2*/ b(1)`, `a?. /*c*/ b(1)`, the 3-segment
/// `a /*c*/ ?.b.c(1)` and the mid-chain `a.b /*c*/ ?.c(1)` all ANSWER the
/// `?.`-spelled patterns (literal and meta heads), js AND ts for trivia
/// AFTER the `?.` link. Pre-fix `member_link_parts` let the comment occupy
/// the base/leaf slot and the two-slot veto silently refused (under-answer).
/// The ts faces where sg ITSELF refuses (trivia BEFORE the `?.` link under
/// tree-sitter-typescript's named `optional_chain` wrapper) are pinned
/// separately in `f113d`. Mutant killed: disabling the trivia skip re-empties
/// the answered rows.
#[test]
fn f113b_optional_chain_comments_are_decomposition_trivia() {
    // sg answers ALL FOUR rows for `a?.b($A)` (js).
    let js = "a /*c*/ ?.b(1);\na /*c1*/ ?. /*c2*/ b(1);\na?. /*c*/ b(1);\na?.b(1);\n";
    let hits = match_pattern(Language::JavaScript, js, "a?.b($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32, 2, 3, 4],
        "js a?.b($A): sg is comment-tolerant inside `?.` chains; the comment \
         rows are silent under-answers (112A-F1): {:?}",
        klass_lines(&hits)
    );
    // Double-meta arg spelling rides the same decomposition (oracle HIT).
    let hits = match_pattern(Language::JavaScript, "a /*c*/ ?.b(1);\n", "a?.b($$A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js a?.b($$A) on a /*c*/ ?.b(1): oracle HIT: {:?}",
        klass_lines(&hits)
    );
    // Meta head (distinct names): oracle HIT.
    let hits = match_pattern(Language::JavaScript, "a /*c*/ ?.b(1);\n", "$A?.b($B)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js $A?.b($B) on a /*c*/ ?.b(1): meta head rides the same chain \
         decomposition (oracle HIT): {:?}",
        klass_lines(&hits)
    );
    // Trivia AFTER the `?.` link answers in ts too (oracle HIT).
    // NOTE (113 scope): the 3+-segment `?.`-chain faces (`a /*c*/ ?.b.c(1)`
    // / `a?.b.c($A)`) are served by the general structural lane, not this
    // decomposer — observed separately in the phase113 report grid.
    let hits = match_pattern(Language::TypeScript, "a?. /*c*/ b(1);\n", "a?.b($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "ts a?.b($A) on a?. /*c*/ b(1): trivia after the `?.` link is \
         transparent in BOTH grammars (oracle HIT): {:?}",
        klass_lines(&hits)
    );
}

/// RED (112B-F1, agent 113): the walk_calls SLOTS arm answers mixed rest-slot
/// patterns (`a($A, $$$B)`, `a($$$A, $B)`, `a($$$A, $$$B)`) through
/// `push_match_with_captures` directly — `capture_call_path` (the PASS 111
/// junction gate's sole site) never runs, so junction-extra candidates the
/// arity admits were over-answers. CONFIRMED DYNAMICALLY against both engines
/// 2026-09-08 (was 112B's static prediction): subject HIT vs oracle [] on
/// every row below (js+ts; ts `type_arguments` row included). Arity mapping:
/// trailing rest + 1 single admits n>=2; non-trailing rest admits n==1; two
/// rests admit n>=1. Plain-call arity controls (oracle HIT) must NOT move.
/// Mutant killed: disabling the slots arm's junction consultation re-admits
/// every refused row — and the killer provably rides the `Some(slots)` branch
/// (the None arm routes push_match → capture gate, which 111 already gated).
#[test]
fn f113c_slots_arm_consults_call_junction_gate() {
    for lang in [Language::JavaScript, Language::TypeScript] {
        for (src, pat) in [
            ("a?.(1);\n", "a($$$A, $B)"),
            ("a?.(1);\n", "a($$$A, $$$B)"),
            ("a?.(1, 2);\n", "a($A, $$$B)"),
            ("a?.(1, 2);\n", "a($$$A, $$$B)"),
            ("a /*c*/ (1);\n", "a($$$A, $B)"),
            ("a /*c*/ (1);\n", "a($$$A, $$$B)"),
            ("a /*c*/ (1, 2);\n", "a($A, $$$B)"),
            ("a /*c*/ (1, 2);\n", "a($$$A, $$$B)"),
        ] {
            let hits = match_pattern(lang, src, pat).unwrap();
            assert!(
                hits.is_empty(),
                "{lang:?} {pat} on {src:?}: sg answers [] — the slots arm must \
                 consult the junction gate (112B-F1): {:?}",
                klass_lines(&hits)
            );
        }
    }
    for (src, pat) in [
        ("a<number>(1);\n", "a($$$A, $B)"),
        ("a<number>(1);\n", "a($$$A, $$$B)"),
        ("a<number>(1, 2);\n", "a($A, $$$B)"),
        ("a<number>(1, 2);\n", "a($$$A, $$$B)"),
    ] {
        let hits = match_pattern(Language::TypeScript, src, pat).unwrap();
        assert!(
            hits.is_empty(),
            "ts {pat} on {src:?}: sg answers [] — type-args junction face \
             (112B-F1): {:?}",
            klass_lines(&hits)
        );
    }
    // Plain-call arity controls: oracle HIT on every one (the slots
    // semantics themselves are registered PASS 105/107 faces).
    for (src, pat) in [
        ("a(1);\n", "a($$$A, $B)"),
        ("a(1);\n", "a($$$A, $$$B)"),
        ("a(1, 2);\n", "a($A, $$$B)"),
        ("a(1, 2);\n", "a($$$A, $$$B)"),
    ] {
        let hits = match_pattern(Language::JavaScript, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "js {pat} on {src:?}: plain-call slot arity face must keep \
             answering (oracle HIT): {:?}",
            klass_lines(&hits)
        );
    }
}

/// Guard rows for the 113 fixes (pass pre-fix BY DESIGN; the mutants must
/// make them fail). (a) ts grammar-position faces where sg ITSELF refuses a
/// commented `?.` link — trivia PRECEDING tree-sitter-typescript's NAMED
/// `optional_chain` wrapper stays refused for literal and meta heads (the
/// 2-segment OptionalCall decomposer's faces; the 3+-segment spellings are a
/// general-lane divergence recorded in the phase113 report), while ts trivia
/// AFTER the wrapper and ts DOTTED-chain trivia stay transparent (oracle
/// grid 2026-09-08). (b) the arity-masked junction
/// rows that hid F-B1 from 111's mutant protocol must STAY refused once the
/// slots arm is gated. Mutants killed: removing the ts veto answers the
/// refused rows; an arity regression in the slots arm breaks the masked rows.
#[test]
fn f113d_trivia_before_named_optional_wrapper_refused_and_arity_masks_hold() {
    // (a) ts: trivia before the named `optional_chain` wrapper refuses.
    // Rows 1-2 ride the 2-segment OptionalCall decomposer (mutant-killed by
    // removing the ts veto). The 3+-segment spellings (`a.b /*c*/ ?.c(1)` /
    // `a.b?.c($A)`, `a /*c*/ ?.b /*d*/ .c(1)` / `a?.b.c($A)`) are served by
    // the general structural lane, which ANSWERS them where sg refuses — an
    // out-of-scope observed divergence recorded in the phase113 report.
    for (src, pat) in [
        ("a /*c*/ ?.b(1);\n", "a?.b($A)"),
        ("a /*c*/ ?.b(1);\n", "$A?.b($B)"),
    ] {
        let hits = match_pattern(Language::TypeScript, src, pat).unwrap();
        assert!(
            hits.is_empty(),
            "ts {pat} on {src:?}: sg refuses a commented `?.` link at this \
             grammar position (trivia precedes the named optional_chain \
             wrapper) — must stay refused: {:?}",
            klass_lines(&hits)
        );
    }
    // ts transparency keeps: trivia after the wrapper; dotted-chain trivia.
    for (src, pat) in [
        ("a?. /*c*/ b(1);\n", "a?.b($A)"),
        ("a /*c*/ .b(1);\n", "a.b($A)"),
    ] {
        let hits = match_pattern(Language::TypeScript, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "ts {pat} on {src:?}: transparent trivia positions must keep \
             answering (oracle HIT): {:?}",
            klass_lines(&hits)
        );
    }
    // (b) arity-masked junction rows stay refused (F-B1 masks pre-fix).
    for (src, pat) in [
        ("a?.(1);\n", "a($A, $$$B)"),
        ("a?.(1, 2);\n", "a($$$A, $B)"),
        ("a /*c*/ (1);\n", "a($A, $$$B)"),
        ("a /*c*/ (1, 2);\n", "a($$$A, $B)"),
        ("a?.b /*c*/ (1, 2);\n", "a?.b($A)"),
        ("a?.b /*c*/ (1, 2);\n", "$A?.b($B)"),
    ] {
        let hits = match_pattern(Language::JavaScript, src, pat).unwrap();
        assert!(
            hits.is_empty(),
            "js {pat} on {src:?}: arity-masked junction row must stay refused \
             (oracle []): {:?}",
            klass_lines(&hits)
        );
    }
}

/// RED (CNR §39.9 residual 1, agent 113B): `required_pattern_literal` derived
/// the required byte literal from the RAW callee segments, so the `?.`
/// connector marker rode into the SIMD prefilter: `a?.b($X)` yielded `"a?"`
/// and the prefilter dropped `a /*c*/ ?.b(1);` — a file the walk ANSWERS
/// (f113b, library green) because a commented chain never contains contiguous
/// `a?` bytes. First-hand grid (phase113b, 23 cells, js+ts): sg treats a
/// trailing `?` in a callee-path segment as connector SYNTAX only, never
/// matchable file content — `a?.b($X)` refuses `a.b(1)` and answers only the
/// `?.` spelling (clean AND trivia-bearing), and `a?.($X)` refuses `a(1)` —
/// so the identifier bytes alone are the sound literal (the `?` is the first
/// byte of the connector, not of the segment's file content). Trim the
/// trailing `?` per segment before the length pick; a shorter literal is the
/// over-broad (sound) direction for a prefilter. Mutant: disable the trim →
/// this test FAILS with the exact `"a?"` / `"b?"` literals.
#[test]
fn f113b1_prefilter_literal_omits_optional_connector_marker() {
    // The registered residual faces: literal-head `?.`-spelled patterns.
    // Pre-fix each picked the connector-marked segment (`"a?"`).
    for pattern in ["a?.b($X)", "a?.b($$A)", "a?.b($$$A)"] {
        assert_eq!(
            ast_sgrep_lang::required_pattern_literal(pattern).as_deref(),
            Some("b"),
            "{pattern}: the `?.` connector marker must not ride into the \
             prefilter literal (dropped the sg-answering trivia face)"
        );
    }
    // Optional-call spelling: the sole segment is `a?` (the marker before
    // the call junction).
    assert_eq!(
        ast_sgrep_lang::required_pattern_literal("a?.($X)").as_deref(),
        Some("a"),
        "a?.($X): the optional-call marker must be trimmed from the literal"
    );
    // Multi-`?.` chain: every segment is trimmed before the pick (pre-fix
    // the last longest raw segment `"b?"` won).
    assert_eq!(
        ast_sgrep_lang::required_pattern_literal("a?.b?.c($X)").as_deref(),
        Some("c"),
        "a?.b?.c($X): every segment's connector marker must be trimmed"
    );
    // Hold-controls: `?`-free patterns keep their exact literals, and a
    // metavariable head still drops its (marked) segment.
    assert_eq!(
        ast_sgrep_lang::required_pattern_literal("a.b($X)").as_deref(),
        Some("b"),
        "?-free control keeps its exact literal"
    );
    assert_eq!(
        ast_sgrep_lang::required_pattern_literal("$X?.m($A)").as_deref(),
        Some("m"),
        "meta-head control keeps the clean tail-segment literal"
    );
    assert_eq!(
        ast_sgrep_lang::required_pattern_literal("$O.out.$M($A)").as_deref(),
        Some("out"),
        "f80a control keeps its exact literal"
    );
}

/// RED (114A-F1, agent 115): a `?.`-spelled chain with PROPERTY segments
/// (`a?.b?.c($X)` — middle link argument-free) classified to the GENERAL
/// structural lane, which consults NONE of the `call_junction_exact` /
/// `member_link_parts` gates — so junction-comment files were ANSWERED where
/// sg 0.45.2 refuses (js+ts+tsx first-hand grid 2026-09-08, 216 cells:
/// `a?.b?.c($X)`, `a?.b?.c($$A)`, `$A?.b?.c($X)` on `a?.b?.c /*c*/ (1)` all
/// subject [1] vs sg []; the 4-segment spelling `a?.b?.c?.d($X)` over-answered
/// `a?.b?.c?.d /*c*/ (1)` the same way; the ts twins of the mid-chain trivia
/// file `a?.b /*c1*/ ?. /*c2*/ c(1)` over-answered via the missing
/// named-`optional_chain` veto). The ONE rule: every `?.`-spelled chain
/// pattern rides the gated optional-chain machinery, which already refuses
/// these faces for ≤2-segment patterns (113/113B). Controls that must NOT
/// move: clean chains answer (3- and 4-segment), the js mid-chain trivia face
/// answers, the dotted twins keep their exact refusals/answers (zero dotted
/// movement), meta-head folding keeps answering, a property-vs-call mismatch
/// refuses (`a?.b(1)?.c(2)` under `a?.b?.c($X)`: sg []), and a call after a
/// property link answers (`a?.b?.c(1)?.d($X)`).
#[test]
fn f115a_property_segment_optional_chains_refuse_junction_and_trivia_faces() {
    // Junction-comment refusals (oracle [] on js AND ts; tsx tracks ts).
    for lang in [Language::JavaScript, Language::TypeScript] {
        for pat in ["a?.b?.c($X)", "a?.b?.c($$A)", "$A?.b?.c($X)"] {
            let hits = match_pattern(lang, "a?.b?.c /*c*/ (1);\n", pat).unwrap();
            assert!(
                hits.is_empty(),
                "{lang:?} {pat} on a?.b?.c /*c*/ (1): sg refuses — the junction \
                 comment breaks the exact-children match; the general-lane \
                 over-answer is 114A-F1: {:?}",
                klass_lines(&hits)
            );
        }
        // 4-segment spelling: the rule generalizes (sg refuses the same face).
        let hits = match_pattern(lang, "a?.b?.c?.d /*c*/ (1);\n", "a?.b?.c?.d($X)").unwrap();
        assert!(
            hits.is_empty(),
            "{lang:?} a?.b?.c?.d($X) on a?.b?.c?.d /*c*/ (1): sg refuses the \
             4-segment junction face too: {:?}",
            klass_lines(&hits)
        );
        // Trivia BETWEEN links: sg refuses ts/tsx (named `optional_chain`
        // wrapper veto, 113's grammar rule); the js twin ANSWERS (anonymous
        // `?.` token grammar — asserted after the loop).
        if lang == Language::TypeScript {
            let hits = match_pattern(lang, "a?.b /*c1*/ ?. /*c2*/ c(1);\n", "a?.b?.c($X)").unwrap();
            assert!(
                hits.is_empty(),
                "{lang:?} a?.b?.c($X) on a?.b /*c1*/ ?. /*c2*/ c(1): the ts \
                 named-wrapper veto must refuse the mid-chain trivia face \
                 (oracle [] for ts): {:?}",
                klass_lines(&hits)
            );
        }
    }
    // The js twin of the mid-chain trivia face must KEEP answering (oracle
    // HIT — anonymous `?.` token grammar).
    let hits = match_pattern(Language::JavaScript, "a?.b /*c*/ ?.c(1);\n", "a?.b?.c($X)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js a?.b?.c($X) on a?.b /*c*/ ?.c(1): sg answers the js mid-chain \
         comment face (must keep): {:?}",
        klass_lines(&hits)
    );
    // Clean faces keep answering, literal and meta heads, 3- and 4-segment.
    for (lang, src, pat) in [
        (Language::JavaScript, "a?.b?.c(1);\n", "a?.b?.c($X)"),
        (Language::TypeScript, "a?.b?.c(1);\n", "a?.b?.c($X)"),
        (Language::JavaScript, "a?.b?.c?.d(1);\n", "a?.b?.c?.d($X)"),
        (Language::JavaScript, "x(1)?.b?.c(2);\n", "$A?.b?.c($X)"),
        (Language::JavaScript, "x.y?.b?.c(2);\n", "$A?.b?.c($X)"),
        // Multi-call face: a call AFTER a property link answers (oracle HIT).
        (
            Language::JavaScript,
            "a?.b?.c(1)?.d(2);\n",
            "a?.b?.c(1)?.d($X)",
        ),
        // Mixed dotted/optional spellings (oracle HIT, both).
        (Language::JavaScript, "a.b?.c(1);\n", "a.b?.c($X)"),
        (Language::JavaScript, "a?.b.c(1);\n", "a?.b.c($X)"),
    ] {
        let hits = match_pattern(lang, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "{lang:?} {pat} on {src:?}: clean chain faces must keep answering \
             (oracle HIT): {:?}",
            klass_lines(&hits)
        );
    }
    // Property-vs-call mismatch refuses (oracle []: the pattern's `b` is a
    // plain member, the candidate's `b(1)` is a call — exact-children break).
    for lang in [Language::JavaScript, Language::TypeScript] {
        let hits = match_pattern(lang, "a?.b(1)?.c(2);\n", "a?.b?.c($X)").unwrap();
        assert!(
            hits.is_empty(),
            "{lang:?} a?.b?.c($X) on a?.b(1)?.c(2): pattern property vs \
             candidate call must refuse (oracle []): {:?}",
            klass_lines(&hits)
        );
    }
    // Dotted twins: ZERO movement (already sg-exact through the dotted lanes).
    for (lang, src, pat, want) in [
        (
            Language::JavaScript,
            "a.b.c /*c*/ (1);\n",
            "a.b.c($X)",
            vec![],
        ),
        (
            Language::TypeScript,
            "a.b.c /*c*/ (1);\n",
            "a.b.c($X)",
            vec![],
        ),
        (Language::JavaScript, "a.b.c(1);\n", "a.b.c($X)", vec![1u32]),
    ] {
        let hits = match_pattern(lang, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "{lang:?} {pat} on {src:?}: dotted twins must not move: {:?}",
            klass_lines(&hits)
        );
    }
    // 2-segment optional faces keep their registered postures (113/113B).
    let hits = match_pattern(Language::JavaScript, "a?.b(1);\n", "a?.b($A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js a?.b($A) clean must keep answering: {:?}",
        klass_lines(&hits)
    );
    let hits = match_pattern(Language::JavaScript, "a?.b /*c*/ (1);\n", "a?.b($A)").unwrap();
    assert!(
        hits.is_empty(),
        "js a?.b($A) junction face must keep refusing (f113a): {:?}",
        klass_lines(&hits)
    );
}

/// RED (114A-F2, agent 115): a 3+-segment `?.`-spelled pattern with a
/// `$$$`-rest argument list (`a?.b?.c($$$A)`) reached NEITHER a native lane
/// NOR the general lane — the ingress classifier refused the property
/// segments, `general_lane_supported` refused the rest-arg chain template,
/// and the query failed closed rc2-LOUD (`pattern requires structural
/// fallback`) on CLEAN js/ts files where sg 0.45.2 answers (first-hand grid
/// 2026-09-08: arities 1/2/3 HIT sg; the empty-list face too; meta heads
/// `$A?.b?.c($$$A)` equally loud vs sg HIT). The fix classifies the spelling
/// into the registered [`NativeKind::OptionalCallChain`] lane (language-
/// free), so the ingress stops failing closed and the gated walk answers
/// sg-exactly. Controls that must NOT move: the sole-rest arity mapping
/// (empty/1/2 all answer), the dotted-file connector refusal, the
/// ts-type-args refusal, the junction refusal, the 2-link rest face, and the
/// mixed-rest `$$$A, $B` spelling (no classification contract — stays
/// fail-closed per H-CONF-026's registered arity class).
#[test]
fn f115b_rest_arg_optional_chains_classify_native_and_answer_sg_exact() {
    // Ingress: the spelling must classify natively (stops the rc2 loud).
    for pat in ["a?.b?.c($$$A)", "$A?.b?.c($$$A)"] {
        let kind = classify_native(pat);
        assert!(
            matches!(kind, Some(NativeKind::OptionalCallChain { .. })),
            "{pat} must classify into the gated optional-chain lane (pre-fix \
             None → structural-fallback loud, 114A-F2): {kind:?}"
        );
        assert!(
            !needs_ast_grep_fallback(pat),
            "{pat} must stop failing closed at ingress (sg answers the clean \
             faces): needs_ast_grep_fallback said true"
        );
    }
    // Walk: sg-exact answers on clean files at every sole-rest arity.
    for (src, desc) in [
        ("a?.b?.c(1);\n", "n=1"),
        ("a?.b?.c(1, 2);\n", "n=2"),
        ("a?.b?.c();\n", "empty"),
    ] {
        for (lang, lang_name) in [(Language::JavaScript, "js"), (Language::TypeScript, "ts")] {
            let hits = match_pattern(lang, src, "a?.b?.c($$$A)").unwrap();
            assert_eq!(
                klass_lines(&hits),
                vec![1u32],
                "{lang_name} a?.b?.c($$$A) on {src:?} ({desc}): sg HITs the \
                 clean sole-rest face: {:?}",
                klass_lines(&hits)
            );
        }
    }
    // Meta head: sg HITs the clean face too.
    for lang in [Language::JavaScript, Language::TypeScript] {
        let hits = match_pattern(lang, "a?.b?.c(1, 2);\n", "$A?.b?.c($$$A)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "{lang:?} $A?.b?.c($$$A) clean: sg HITs (meta head): {:?}",
            klass_lines(&hits)
        );
    }
    // Refusals that must NOT move (oracle []).
    for (lang, src, pat) in [
        // Dotted file: the `?.` connector distinction is token-exact.
        (Language::JavaScript, "a.b.c(1, 2);\n", "a?.b?.c($$$A)"),
        (Language::TypeScript, "a.b.c(1, 2);\n", "a?.b?.c($$$A)"),
        // Junction comment (f115a's rule at rest arity).
        (
            Language::JavaScript,
            "a?.b?.c /*c*/ (1);\n",
            "a?.b?.c($$$A)",
        ),
        // ts type arguments in the candidate callee.
        (
            Language::TypeScript,
            "a?.b?.c<number>(1);\n",
            "a?.b?.c($$$A)",
        ),
        // Dotted pattern spelling refuses the optional file (connector
        // distinction, registered).
        (Language::JavaScript, "a?.b?.c(1);\n", "a.b.c($X)"),
    ] {
        let hits = match_pattern(lang, src, pat).unwrap();
        assert!(
            hits.is_empty(),
            "{lang:?} {pat} on {src:?}: must keep refusing (oracle []): {:?}",
            klass_lines(&hits)
        );
    }
    // 2-link rest face keeps answering (registered R-107-B clean leg).
    let hits = match_pattern(Language::JavaScript, "a?.b(1);\n", "a?.b($$$A)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js a?.b($$$A) clean must keep answering: {:?}",
        klass_lines(&hits)
    );
}

/// Guards (agent 115): the property-segment `?.` chain routing is
/// LANGUAGE-SCOPED to the optional machinery's registered grammar family
/// ({TypeScript, JavaScript} — where `member_link_parts`' trivia skip, the
/// named-wrapper veto, and `call_junction_exact` are oracle-proven). Every
/// OTHER language keeps the general structural lane the family rode pre-115,
/// byte-identical: kotlin/swift/rust spell `?.`/`?` natively and their clean
/// 3-segment faces answered sg-exactly through the general lane (first-hand
/// probe 2026-09-08: kt/swift/rust `a?.b?.c($X)` on clean files HIT-agree);
/// routing them into the optional walk would veto them into silent empties
/// (kotlin/swift navigation-suffix leaf checks, rust try-expression folding).
/// The answerable gate mirrors the walk: js/ts answerable, other languages
/// answerable exactly when the general template builds (census-loud when it
/// does not — the registered loud-under class for `$$$` spellings outside
/// the family).
#[test]
fn f115c_property_chain_routing_preserves_general_lane_fallbacks() {
    // Non-family languages keep answering the clean faces (general lane).
    for (lang, lang_name) in [
        (Language::Kotlin, "kotlin"),
        (Language::Swift, "swift"),
        (Language::Rust, "rust"),
    ] {
        let hits = match_pattern(lang, "a?.b?.c(1);\n", "a?.b?.c($X)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "{lang_name} a?.b?.c($X) clean: the general-lane fallback must keep \
             answering (pre-115 posture, oracle HIT): {:?}",
            klass_lines(&hits)
        );
    }
    // The answerable gate: family languages answerable; kotlin stays
    // answerable through the general template it actually walks.
    for lang in [Language::JavaScript, Language::TypeScript, Language::Kotlin] {
        assert!(
            native_pattern_answerable(lang, "a?.b?.c($X)"),
            "{lang:?} a?.b?.c($X) must stay answerable (the walk has an sg-exact \
             lane for it)"
        );
    }
    // A `$$$` spelling outside the family: no general template builds →
    // NOT answerable (the registered census-loud class; pre-115 it was the
    // language-free ingress loud).
    assert!(
        !native_pattern_answerable(Language::Kotlin, "a?.b?.c($$$A)"),
        "kotlin a?.b?.c($$$A): no general template builds — the face stays \
         fail-closed (census loud), never a silent empty"
    );
    assert!(
        !native_pattern_answerable(Language::Rust, "a?.b?.c($$$A)"),
        "rust a?.b?.c($$$A): same census-loud class"
    );
}

/// RED (116A-F1, agent 117): the general-lane arm for `?.`-chain PROPERTY
/// faces OUTSIDE {TypeScript, JavaScript} (the byte-identical preservation
/// arm 115 shipped) consults NONE of the chain gates — kotlin and swift
/// junction-comment faces and kotlin mid-chain/receiver-trivia faces
/// OVER-ANSWERED where sg 0.45.2 refuses (first-hand grid 2026-09-08: kt
/// `a?.b?.c($X)` on `a?.b?.c /*c*/ (1);` / `a?.b /*c*/ ?.c(1);` /
/// `a /*c*/ ?.b?.c(1);` and swift on the junction twin: subject [1] vs sg
/// []; swift mid-trivia already refused). The fix gates that arm with the
/// ONE junction rule (every consumed call level's callee→`(` gap must be
/// exact-children) plus the mid-link trivia veto, applied to the CANDIDATE
/// nodes the general lane matched. Controls that must NOT move: kt/swift
/// clean faces answer, statement-level trivia BEFORE the chain answers, the
/// swift mid-trivia refusal holds, and mid-link junction/trivia twins of
/// 4-link chains refuse on kt too.
#[test]
fn f117a_non_family_chain_faces_obey_the_junction_and_trivia_gates() {
    // Over-answers pre-fix (oracle []): must refuse after the fix.
    for (lang, lang_name, src) in [
        (Language::Kotlin, "kotlin", "a?.b?.c /*c*/ (1);\n"),
        (Language::Kotlin, "kotlin", "a?.b /*c*/ ?.c(1);\n"),
        (Language::Kotlin, "kotlin", "a /*c*/ ?.b?.c(1);\n"),
        (Language::Swift, "swift", "a?.b?.c /*c*/ (1);\n"),
        // 4-link property chain, junction on the MID call link (kt rides the
        // general arm — sg refuses the same face it refuses on js/ts).
        (Language::Kotlin, "kotlin", "a?.b?.c /*c*/ (1)?.d(2);\n"),
        (Language::Swift, "swift", "a?.b?.c /*c*/ (1)?.d(2);\n"),
        // Mid-chain trivia on a 4-link chain (kt over-answers pre-fix).
        (Language::Kotlin, "kotlin", "a?.b /*c*/ ?.c(1)?.d(2);\n"),
    ] {
        let pat = if src.contains("?.d(2)") {
            "a?.b?.c($X)?.d($Y)"
        } else {
            "a?.b?.c($X)"
        };
        let hits = match_pattern(lang, src, pat).unwrap();
        assert!(
            hits.is_empty(),
            "{lang_name} {pat} on {src:?}: sg refuses the commented-chain face \
             — the general-lane arm must consult the junction/trivia gates \
             (116A-F1): {:?}",
            klass_lines(&hits)
        );
    }
    // Controls that must NOT move (oracle grid).
    for (lang, lang_name, src) in [
        (Language::Kotlin, "kotlin", "a?.b?.c(1);\n"),
        (Language::Swift, "swift", "a?.b?.c(1);\n"),
        // Statement-level trivia before the chain: sg answers.
        (Language::Kotlin, "kotlin", " /*c*/ a?.b?.c(1);\n"),
        (Language::Swift, "swift", " /*c*/ a?.b?.c(1);\n"),
    ] {
        let hits = match_pattern(lang, src, "a?.b?.c($X)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "{lang_name} on {src:?}: clean/pre-chain-trivia faces must keep \
             answering through the gated general arm (oracle HIT): {:?}",
            klass_lines(&hits)
        );
    }
    // Swift mid-trivia keeps refusing (pre-fix posture, oracle []).
    let hits = match_pattern(Language::Swift, "a?.b /*c*/ ?.c(1);\n", "a?.b?.c($X)").unwrap();
    assert!(
        hits.is_empty(),
        "swift mid-trivia face must keep refusing: {:?}",
        klass_lines(&hits)
    );
}

/// RED (116A-F2, agent 117): MIXED-REST argument lists in `?.`-spelled
/// chains (`a?.b?.c($A, $$$B)`, `a?.b?.c($$$A, $$$B)`, `a?.b?.c($$$A, $B)`)
/// refused classification (`parse`-side `Some(_) => return None`), so the
/// spellings failed closed rc2-LOUD on clean js/ts files where sg 0.45.2
/// ANSWERS (first-hand grid 2026-09-08). The registered plain-call rest-slot
/// semantics (PASS 105: trailing rest + single ⇒ n ≥ 2; k ≥ 2 rests ⇒ n ≥
/// k-1; non-trailing rest ⇒ n == 1) carry over EXACTLY — sg answers the
/// chain faces at the same arities it answers `q($A, $$$B)`. Controls that
/// must NOT move: dotted chains keep refusing every multi-rest spelling (sg
/// refuses `a.b.c($A, $$$B)` on arity 2 AND 3), same-name rest/single combos
/// stay census-loud (H-CONF-026 genus), sole-rest keeps its 115 arity grid,
/// and the non-family census-loud disposition holds (sg's kt/swift answers
/// on these spellings stay the registered miss — never silent-empty).
#[test]
fn f117b_mixed_rest_optional_chains_classify_and_answer_sg_exact() {
    // Ingress: the spellings must classify natively (stops the rc2 loud).
    for pat in [
        "a?.b?.c($A, $$$B)",
        "a?.b?.c($$$A, $$$B)",
        "a?.b?.c($$$A, $B)",
    ] {
        let kind = classify_native(pat);
        assert!(
            matches!(kind, Some(NativeKind::OptionalCallChain { .. })),
            "{pat} must classify into the gated optional-chain lane (pre-fix \
             None → structural-fallback loud, 116A-F2): {kind:?}"
        );
        assert!(
            !needs_ast_grep_fallback(pat),
            "{pat} must stop failing closed at ingress (sg answers the clean \
             js/ts faces)"
        );
    }
    // Trailing rest + single: sg refuses n ≤ 1, answers n ≥ 2 (js+ts).
    for (lang, lang_name) in [(Language::JavaScript, "js"), (Language::TypeScript, "ts")] {
        for (src, want, desc) in [
            ("a?.b?.c();\n", vec![], "n=0"),
            ("a?.b?.c(1);\n", vec![], "n=1"),
            ("a?.b?.c(1, 2);\n", vec![1u32], "n=2"),
            ("a?.b?.c(1, 2, 3);\n", vec![1u32], "n=3"),
        ] {
            let hits = match_pattern(lang, src, "a?.b?.c($A, $$$B)").unwrap();
            assert_eq!(
                klass_lines(&hits),
                want,
                "{lang_name} a?.b?.c($A, $$$B) on {src:?} ({desc}): trailing-\
                 rest arity grid must match sg's plain-call semantics: {:?}",
                klass_lines(&hits)
            );
        }
    }
    // js arity 4 too (grid row).
    let hits = match_pattern(
        Language::JavaScript,
        "a?.b?.c(1, 2, 3, 4);\n",
        "a?.b?.c($A, $$$B)",
    )
    .unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js trailing rest binds n=4: {:?}",
        klass_lines(&hits)
    );
    // Two rests: sg answers n ≥ 1 (ar0 refuses).
    for (src, want, desc) in [
        ("a?.b?.c();\n", vec![], "n=0"),
        ("a?.b?.c(1);\n", vec![1u32], "n=1"),
        ("a?.b?.c(1, 2, 3);\n", vec![1u32], "n=3"),
    ] {
        let hits = match_pattern(Language::JavaScript, src, "a?.b?.c($$$A, $$$B)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "js a?.b?.c($$$A, $$$B) on {src:?} ({desc}): k≥2 rest arity grid \
             (n ≥ k-1): {:?}",
            klass_lines(&hits)
        );
    }
    // Non-trailing rest: sg answers ONLY the 1-arg row (PASS 105 semantics).
    for (src, want, desc) in [
        ("a?.b?.c(1);\n", vec![1u32], "n=1"),
        ("a?.b?.c(1, 2);\n", vec![], "n=2"),
    ] {
        let hits = match_pattern(Language::JavaScript, src, "a?.b?.c($$$A, $B)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "js a?.b?.c($$$A, $B) on {src:?} ({desc}): non-trailing rest \
             anchors the arity to the single count: {:?}",
            klass_lines(&hits)
        );
    }
    // MID-LINK rest slots: the contract rides every consumed call level.
    let hits = match_pattern(
        Language::JavaScript,
        "a?.b(1, 2)?.c(3);\n",
        "a?.b($A, $$$B)?.c($X)",
    )
    .unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "js a?.b($A, $$$B)?.c($X) on the mid-link arity-2 face: sg HITs: {:?}",
        klass_lines(&hits)
    );
    // Same-name rest/single combo stays census-loud at ingress (sg [] is the
    // envelope-only leg of the registered H-CONF-026 loud posture).
    assert!(
        needs_ast_grep_fallback("a?.b?.c($$$A, $A)"),
        "same-name chain rest combos must keep the census-loud envelope"
    );
    // Dotted multi-rest spellings are OUT of F2's scope: the plain-call
    // slots arm (§39.10, `NativeKind::Call` with a dotted callee path)
    // classifies `a.b.c($A, $$$B)` independently of this lane, and sg's own
    // answer grid there (probe3 2026-09-08: sg [] on the dotted file at
    // arities 2 AND 3, both spellings) is a PRE-EXISTING library/CLI
    // boundary — registered as the 117-Info row, not touched by this lane.
    // Sole-rest arity grid keeps holding (115's contract).
    for src in ["a?.b?.c();\n", "a?.b?.c(1, 2);\n"] {
        let hits = match_pattern(Language::JavaScript, src, "a?.b?.c($$$A)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "js sole rest on {src:?} must keep answering: {:?}",
            klass_lines(&hits)
        );
    }
    // Non-family census-loud disposition holds for the new spellings.
    for lang in [Language::Kotlin, Language::Swift] {
        assert!(
            !native_pattern_answerable(lang, "a?.b?.c($A, $$$B)"),
            "{lang:?} multi-rest chain: no general template builds — the face \
             stays fail-closed (registered class), never a silent empty"
        );
    }
}

/// PASS 117 (116A-F3, agent 117 — RESOLVED AS REGISTERED ENVELOPE, the
/// original admission plan REVERTED on parse evidence): template-literal
/// property links (``a?.`b`?.c($X)``) keep the fail-closed loud envelope.
/// The grid-first probes went DEEPER than the 116A finding: sg 0.45.2
/// answers the template faces (js+ts, semi and no-semi), but this
/// workspace's TSX parse shapes the same source DIFFERENTLY — the template
/// link parses as a connector-less glued sibling above a zero-width
/// property_identifier (first-hand CST dump: member[a?. <empty prop>] +
/// template_string sibling) — so the chain walk cannot see sg's 2-link
/// model, and emulating the glue would need new walker machinery while the
/// cross/meta boundaries below already hold. Fail-closed loud (never
/// silent-empty) is the exact registered posture; CNR §39.12 carries the
/// full grid + form-1 retry predicate. Pins that MUST hold: the
/// identifier-shape refusal at classify (the loud root), the cross controls
/// (an identifier or meta link never matches a template leaf — sg []), and
/// the kt/swift general-lane template faces keep answering.
#[test]
fn f117c_template_literal_property_links_keep_registered_envelope() {
    let pat = "a?.`b`?.c($X)";
    let kind = classify_native(pat);
    assert_eq!(
        kind, None,
        "{pat} must keep the identifier-shape refusal (the fail-closed loud \
         envelope; admission is parse-blocked, CNR §39.12): {kind:?}"
    );
    assert!(
        !native_pattern_answerable(Language::JavaScript, pat)
            && !native_pattern_answerable(Language::TypeScript, pat),
        "{pat} stays UNANSWERABLE for js/ts (the census loud envelope — loud, \
         never silent-empty)"
    );
    assert!(native_pattern_answerable(Language::Kotlin, pat));
    // Cross controls (oracle []): identifier pattern link vs template leaf,
    // meta link vs template leaf — both refuse via the leaf gate.
    for (lang, src, p) in [
        (Language::JavaScript, "a?.`b`?.c(1);\n", "a?.b?.c($X)"),
        (Language::TypeScript, "a?.`b`?.c(1);\n", "a?.b?.c($X)"),
        (Language::JavaScript, "a?.`b`?.c(1);\n", "a?.$B?.c($X)"),
        (Language::TypeScript, "a?.`b`?.c(1);\n", "a?.$B?.c($X)"),
    ] {
        let hits = match_pattern(lang, src, p).unwrap();
        assert!(
            hits.is_empty(),
            "{lang:?} {p} on {src:?}: cross-shape link must refuse (oracle []): {:?}",
            klass_lines(&hits)
        );
    }
    // kt/swift keep their general-lane answers on the template face
    // (oracle HIT; pre-115 posture — the pattern never classifies).
    for (lang, lang_name) in [(Language::Kotlin, "kotlin"), (Language::Swift, "swift")] {
        let hits = match_pattern(lang, "a?.`b`?.c(1);\n", pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "{lang_name} template face must keep answering through the general lane (oracle HIT): {:?}",
            klass_lines(&hits)
        );
    }
}

/// PASS 117 (116A-F4, agent 117 — RESOLVED AS REGISTERED ENVELOPE with the
/// sg QUIRK documented): numeric property links (`a?.0?.b($X)`) keep the
/// fail-closed loud envelope. First-hand sg grid (2026-09-08, the
/// registered quirk): js answers the SEMICOLON-LESS spelling and refuses
/// the `;`-ful twin; ts refuses BOTH — sg's tree-sitter disambiguation of
/// `?.0` flips with the statement terminator and the file extension. This
/// workspace's TSX parse of the no-semi spelling is an ERROR extra plus a
/// number-rooted chain (CST dump) — admission would leave that face
/// SILENTLY EMPTY where sg ANSWERS, which the fail-closed constitution
/// forbids — so the identifier-shape refusal stands. Encoding the observed
/// grid exactly means: every numeric face loud, the quirk is sg's, never
/// "fixed" away. CNR §39.12 carries the grid + form-1 retry predicate
/// (re-probe if sg changes its pin OR the workspace ever parses `?.0`
/// into sg's chain shape).
#[test]
fn f117d_numeric_property_links_keep_registered_envelope() {
    let pat = "a?.0?.b($X)";
    let kind = classify_native(pat);
    assert_eq!(
        kind, None,
        "{pat} must keep the identifier-shape refusal (the fail-closed loud \
         envelope; admission would go silent-empty on the js no-semi face \
         where sg answers, CNR §39.12): {kind:?}"
    );
    assert!(
        !native_pattern_answerable(Language::JavaScript, pat)
            && !native_pattern_answerable(Language::TypeScript, pat),
        "{pat} stays UNANSWERABLE for js/ts (the census loud envelope — loud, \
         never silent-empty; the js no-semi sg-answer miss is part of the \
         registered class)"
    );
    // Cross control (oracle []): identifier pattern link vs numeric leaf.
    for lang in [Language::JavaScript, Language::TypeScript] {
        let hits = match_pattern(lang, "a?.0?.b(1)\n", "a?.b?.c($X)").unwrap();
        assert!(
            hits.is_empty(),
            "{lang:?} a?.b?.c($X) on the numeric file: must refuse (oracle []): {:?}",
            klass_lines(&hits)
        );
    }
}

/// PASS 117 (116B-F2/CCB-2 + F5 pins, agent 117): mid-link CALL junction
/// comments were never junction-checked — the optional chain walker
/// consulted `call_junction_exact` on the TERMINAL node only, so
/// `a?.b?.c($X)?.d($Y)` ANSWERED `a?.b?.c /*c*/ (1)?.d(2)` (js+ts, and the
/// pre-existing all-call 3+-segment lane `a?.b($X)?.c($Y)` answered
/// `a?.b /*c*/ (1)?.c(2)` the same way) where sg 0.45.2 refuses (first-hand
/// grid 2026-09-08). The ONE junction rule now extends to EVERY consumed
/// call level: mid-link hops AND the head-args hop (sg also refuses
/// `a($P)?.b?.c($X)` on `a /*c*/ (0)?.b?.c(1)`, probe3 — spelled with a
/// META head argument because a literal `a(0)` head never classifies).
/// F5 pins (probed, sg-ANSWERS — the meta mid-link admission is sg-EXACT
/// as-is, registered evidence): `$B`/`$$A`/`$$$A` mid-links bind identifier
/// leaves on js+ts.
#[test]
fn f117e_mid_link_call_junctions_refuse_and_meta_mid_links_pin() {
    // Consumed-call-level junction faces (oracle []): must refuse.
    for (lang, lang_name, src, pat) in [
        (
            Language::JavaScript,
            "js",
            "a?.b?.c /*c*/ (1)?.d(2);\n",
            "a?.b?.c($X)?.d($Y)",
        ),
        (
            Language::TypeScript,
            "ts",
            "a?.b?.c /*c*/ (1)?.d(2);\n",
            "a?.b?.c($X)?.d($Y)",
        ),
        // The pre-existing all-call lane shares the ONE rule.
        (
            Language::JavaScript,
            "js",
            "a?.b /*c*/ (1)?.c(2);\n",
            "a?.b($X)?.c($Y)",
        ),
        (
            Language::TypeScript,
            "ts",
            "a?.b /*c*/ (1)?.c(2);\n",
            "a?.b($X)?.c($Y)",
        ),
        // Head-args junction (oracle [] js+ts, probe3 meta-arg spelling).
        (
            Language::JavaScript,
            "js",
            "a /*c*/ (0)?.b?.c(1);\n",
            "a($P)?.b?.c($X)",
        ),
        (
            Language::TypeScript,
            "ts",
            "a /*c*/ (0)?.b?.c(1);\n",
            "a($P)?.b?.c($X)",
        ),
    ] {
        let hits = match_pattern(lang, src, pat).unwrap();
        assert!(
            hits.is_empty(),
            "{lang_name} {pat} on {src:?}: EVERY consumed call level obeys the \
             junction rule (sg refuses): {:?}",
            klass_lines(&hits)
        );
    }
    // Clean controls keep answering (oracle HIT).
    for (lang, src, pat) in [
        (
            Language::JavaScript,
            "a?.b?.c(1)?.d(2);\n",
            "a?.b?.c($X)?.d($Y)",
        ),
        (
            Language::TypeScript,
            "a?.b?.c(1)?.d(2);\n",
            "a?.b?.c($X)?.d($Y)",
        ),
        (Language::JavaScript, "a?.b(1)?.c(2);\n", "a?.b($X)?.c($Y)"),
        (Language::JavaScript, "a(0)?.b?.c(1);\n", "a($P)?.b?.c($X)"),
    ] {
        let hits = match_pattern(lang, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "{lang:?} {pat} on {src:?}: clean face must keep answering: {:?}",
            klass_lines(&hits)
        );
    }
    // F5 pins: sg ANSWERS every meta mid-link spelling on identifier leaves
    // (first-hand probe grid 2026-09-08 — previously ZERO oracle evidence;
    // the admission is sg-exact as registered).
    for pat in ["a?.$B?.c($X)", "a?.$$A?.c($X)", "a?.$$$A?.c($X)"] {
        for (lang, lang_name) in [(Language::JavaScript, "js"), (Language::TypeScript, "ts")] {
            let hits = match_pattern(lang, "a?.b?.c(1);\n", pat).unwrap();
            assert_eq!(
                klass_lines(&hits),
                vec![1u32],
                "{lang_name} {pat} on a?.b?.c(1): sg answers the meta mid-link \
                 (F5 probe pin): {:?}",
                klass_lines(&hits)
            );
        }
    }
}

/// RED (117E-1, agent 118): the PASS 117 `member_link_has_trivia` veto
/// over-refuses CALLEE-position comment trivia on kotlin chains. kotlin-ng
/// flattens `navigation_suffix`, so a post-`?.` comment is a DIRECT child of
/// the member link (first-hand CST dump 2026-09-10: `a?. /*c*/ b` =
/// [identifier, `?.` anon, comment EXTRA, identifier]) and the veto fired
/// — but sg 0.45.2 ANSWERS every callee-side face (first-hand grid
/// /tmp/phase118/g118a*.sh: `a?. /*c*/ b?.c(1)`, `a?.b?. /*c*/ c(1)`, line
/// and block comments, dotted `.` connectors, 2/3/4-link chains, double
/// comments — oracle HIT each) while REFUSING trivia BEFORE the connector
/// (receiver-side `a /*c*/ ?.b?.c(1)` and between-links `a?.b /*c*/ ?.c(1)`
/// — oracle []). The veto must be position-scoped: link-STRUCTURAL trivia
/// (a direct trivia child with a LATER ANONYMOUS sibling — the connector
/// token) refuses; callee-internal trivia (named siblings only after it)
/// is transparent. Swift never enters the class (its grammar wraps the
/// comment inside `navigation_suffix`, never a link child — CST dump).
#[test]
fn f118a_callee_position_link_trivia_is_transparent() {
    // sg ANSWERS (oracle HIT): the 117-caused over-refusals must answer.
    for (src, pat, desc) in [
        (
            "a?. /*c*/ b?.c(1);\n",
            "a?.b?.c($X)",
            "block comment after first ?.",
        ),
        (
            "a?.b?. /*c*/ c(1);\n",
            "a?.b?.c($X)",
            "block comment after second ?.",
        ),
        (
            "a?. // c\nb?.c(1);\n",
            "a?.b?.c($X)",
            "line comment after first ?.",
        ),
        // NOTE (agent 118 disclosure): the `$`-LESS 2-link property twin
        // (`a?.b` on `a?. /*c*/ b`) also answers [] where sg answers, but it
        // routes through the LITERAL lane (no `$` → match_literal_pattern),
        // which no 117 hunk touched — a PRE-EXISTING, uncharted silent-under
        // class outside this round's charter (registered CNR §39.13 with a
        // form-1 retry predicate; NOT the 117-caused veto regression).
        (
            "a?. /*c*/ b?.c?.d(1);\n",
            "a?.b?.c?.d($X)",
            "4-link callee-side",
        ),
        (
            "a?.b?. /*c*/ c?.d(1);\n",
            "a?.b?.c?.d($X)",
            "4-link second-link callee-side",
        ),
        (
            "a. /*c*/ b?.c(1);\n",
            "a.b?.c($X)",
            "dotted connector callee-side",
        ),
        (
            "a?. /*c1*/ /*c2*/ b?.c(1);\n",
            "a?.b?.c($X)",
            "double comment after ?.",
        ),
    ] {
        let hits = match_pattern(Language::Kotlin, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "kotlin {pat} on {src:?} ({desc}): sg ANSWERS the callee-position \
             trivia face — the 117 member-link veto must not fire on trivia \
             that only has named siblings after it (117E-1): {:?}",
            klass_lines(&hits)
        );
    }
    // Swift callee-side keeps answering (never vetoed — its comment attaches
    // inside navigation_suffix, outside the link children).
    for (src, pat) in [
        ("a?. /*c*/ b?.c(1);\n", "a?.b?.c($X)"),
        ("a?.b?. /*c*/ c(1);\n", "a?.b?.c($X)"),
    ] {
        let hits = match_pattern(Language::Swift, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "swift {pat} on {src:?}: callee-side trivia stays answered: {:?}",
            klass_lines(&hits)
        );
    }
    // sg REFUSES (oracle []): link-STRUCTURAL positions must keep refusing —
    // the position scope may not un-refuse the registered 117 F1 cells.
    for (lang, lang_name, src) in [
        (Language::Kotlin, "kotlin", "a /*c*/ ?.b?.c(1);\n"),
        (Language::Kotlin, "kotlin", "a?.b /*c*/ ?.c(1);\n"),
        (Language::Kotlin, "kotlin", "a?.b(1) /*c*/ ?.c(2);\n"),
        (Language::Kotlin, "kotlin", "a. /*c*/ ?.b?.c(1);\n"),
        (Language::Swift, "swift", "a /*c*/ ?.b?.c(1);\n"),
        (Language::Swift, "swift", "a?.b /*c*/ ?.c(1);\n"),
    ] {
        let hits = match_pattern(lang, src, "a?.b?.c($X)").unwrap();
        assert!(
            hits.is_empty(),
            "{lang_name} on {src:?}: trivia BEFORE the connector is \
             link-structural — sg refuses, the veto must keep firing: {:?}",
            klass_lines(&hits)
        );
    }
    // Clean controls keep answering through the gated arm.
    for (lang, lang_name) in [(Language::Kotlin, "kotlin"), (Language::Swift, "swift")] {
        let hits = match_pattern(lang, "a?.b?.c(1);\n", "a?.b?.c($X)").unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "{lang_name} clean chain keeps answering: {:?}",
            klass_lines(&hits)
        );
    }
}

/// RED (117E-Info, agent 118 — kt half FIXED, swift half REGISTERED): clean
/// ALL-CALL `?.`-chain faces on kotlin/swift went rc2-LOUD where sg 0.45.2
/// answers (first-hand grid /tmp/phase118/g118b.sh: `a?.b($X)?.c($Y)` on
/// `a?.b(1)?.c(2)` kt+swift, 4-link twins, `$$$`-rest all-call chains, swift
/// 2-link — LOUD2 vs oracle HIT; kt 2-link already answers through the
/// OptionalCall lane). The optional-chain walker (PASS 115 hunk B + 117
/// junction contracts) already pairs pattern-CALL segments with candidate
/// CALL nodes — kotlin-ng nests mid-chain calls as real `call_expression`
/// nodes inside `navigation_expression` (first-hand CST dump 2026-09-10) —
/// so the kt loud is pure answerability admission. SWIFT stays registered:
/// its `?`-connector + `navigation_suffix` link shape is outside
/// `member_link_parts`' decomposition; admitting it would walk silent-empty
/// where sg answers (fail-open), so the swift census loud is the registered
/// class (form-1 predicate in CNR §39.13).
#[test]
fn f118b_kotlin_all_call_optional_chains_answer_sg_exact() {
    // Ingress: kotlin must stop refusing the all-call chain spellings.
    for pat in [
        "a?.b($X)?.c($Y)",
        "a?.b($X)?.c($Y)?.d($Z)",
        "a?.b($X)?.c($$$A)",
    ] {
        assert!(
            native_pattern_answerable(Language::Kotlin, pat),
            "kotlin {pat}: all-call `?.` chains must be answerable — the \
             walker pairs pattern-CALL↔candidate-CALL (117E-Info kt fix)"
        );
    }
    // Swift keeps its REGISTERED census-loud class (never admitted without
    // the missing link-shape machinery — silent-empty is forbidden).
    for pat in ["a?.b($X)?.c($Y)", "a?.b($X)", "a?.b($X)?.c($$$A)"] {
        assert!(
            !native_pattern_answerable(Language::Swift, pat),
            "swift {pat}: the census loud is the REGISTERED 117E-Info swift \
             class — do not admit without the navigation_suffix decomposer"
        );
    }
    // Answers: clean kt all-call chains sg-exact.
    for (src, pat, desc) in [
        (
            "a?.b(1)?.c(2);\n",
            "a?.b($X)?.c($Y)",
            "3-link, binds X=1 Y=2",
        ),
        ("a?.b(1)?.c(2)?.d(3);\n", "a?.b($X)?.c($Y)?.d($Z)", "4-link"),
        (
            "a?.b(1)?.c(2, 3);\n",
            "a?.b($X)?.c($$$A)",
            "trailing rest in mid-chain call",
        ),
    ] {
        let hits = match_pattern(Language::Kotlin, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "kotlin {pat} on {src:?} ({desc}): sg answers the clean all-call \
             chain: {:?}",
            klass_lines(&hits)
        );
    }
    // Negative control: connector/name mismatch stays empty (sg rc1-empty).
    let hits = match_pattern(Language::Kotlin, "a?.b(1)?.d(2);\n", "a?.b($X)?.c($Y)").unwrap();
    assert!(
        hits.is_empty(),
        "kotlin negative control must stay empty: {:?}",
        klass_lines(&hits)
    );
    // Connector-flag exactness: an all-plain candidate never answers an
    // all-`?.` pattern (the ONE token-exact rule).
    let hits = match_pattern(Language::Kotlin, "a.b(1)?.c(2);\n", "a?.b($X)?.c($Y)").unwrap();
    assert!(
        hits.is_empty(),
        "kotlin connector mismatch (plain head) must stay empty: {:?}",
        klass_lines(&hits)
    );
    // Unmoved controls: kt 2-link already answers; property-face mixed chains
    // keep riding the gated general arm.
    for (src, pat, desc) in [
        ("a?.b(1);\n", "a?.b($X)", "kt 2-link (OptionalCall lane)"),
        (
            "a?.b?.c(1);\n",
            "a?.b?.c($X)",
            "property face (general arm)",
        ),
    ] {
        let hits = match_pattern(Language::Kotlin, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "kotlin {pat} on {src:?} ({desc}) must keep answering: {:?}",
            klass_lines(&hits)
        );
    }
}

/// RED (agent 120, 119B-F1 cross-check — REGRESSION from the 118-H2 kotlin
/// all-call admission): the admitted kt all-call lane decomposes candidate
/// member links through `member_link_parts`, which skips trivia children
/// unconditionally — the lane consults NONE of the 118 position rule. The
/// trivia×all-call intersection over-answers where sg 0.45.2 REFUSES
/// (first-hand probe grid /tmp/phase120/s0 + /tmp/phase120/g2: subject HIT /
/// oracle [] rc1 on every link-STRUCTURAL cell, 2-link and 3-link, plain and
/// meta heads). sg's rule (CNR §39.13, 118 grids): a direct link-child
/// trivia with a LATER ANONYMOUS sibling is link-STRUCTURAL — refused; a
/// trivia child followed only by NAMED siblings is callee-INTERNAL —
/// transparent, answered. The kt all-call lane must apply the SAME
/// position-scoped veto the kt property-face general arm already applies
/// (`member_link_has_trivia`) — the two kt lanes must hold ONE doctrine.
/// Also pins (119B-F3): the head-args mixed-rest slot contract cells
/// (sg-agree first-hand: trailing-rest head binds n ≥ 2), which must not
/// move with the veto.
#[test]
fn f120a_kotlin_all_call_links_refuse_link_structural_trivia() {
    // sg REFUSES (oracle [] rc1, probe grid 2026-09-08): link-STRUCTURAL.
    for (src, pat, desc) in [
        (
            "a?.b(1) /*c*/ ?.c(2);\n",
            "a?.b($X)?.c($Y)",
            "call-link tail trivia, 3-link",
        ),
        (
            "a /*c*/ ?.b(1)?.c(2);\n",
            "a?.b($X)?.c($Y)",
            "receiver-side trivia, 3-link",
        ),
        (
            "a?.b(1) /*c*/ ?.c(2);\n",
            "$A?.b($X)?.c($Y)",
            "meta head, call-link tail",
        ),
        (
            "a /*c*/ ?.b(1)?.c(2);\n",
            "$A?.b($X)?.c($Y)",
            "meta head, receiver-side",
        ),
        (
            "a /*c*/ ?.b(1);\n",
            "a?.b($X)",
            "2-link receiver-side (OptionalCall lane)",
        ),
        (
            "a /*c*/ ?.b(1);\n",
            "$A?.b($X)",
            "2-link receiver-side, meta head",
        ),
    ] {
        let hits = match_pattern(Language::Kotlin, src, pat).unwrap();
        assert!(
            hits.is_empty(),
            "kotlin {pat} on {src:?} ({desc}): sg REFUSES the link-structural \
             trivia face — the all-call lanes must apply the position-scoped \
             trivia veto (119B-F1 regression): {:?}",
            klass_lines(&hits)
        );
    }
    // sg ANSWERS (oracle HIT): transparent positions keep answering — the
    // position scope may not over-refuse (118 callee-internal doctrine).
    for (src, pat, desc) in [
        ("a?.b(1)?.c(2);\n", "a?.b($X)?.c($Y)", "clean 3-link"),
        (
            "a?. /*c*/ b(1)?.c(2);\n",
            "a?.b($X)?.c($Y)",
            "callee-internal 3-link",
        ),
        (
            " /*c*/ a?.b(1)?.c(2);\n",
            "a?.b($X)?.c($Y)",
            "pre-chain trivia 3-link",
        ),
        ("a?.b(1);\n", "a?.b($X)", "clean 2-link"),
        ("a?. /*c*/ b(1);\n", "a?.b($X)", "callee-internal 2-link"),
        (" /*c*/ a?.b(1);\n", "a?.b($X)", "pre-chain trivia 2-link"),
    ] {
        let hits = match_pattern(Language::Kotlin, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "kotlin {pat} on {src:?} ({desc}): sg ANSWERS the transparent \
             trivia face — the veto must stay position-scoped: {:?}",
            klass_lines(&hits)
        );
    }
    // 119B-F3 pins: head-args mixed-rest slot contract (sg-agree cells,
    // first-hand probe grid /tmp/phase120/g2/kthead+ktslot).
    for (src, pat, want, desc) in [
        (
            "a(0, 1, 2)?.c(3);\n",
            "a($A, $$$B)?.c($X)",
            vec![1u32],
            "head trailing rest, n=3",
        ),
        (
            "a(1, 2, 3)?.c(4);\n",
            "a($A, $$$B)?.c($X)",
            vec![1u32],
            "head trailing rest, n=3 (slot twin)",
        ),
        (
            "a(0, 1, 2)?.c(3);\n",
            "a($A, $B)?.c($X)",
            vec![],
            "head plain two-meta vs n=3: arity veto",
        ),
        (
            "a(1)?.c(2);\n",
            "a($A, $$$B)?.c($X)",
            vec![],
            "head trailing rest needs n >= 2",
        ),
    ] {
        let hits = match_pattern(Language::Kotlin, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            want,
            "kotlin {pat} on {src:?} ({desc}): head-args slot contract must \
             stay sg-exact: {:?}",
            klass_lines(&hits)
        );
    }
}

/// RED (agent 120, 119A-F2 family — census-loud expression ROOT kinds where
/// sg 0.45.2 ANSWERS, first-hand grid /tmp/phase120/g2): `new q($X)` js/ts,
/// parenthesized arrow roots `($X) => $Y` js, and python `lambda $X: $Y` all
/// rc2-LOUD on match-bearing files where sg answers the root node hit. The
/// general-lane machinery is root-kind-generic (walk_general + general_eq)
/// — the loud is pure admission: the eligibility bare-keyword guard refuses
/// the `new`/`lambda` heads and `is_general_root_kind` refuses the
/// `arrow_function` root. sg truth pinned here: every admitted face answers
/// on its match file; bare-param arrows ANSWER (premise corrected per CNR
/// §40.2 — the earlier "accepted-empty" reading was fixture-specific
/// semantic empty — so `x => q($X)` / `$X => q($Y)` are admitted above);
/// the non-admitted root kinds (tagged template — B-97A-3's interpolation
/// gap; brace-less for-head — no For lane; kt lambda_literal — unprobed
/// sibling faces) KEEP the registered census-loud class. (Doc corrected by
/// agent 122, 121B-F2 — the old wording still asserted the refuted
/// accepted-empty premise.)
#[test]
fn f120b_expression_root_kinds_admitted_per_sg_grid() {
    // sg ANSWERS (oracle HIT, grid 2026-09-08): these faces must answer.
    for (lang, src, pat, desc) in [
        (
            Language::JavaScript,
            "const x = new q(1);\n",
            "new q($X)",
            "new_expression root js",
        ),
        (
            Language::TypeScript,
            "const x = new q(1);\n",
            "new q($X)",
            "new_expression root ts",
        ),
        (
            Language::JavaScript,
            "const a = new q(1, 2);\n",
            "new q($A, $B)",
            "new two-meta args",
        ),
        (
            Language::JavaScript,
            "const m = new ns.Q(3);\n",
            "new ns.Q($X)",
            "new dotted callee",
        ),
        (
            Language::JavaScript,
            "const f = (a) => a + 1;\n",
            "($X) => $Y",
            "paren-param arrow root",
        ),
        (
            Language::JavaScript,
            "const h = (a, b) => q(a, b);\n",
            "($A, $B) => q($A, $B)",
            "two-param arrow",
        ),
        (
            Language::JavaScript,
            "setTimeout(() => q(1));\n",
            "() => q($X)",
            "empty-param arrow",
        ),
        (
            Language::JavaScript,
            "const f = x => q(1);\n",
            "x => q($X)",
            "bare-param literal-head arrow",
        ),
        (
            Language::JavaScript,
            "const f = x => q(1);\n",
            "$X => q($Y)",
            "bare-param meta-head arrow",
        ),
        (
            Language::Python,
            "f = lambda x: x + 1\n",
            "lambda $X: $Y",
            "py lambda root",
        ),
        (
            Language::Python,
            "g = lambda: 1\n",
            "lambda: $Y",
            "py lambda no params",
        ),
        (
            Language::Python,
            "h = lambda x: x\n",
            "lambda $X: $X",
            "py lambda same-name bind",
        ),
        (
            Language::Python,
            "f = lambda x: q(x)\n",
            "lambda $X: q($X)",
            "py lambda call body",
        ),
    ] {
        let hits = match_pattern(lang, src, pat).unwrap();
        assert_eq!(
            klass_lines(&hits),
            vec![1u32],
            "{pat:?} on {src:?} ({desc}): sg ANSWERS this expression-root \
             face — the census must not refuse it (119A-F2): {:?}",
            klass_lines(&hits)
        );
    }
    // Negative: arity mismatch stays empty (sg rc1 [] on the twin file).
    let hits = match_pattern(
        Language::JavaScript,
        "const x = new q(1);\n",
        "new q($A, $B)",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "js new arity mismatch must stay empty: {:?}",
        klass_lines(&hits)
    );
    // Unmoved: rust `Q::new($X)` already answers through the Call lane.
    let hits = match_pattern(Language::Rust, "let a = Q::new(1);\n", "Q::new($X)").unwrap();
    assert_eq!(
        klass_lines(&hits),
        vec![1u32],
        "rust Q::new stays answered (Call lane): {:?}",
        klass_lines(&hits)
    );
    // Census classes KEEP the loud refusal (registered rows — admission
    // without sg-mapped machinery is forbidden): the tagged template root is
    // B-97A-3 (no interpolation binding machinery); the brace-less for-head
    // has no For lane (F5 register row); the kt lambda_literal is the
    // F2-family residual (only the single implicit-param-free face probed).
    // NOTE (agent 120 correction): `x => q($X)` was REMOVED from this list —
    // the original sg `[]` on the arrow grid was fixture-specific semantic
    // empty (no `x =>` candidate in that fixture), NOT an accepted-empty
    // class; f89b's registered `$A => $B;` face plus fresh sg probes
    // (`x => q($X)` / `$X => q($Y)` answer their candidates) pinned the
    // bare-param arrow as sg-ANSWERING, so it is admitted above instead.
    for (lang, pat, why) in [
        (
            Language::JavaScript,
            "tag`abc${$X}`",
            "tagged template root stays B-97A-3 registered loud",
        ),
        (
            Language::JavaScript,
            "for ($$$A) $B",
            "brace-less for-head stays the F5 registered loud (no For lane)",
        ),
        (
            Language::Kotlin,
            "{ $A -> $B }",
            "kt lambda_literal stays the F2-family registered loud",
        ),
    ] {
        assert!(
            !native_pattern_answerable(lang, pat),
            "{lang} {pat:?}: {why} — do not admit without the sg-mapped \
             machinery"
        );
    }
    // The admitted faces are answerable for their languages (ingress pin).
    for (lang, pat) in [
        (Language::JavaScript, "new q($X)"),
        (Language::TypeScript, "new q($X)"),
        (Language::JavaScript, "($X) => $Y"),
        (Language::Python, "lambda $X: $Y"),
    ] {
        assert!(
            native_pattern_answerable(lang, pat),
            "{lang} {pat:?}: sg-answerable root kind must be answerable"
        );
    }
}

// ---------------------------------------------------------------------------
// PASS 122 (r64 remediation): F1 if-template brace-ness, F2 swift member-link
// trivia positions, F3 literal-root under-answers, F4 root-kind admissions.
// Every expected class below is a first-hand oracle probe of ast-grep 0.45.2
// (sha16 9585263377c1fc98) recorded on the 2026-09-10 grids under
// /tmp/phase122/{f1,f2,f3,f4}; failure-first RED runs and the M-122a..d
// mutation cycle are attested in the phase122 report. Grid-derived sg rules:
// (F1) a braced pattern body requires a braced candidate consequence; a bare
// `:` suite section is sg's EMPTY suite; only NAMED if-nodes answer.
// (F2) swift links refuse candidates whose comment sits before a later
// navigation_suffix; callee-internal trivia stays transparent.
// (F3) string/keyword-literal roots answer their leaf nodes at every
// position; numbers already agreed.
// (F4) async-arrow / generator / let-const-var (js/ts) / collection-literal
// roots answer; class-method, import/export/try, java/rb/rust-macro roots
// KEEP the registered census-loud class.
// ---------------------------------------------------------------------------

/// F1 (f122a): sg's brace-ness rule — a `{ $B }`-spelled pattern never
/// answers a brace-less candidate (`if (c) d();` is sg `[]` rc1 silent on
/// js/ts/c/php, oracle grid f1), while the braced file answers and a mixed
/// file answers ONLY the braced row.
#[test]
fn f122a_if_pattern_brace_ness_is_structural() {
    // Braced file answers; brace-less file refuses; mixed file answers only
    // the braced row.
    let braced =
        match_pattern(Language::JavaScript, "if (a) { b(); }\n", "if ($X) { $B }").unwrap();
    assert_eq!(lines_of(&braced), vec![1]);
    let braceless = match_pattern(Language::JavaScript, "if (c) d();\n", "if ($X) { $B }").unwrap();
    assert!(
        braceless.is_empty(),
        "braced pattern must refuse brace-less if"
    );
    let mixed = match_pattern(
        Language::JavaScript,
        "if (a) { b(); }\nif (c) d();\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&mixed), vec![1]);
    // The rest meta body carries the same brace-ness rule.
    let rest = match_pattern(Language::JavaScript, "if (c) d();\n", "if ($X) { $$$B }").unwrap();
    assert!(rest.is_empty());
    // Brace-less else rows agree with the consequence rule (sg [] f1).
    let else_braceless = match_pattern(
        Language::JavaScript,
        "if (a) b(); else c();\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert!(else_braceless.is_empty());
    // go braces are mandatory: the braced pattern answers (grid f1 AGREE n1).
    let go = match_pattern(Language::Go, "if x {\n    y()\n}\n", "if $X { $B }").unwrap();
    assert_eq!(lines_of(&go), vec![1]);
    // PASS 122 live-grid cross-grammar alignment (probe_go1/go2, probe_py1/
    // py2): go binds the pattern's condition parens structurally — `($X)`
    // needs a parenthesized_expression candidate (plain condition refuses);
    // the paren-free pattern answers both shapes. python can never align a
    // `{...}` if body (suites are `:`-indented; even the literal `{q()}`
    // suite face is sg semantic-empty) — the braced pattern refuses every
    // py candidate.
    let go_paren = match_pattern(
        Language::Go,
        "func f() {\n    if (x) {\n        y()\n    }\n}\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&go_paren), vec![2]);
    let go_plain_cond =
        match_pattern(Language::Go, "if x {\n    y()\n}\n", "if ($X) { $B }").unwrap();
    assert!(go_plain_cond.is_empty());
    let go_parenfree_pattern = match_pattern(
        Language::Go,
        "func f() {\n    if (x) {\n        y()\n    }\n}\n",
        "if $X { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&go_parenfree_pattern), vec![2]);
    let py_braced_suite =
        match_pattern(Language::Python, "if x:\n    {q()}\n", "if ($X) { $B }").unwrap();
    assert!(py_braced_suite.is_empty());
    let py_braced_suite2 = match_pattern(
        Language::Python,
        "if x:\n    q()\nif y:\n    r()\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert!(py_braced_suite2.is_empty());
    // kt braced if answers, kt brace-less if refuses (grid f1; kt's
    // control_structure_body wrapper counts as braced via its block child).
    let kt_braced = match_pattern(
        Language::Kotlin,
        "fun main() {\n    if (a) { b() }\n}\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&kt_braced), vec![2]);
    let kt_loose = match_pattern(
        Language::Kotlin,
        "fun main() {\n    if (a) b()\n}\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert!(kt_loose.is_empty());
    // Registered H-CONF-IFBODY louds stay byte-stable (§30.6).
    // PASS 127 correction (§30.6 re-adjudication, oracle grid
    // /tmp/phase127/g1 f2_js_while_meta + cells2 o1): the `while ($X) { $B }`
    // row LEFT this list — its census-loud premise is oracle-false (sg
    // 0.45.2 ANSWERS the braced and brace-less while spellings n1 with
    // X/B bindings) and the face now answers sg-exactly through the
    // PASS-127 loop-root admission (f127b). The if-family rows and the
    // concrete-body if row keep their registered loud classes (sg
    // refuses those spellings).
    for pat in ["if ($X) $B", "if ($X) q($A)", "if ($X) { q($A) }"] {
        assert!(
            needs_ast_grep_fallback(pat),
            "{pat} must keep the registered census-loud class"
        );
    }
    let while_braced = match_pattern(
        Language::JavaScript,
        "while (a) { b(); }\n",
        "while ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&while_braced), vec![1]);
}

/// F1 (f122a): candidate-position comment discipline at the if node — a
/// direct trivia child BEFORE the consequence breaks sg's structural match
/// (`if (a) /*c*/ { b(); }`, `if /*c*/ (a) { b(); }` are sg `[]`), while
/// comments after the consequence (pre-`else`), inside the condition, inside
/// the body, and outside the if are transparent (sg n1 each, grid f1).
#[test]
fn f122a_if_lane_direct_trivia_before_consequence_refuses() {
    let refused1 = match_pattern(
        Language::JavaScript,
        "if (a) /*c*/ { b(); }\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert!(refused1.is_empty(), "comment before the block must refuse");
    let refused2 = match_pattern(
        Language::JavaScript,
        "if /*c*/ (a) { b(); }\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert!(refused2.is_empty());
    let transparent_body = match_pattern(
        Language::JavaScript,
        "if (a) { /*c*/ b(); }\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&transparent_body), vec![1]);
    let transparent_cond = match_pattern(
        Language::JavaScript,
        "if (a /*c*/) { b(); }\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&transparent_cond), vec![1]);
    let transparent_preelse = match_pattern(
        Language::JavaScript,
        "if (a) { b(); } /*c*/ else { d(); }\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&transparent_preelse), vec![1]);
}

/// F1 fold (f122e): the anonymous `if` KEYWORD TOKEN is not an if-site —
/// body-less templates answered the token row on go/py before the named-node
/// guard (oracle grid f1: `if $X` go subj n2 vs sg n1; py n4 vs sg n2).
#[test]
fn f122a_bodyless_if_template_answers_named_if_nodes_only() {
    let go = match_pattern(Language::Go, "if x {\n    y()\n}\n", "if $X").unwrap();
    assert_eq!(lines_of(&go), vec![1]);
    let py = match_pattern(
        Language::Python,
        "if x:\n    q()\nif y:\n    r()\n",
        "if $X",
    )
    .unwrap();
    assert_eq!(lines_of(&py), vec![1, 3]);
}

/// F2 (f122b): the swift member-link trivia position rule. sg 0.45.2 refuses
/// swift dotted-call candidates whose comment sits before a LATER
/// navigation_suffix — receiver-side, mid-link, and doubled — while the
/// callee-internal position (inside the suffix, after the `.`) stays
/// transparent and the js transparency contract is untouched (CST dumps +
/// oracle grid /tmp/phase122/f2).
#[test]
fn f122b_swift_link_structural_trivia_refuses_and_transparent_keeps() {
    // Fail-open cells closed (were subject HITS where sg refuses).
    for src in [
        "let r = a /*c*/ .b(1)\n",
        "let r = a /*c*/ /*d*/ .b(1)\n",
        "let r = a.b /*c*/ .c(2)\n",
    ] {
        let hits = match_pattern(Language::Swift, src, "a.b($X)").unwrap();
        assert!(hits.is_empty(), "swift {src:?} must refuse a.b($X)");
        let meta = match_pattern(Language::Swift, src, "$A.b($X)").unwrap();
        assert!(meta.is_empty(), "swift {src:?} must refuse $A.b($X)");
    }
    // PASS 122 live-grid CORRECTION (oracle /tmp/phase122/f2 sw_mid3 +
    // sw_mid3meta): sg 0.45.2 ANSWERS the trivia-free INNER link of a
    // 3-link chain whose LATER suffix path carries the trivia
    // (`a.b(1) /*c*/ .c(2)` × `a.b($X)` sg n1). The structural veto is
    // PER-CANDIDATE — the candidate's own callee (`a.b`) is trivia-free —
    // not ancestor-chain; the ancestor-chain form over-refused (RED: the
    // asserts below failed against it) and was removed. The OUTER `.c(2)`
    // call still refuses: its own suffix path carries the trivia.
    let inner = match_pattern(Language::Swift, "let r = a.b(1) /*c*/ .c(2)\n", "a.b($X)").unwrap();
    assert_eq!(lines_of(&inner), vec![1]);
    let inner_meta =
        match_pattern(Language::Swift, "let r = a.b(x) /*c*/ .c(y)\n", "$A.b($X)").unwrap();
    assert_eq!(lines_of(&inner_meta), vec![1]);
    let chain = match_pattern(
        Language::Swift,
        "let r = a.b(1) /*c*/ .c(2)\n",
        "a.b($X).c($Y)",
    )
    .unwrap();
    assert!(chain.is_empty(), "mid-link trivia must refuse the 3-link");
    // Transparent cells keep answering (grid f2 AGREE / sg n1).
    let clean = match_pattern(Language::Swift, "let r = a.b(1)\n", "a.b($X)").unwrap();
    assert_eq!(lines_of(&clean), vec![1]);
    let callee_internal =
        match_pattern(Language::Swift, "let r = a. /*c*/ b(1)\n", "a.b($X)").unwrap();
    assert_eq!(lines_of(&callee_internal), vec![1]);
    // js keeps the PASS 111 comment-transparency contract byte-stable.
    let js_transparent =
        match_pattern(Language::JavaScript, "let r = a /*c*/ .b(1);\n", "a.b($X)").unwrap();
    assert_eq!(lines_of(&js_transparent), vec![1]);
}

/// F3 (f122c): string and keyword-literal roots answer their leaf nodes at
/// every position — sg n1 on every grid face (init/arg/return/array, js/ts/
/// py) where the subject previously answered rc0 [] (oracle grid
/// /tmp/phase122/f3). Number roots already agreed and stay pinned.
#[test]
fn f122c_literal_roots_answer_leaf_nodes() {
    let string_init = match_pattern(Language::JavaScript, "const s = 'q';\n", "'q'").unwrap();
    assert_eq!(lines_of(&string_init), vec![1]);
    let string_arg = match_pattern(Language::JavaScript, "f('q');\n", "'q'").unwrap();
    assert_eq!(lines_of(&string_arg), vec![1]);
    let string_arr = match_pattern(Language::JavaScript, "const a = ['q'];\n", "'q'").unwrap();
    assert_eq!(lines_of(&string_arr), vec![1]);
    for (src, pat) in [
        ("x = null;\n", "null"),
        ("x = true;\n", "true"),
        ("x = false;\n", "false"),
        ("g(null);\n", "null"),
        ("g(true);\n", "true"),
        ("function f() { return null; }\n", "null"),
        ("const a = [null];\n", "null"),
        ("x = None\n", "None"),
        ("x = True\n", "True"),
    ] {
        let lang = if pat == "None" || pat == "True" {
            Language::Python
        } else {
            Language::JavaScript
        };
        let hits = match_pattern(lang, src, pat).unwrap();
        assert_eq!(lines_of(&hits), vec![1], "{lang} {src:?} × {pat:?}");
    }
    // Number roots keep their agreeing answer (no regression).
    let num = match_pattern(Language::JavaScript, "const n = 1;\n", "1").unwrap();
    assert_eq!(lines_of(&num), vec![1]);
    // The keyword-literal class the core ingress must route to the walk.
    // PASS 124 (f124g): `False`/`super` added — a mutant deleting either arm
    // previously survived the whole suite (123B-F3 test hole).
    for kw in [
        "null", "true", "false", "None", "True", "this", "False", "super",
    ] {
        assert!(
            ast_sgrep_lang::pattern_is_keyword_literal_root(kw),
            "{kw} must be keyword-literal"
        );
    }
    for ident in ["undefined", "self", "greet"] {
        assert!(
            !ast_sgrep_lang::pattern_is_keyword_literal_root(ident),
            "{ident} is a plain identifier root"
        );
    }
}

/// F4 (f122d): the sg-answering root kinds admitted per the f4 grids —
/// async-led arrows, the js generator root, js/ts declarator roots, and the
/// collection-literal roots — plus the registered census-loud faces that must
/// NOT move (class-with-method, import/export/try, java void method, rb
/// def/blocks, rust macro).
#[test]
fn f122d_root_kind_admissions_per_sg_grid() {
    // async arrows (sg n1 each, grid f4).
    assert!(!needs_ast_grep_fallback("async (x) => q($X)"));
    let async_arrow = match_pattern(
        Language::JavaScript,
        "const f = async (x) => q(x);\n",
        "async (x) => q($X)",
    )
    .unwrap();
    assert_eq!(lines_of(&async_arrow), vec![1]);
    let async_meta = match_pattern(
        Language::JavaScript,
        "const f = async (x) => q(x);\n",
        "async ($A) => q($X)",
    )
    .unwrap();
    assert_eq!(lines_of(&async_meta), vec![1]);
    let async_bare = match_pattern(
        Language::JavaScript,
        "const f = async x => q(x);\n",
        "async x => q($X)",
    )
    .unwrap();
    assert_eq!(lines_of(&async_bare), vec![1]);
    // generator: the single-meta body spelling answers (sg n1, matching-name
    // grid f4); the `$$$`-body spelling keeps the registered H-CONF-002
    // statement-template rest refusal (F72a-2 envelope — the general lane's
    // `$$$` guard precedes root-kind admission), so it stays census-loud.
    assert!(!needs_ast_grep_fallback("function* q($X) { $B }"));
    let gen = match_pattern(
        Language::JavaScript,
        "function* q(a) {\n    yield a;\n}\n",
        "function* q($X) { $B }",
    )
    .unwrap();
    // sg n1: exactly the whole generator_declaration row (grid f4).
    assert_eq!(lines_of(&gen), vec![1]);
    assert!(needs_ast_grep_fallback("function* q($X) { $$$B }"));
    // let/const/var declarator roots (sg n1, grid f4). The rust per-file
    // census keeps its loud class (rust builds root at let_statement).
    assert!(!needs_ast_grep_fallback("let $A = $B"));
    assert!(!needs_ast_grep_fallback("const $A = $B"));
    assert!(!needs_ast_grep_fallback("var $A = $B"));
    let let_hits = match_pattern(Language::JavaScript, "let a = b();\n", "let $A = $B").unwrap();
    assert_eq!(lines_of(&let_hits), vec![1]);
    assert_eq!(
        let_hits[0].captures.get("B").map(String::as_str),
        Some("b()")
    );
    let const_hits =
        match_pattern(Language::JavaScript, "const a = b();\n", "const $A = $B").unwrap();
    assert_eq!(lines_of(&const_hits), vec![1]);
    let var_hits = match_pattern(Language::JavaScript, "var a = b();\n", "var $A = $B").unwrap();
    assert_eq!(lines_of(&var_hits), vec![1]);
    assert!(!native_pattern_answerable(Language::Rust, "let $A = $B"));
    // collection-literal roots (sg n1 js/py/swift, grid f4).
    assert!(!needs_ast_grep_fallback("[$A, $B]"));
    assert!(!needs_ast_grep_fallback("{ a: $A }"));
    let arr = match_pattern(Language::JavaScript, "const a = [x, y];\n", "[$A, $B]").unwrap();
    assert_eq!(lines_of(&arr), vec![1]);
    let py_arr = match_pattern(Language::Python, "a = [x, y]\n", "[$A, $B]").unwrap();
    assert_eq!(lines_of(&py_arr), vec![1]);
    let obj = match_pattern(Language::JavaScript, "const o = { a: x };\n", "{ a: $A }").unwrap();
    assert_eq!(lines_of(&obj), vec![1]);
    // Registered census-loud faces keep their class (F4 residual bundle).
    // CORRECTED PASS 137: `export const $A = $B` is REFUTED as loud — the
    // oracle probe (2026-09-11, f137i registration) answers n1 binding
    // A/B (`export const a = 1;` → row 1); the F4 grid cell had been misread
    // under the accepted-empty convention. It joined the general lane's
    // export_statement root admission (f137i pins the binding).
    for (lang, pat) in [
        (Language::JavaScript, "class q { m($A) { $$$B } }"),
        (Language::JavaScript, "import $A from 'q'"),
        (Language::JavaScript, "try { $A } catch { $B }"),
        (Language::JavaScript, "catch ($E) { $B }"),
        (Language::Java, "void q($X) { $$$B }"),
        (Language::Ruby, "q { $B }"),
        (Language::Ruby, "q do $B end"),
    ] {
        assert!(
            needs_ast_grep_fallback(pat),
            "{lang} {pat:?} must keep the registered census-loud class"
        );
    }
    // rust `q!($A)` (F4 residual): the registered loud class is the RUST
    // per-language census, not the language-free ingress gate (the
    // any-language general-lane loop templated the TS non-null spelling
    // first, so the ingress admits the bytes). rust's build roots at
    // `macro_invocation`, which `is_general_root_kind` refuses → the rust
    // census stays loud where sg 0.45.2 runs-empty (CNR §16.1 pass-63
    // form-1 predicate: metavar-in-token_tree never matches in sg; loud
    // refusal is the registered terminal state until macro-face demand +
    // token-tree semantics exist).
    assert!(
        !native_pattern_answerable(Language::Rust, "q!($A)"),
        "rust macro face must keep the registered census-loud class"
    );
    let rust_macro = match_pattern(Language::Rust, "fn f() {\n    q!(x);\n}\n", "q!($A)").unwrap();
    assert!(
        rust_macro.is_empty(),
        "rust macro walk must refuse: {rust_macro:?}"
    );
}

/// F1 fold (f122e): sg's bare `:` suite section is the EMPTY suite, not an
/// unconstrained body — `if $X:` answers [] on files with real py bodies,
/// `def $A($B):` answers [] (was a wrong-hit over-face), and `class $A:`
/// keeps its sg-empty answer space (the class classifier refuses the Exactly
/// body, template-None; the census judges the sg-accepted spelling
/// answerable, so the served-empty walk IS the sg rc1-[] agreement).
/// `if $X: $B` keeps its agreeing answer.
#[test]
fn f122e_bare_colon_suite_is_sg_empty_suite() {
    let if_colon = match_pattern(Language::Python, "if x:\n    q()\n", "if $X:").unwrap();
    assert!(if_colon.is_empty(), "empty-suite if pattern must answer []");
    let def_colon =
        match_pattern(Language::Python, "def f(x):\n    return x\n", "def $A($B):").unwrap();
    assert!(
        def_colon.is_empty(),
        "empty-suite def pattern must answer []"
    );
    let class_colon = match_pattern(Language::Python, "class A:\n    pass\n", "class $A:").unwrap();
    assert!(
        class_colon.is_empty(),
        "empty-suite class pattern must answer []"
    );
    let if_meta = match_pattern(Language::Python, "if x: q()\n", "if $X: $B").unwrap();
    assert_eq!(lines_of(&if_meta), vec![1]);
}

// ---------------------------------------------------------------------------
// PASS 124 (r65 remediation, oracle grids /tmp/phase124): F1 if-condition
// capture conflict, F2 member-link trivia vetoes on the kt dotted + general
// lanes, F3 swift/py if-trivia transparency, the py colon-suite counting
// rule, and the F4/F5 registered envelopes.
// ---------------------------------------------------------------------------

/// F1 (f124a): `if ($X) { $B }` must answer an if whose CONDITION is a call
/// with >=1 argument, binding $X to the WHOLE condition text (sg oracle:
/// X="q(1)", X="a.b(1)", X="$o->m(1)", X="q(1, 2)"). The pre-124 defect
/// misread the condition meta as an argument template (pattern_argument_text
/// grabbed the first paren section), bound $X to the ARGUMENT text, and the
/// bind_capture conflict silently dropped the candidate.
#[test]
fn f124a_if_condition_call_binds_whole_condition_text() {
    for (lang, src, x, line) in [
        (Language::JavaScript, "if (q(1)) { b(); }\n", "q(1)", 1),
        (
            Language::JavaScript,
            "if (q(1, 2)) { b(); }\n",
            "q(1, 2)",
            1,
        ),
        (Language::JavaScript, "if (a.b(1)) { b(); }\n", "a.b(1)", 1),
        (
            Language::JavaScript,
            "if (q(q(1))) { b(); }\n",
            "q(q(1))",
            1,
        ),
        (Language::TypeScript, "if (q(1)) { b(); }\n", "q(1)", 1),
        // php cells live on line 2 (the `<?php` opener is line 1).
        (Language::Php, "<?php\nif (q(1)) { b(); }\n", "q(1)", 2),
        (
            Language::Php,
            "<?php\nif ($o->m(1)) { b(); }\n",
            "$o->m(1)",
            2,
        ),
    ] {
        let hits = match_pattern(lang, src, "if ($X) { $B }")
            .unwrap_or_else(|e| panic!("{lang} {src:?}: {e}"));
        assert_eq!(lines_of(&hits), vec![line], "{lang} {src:?}");
        assert_eq!(
            hits[0].captures.get("X").map(String::as_str),
            Some(x),
            "{lang} {src:?}: $X must bind the whole condition text"
        );
        assert_eq!(hits[0].captures.get("B").map(String::as_str), Some("b();"));
    }
    // Mixed file: the call-condition if must not vanish (subject lost one of
    // two sg hits pre-124).
    let mixed = match_pattern(
        Language::JavaScript,
        "if (a) { b(); }\nif (q(1)) { r(2); }\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&mixed), vec![1, 2]);
    // Paren-spelled go/py spellings carry the same defect (X = condition
    // text without the outer parens).
    let go = match_pattern(
        Language::Go,
        "package main\nfunc main() {\n\tif (q(1)) { b() }\n}\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&go), vec![3], "go paren-spelled call condition");
    assert_eq!(go[0].captures.get("X").map(String::as_str), Some("q(1)"));
    let py = match_pattern(Language::Python, "if (q(1)):\n    b()\n", "if ($X): $B").unwrap();
    assert_eq!(lines_of(&py), vec![1]);
    assert_eq!(py[0].captures.get("X").map(String::as_str), Some("q(1)"));
    // sg capture-text controls that must NOT move: non-call conditions bind
    // the condition text (including a double-paren face), zero-arg calls
    // answer, body-position calls never conflicted.
    for (lang, src, x) in [
        (Language::JavaScript, "if (a) { b(); }\n", "a"),
        (Language::JavaScript, "if (a > 1) { b(); }\n", "a > 1"),
        (Language::JavaScript, "if (!a) { b(); }\n", "!a"),
        (Language::JavaScript, "if ((a)) { b(); }\n", "(a)"),
        (Language::JavaScript, "if (q()) { b(); }\n", "q()"),
        (Language::Php, "<?php\nif ($a > 1) { b(); }\n", "$a > 1"),
    ] {
        let hits = match_pattern(lang, src, "if ($X) { $B }").unwrap();
        let expected_line = if lang == Language::Php { 2 } else { 1 };
        assert_eq!(lines_of(&hits), vec![expected_line], "{lang} {src:?}");
        assert_eq!(hits[0].captures.get("X").map(String::as_str), Some(x));
    }
    let body_call =
        match_pattern(Language::JavaScript, "if (a) { q(1); }\n", "if ($X) { $B }").unwrap();
    assert_eq!(lines_of(&body_call), vec![1]);
    assert_eq!(
        body_call[0].captures.get("B").map(String::as_str),
        Some("q(1);")
    );
    // Paren-free spellings were never affected (no `(` for the argument
    // misread) and keep answering.
    let go_pf = match_pattern(
        Language::Go,
        "package main\nfunc main() {\n\tif q(1) != nil { b() }\n}\n",
        "if $X { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&go_pf), vec![3]);
    let py_pf = match_pattern(Language::Python, "if q(1):\n    b()\n", "if $X: $B").unwrap();
    assert_eq!(lines_of(&py_pf), vec![1]);
}

/// F2 (f124b): the per-candidate member-link trivia veto must hold on EVERY
/// member path. The kt DOTTED call spelling (`a.b($X)` walk lane) and the
/// kt/swift dotted PROPERTY + `$`-less general-lane spellings over-answered
/// receiver-link trivia where sg refuses (oracle grids /tmp/phase124 f2/f2b);
/// callee-internal trivia, tails, and js/ts transparency stay put; the kt
/// `?.` step-0 postures and the swift per-candidate call grid (f122b) are
/// byte-frozen.
#[test]
fn f124b_member_link_trivia_veto_on_all_member_paths() {
    // kt dotted CALL: receiver-link trivia refuses on every spelling
    // ($-carrying walk lane AND the $-less general lane).
    for pat in ["a.b($X)", "$A.b($X)", "a.b(1)"] {
        let hits = match_pattern(Language::Kotlin, "val r = a /*c*/ .b(1)\n", pat).unwrap();
        assert!(hits.is_empty(), "kt recv trivia must refuse {pat:?}");
    }
    // kt dotted PROPERTY: recv/doubled trivia refuse; insuffix + tail + clean
    // keep their sg answers (insuffix n1, mid n0, clean n1, tail n1).
    {
        let pat = "a.b.c";
        let recv = match_pattern(Language::Kotlin, "val r = a /*c*/ .b.c\n", pat).unwrap();
        assert!(recv.is_empty(), "kt property recv trivia must refuse");
        let recv2 = match_pattern(Language::Kotlin, "val r = a /*c*/ /*d*/ .b.c\n", pat).unwrap();
        assert!(
            recv2.is_empty(),
            "kt property doubled recv trivia must refuse"
        );
    }
    let kt_insuffix = match_pattern(Language::Kotlin, "val r = a. /*c*/ b.c\n", "a.b.c").unwrap();
    assert_eq!(
        lines_of(&kt_insuffix),
        vec![1],
        "kt insuffix trivia transparent"
    );
    let kt_clean = match_pattern(Language::Kotlin, "val r = a.b.c\n", "a.b.c").unwrap();
    assert_eq!(lines_of(&kt_clean), vec![1]);
    // swift dotted PROPERTY: recv trivia refuses (the third member path),
    // tail trivia (outside the candidate) stays transparent.
    let sw_recv = match_pattern(Language::Swift, "let x = a /*c*/ .b.c\n", "a.b.c").unwrap();
    assert!(sw_recv.is_empty(), "swift property recv trivia must refuse");
    let sw_recv2 = match_pattern(Language::Swift, "let x = a /*c*/ /*d*/ .b.c\n", "a.b.c").unwrap();
    assert!(
        sw_recv2.is_empty(),
        "swift property doubled recv trivia must refuse"
    );
    let sw_tail = match_pattern(Language::Swift, "let x = a.b.c /*c*/\n", "a.b.c").unwrap();
    assert_eq!(
        lines_of(&sw_tail),
        vec![1],
        "tail trivia is outside the candidate"
    );
    let sw_clean = match_pattern(Language::Swift, "let x = a.b.c\n", "a.b.c").unwrap();
    assert_eq!(lines_of(&sw_clean), vec![1]);
    // swift $-less dotted CALL recv trivia (the registered §41.5 faces) now
    // closes the same way: the general lane consult fires.
    let sw_lit = match_pattern(Language::Swift, "let r = a /*c*/ .b(1)\n", "a.b(1)").unwrap();
    assert!(sw_lit.is_empty(), "swift $-less recv trivia must refuse");
    // kt `?.` step-0 postures byte-frozen.
    let ktopt_recv =
        match_pattern(Language::Kotlin, "val r = a /*c*/ ?.b(1)\n", "a?.b($X)").unwrap();
    assert!(ktopt_recv.is_empty(), "kt ?. recv trivia stays refused");
    let ktopt_recv_meta =
        match_pattern(Language::Kotlin, "val r = a /*c*/ ?.b(1)\n", "$A?.b($X)").unwrap();
    assert!(ktopt_recv_meta.is_empty());
    let ktopt_clean = match_pattern(Language::Kotlin, "val r = a?.b(1)\n", "a?.b($X)").unwrap();
    assert_eq!(lines_of(&ktopt_clean), vec![1]);
    let ktopt_callee =
        match_pattern(Language::Kotlin, "val r = a?. /*c*/ b(1)\n", "a?.b($X)").unwrap();
    assert_eq!(lines_of(&ktopt_callee), vec![1]);
    // kt OPTIONAL property chain (the gated `?.` property arm): recv trivia
    // refuses (oracle /tmp/phase124/f2c), insuffix + clean keep answering.
    let ktopt_prop_recv =
        match_pattern(Language::Kotlin, "val r = a /*c*/ ?.b?.c\n", "a?.b?.c").unwrap();
    assert!(
        ktopt_prop_recv.is_empty(),
        "kt optional property recv trivia must refuse"
    );
    let ktopt_prop_insuffix =
        match_pattern(Language::Kotlin, "val r = a?. /*c*/ b?.c\n", "a?.b?.c").unwrap();
    assert_eq!(
        lines_of(&ktopt_prop_insuffix),
        vec![1],
        "insuffix transparent"
    );
    let ktopt_prop_clean = match_pattern(Language::Kotlin, "val r = a?.b?.c\n", "a?.b?.c").unwrap();
    assert_eq!(lines_of(&ktopt_prop_clean), vec![1]);
    // kt mid-call trivia keeps its sg refusal; kt callee-internal trivia keeps
    // its sg transparency; kt clean call answers.
    let kt_mid = match_pattern(Language::Kotlin, "val r = a.b /*c*/ (1)\n", "a.b($X)").unwrap();
    assert!(kt_mid.is_empty(), "kt mid-call trivia stays refused");
    let kt_callee = match_pattern(Language::Kotlin, "val r = a. /*c*/ b(1)\n", "a.b($X)").unwrap();
    assert_eq!(lines_of(&kt_callee), vec![1]);
    let kt_call_clean = match_pattern(Language::Kotlin, "val r = a.b(1)\n", "a.b($X)").unwrap();
    assert_eq!(lines_of(&kt_call_clean), vec![1]);
    // swift per-candidate call grid (f122b) byte-frozen: inner link answers.
    let sw_inner =
        match_pattern(Language::Swift, "let r = a.b(1) /*c*/ .c(2)\n", "a.b($X)").unwrap();
    assert_eq!(lines_of(&sw_inner), vec![1]);
    let sw_call_recv =
        match_pattern(Language::Swift, "let r = a /*c*/ .b(1)\n", "a.b($X)").unwrap();
    assert!(sw_call_recv.is_empty());
    // js/ts comment transparency byte-frozen.
    for lang in [Language::JavaScript, Language::TypeScript] {
        let hits = match_pattern(lang, "a /*c*/ .b(1);\n", "a.b($X)").unwrap();
        assert_eq!(
            lines_of(&hits),
            vec![1],
            "{lang} transparency must not move"
        );
    }
}

/// F3 (f124c): sg treats if-level comment trivia as TRANSPARENT on swift and
/// python (every probed position answers n1) while js/kt/go stay STRUCTURAL
/// (pre-condition/pre-brace trivia refuses, 122 grid + f3 controls). The
/// grammar-blind direct-trivia-before-consequence guard must not fire for
/// swift/py.
#[test]
fn f124c_swift_py_if_trivia_transparent_structural_elsewhere() {
    // Swift: trivia at every if-level position keeps the sg n1 answer.
    for src in [
        "if c /*c*/ {\n    d()\n}\n",
        "if /*c*/ c {\n    d()\n}\n",
        "if c {\n    /*c*/ d()\n}\n",
        "if c { /*c*/ d() }\n",
        "if q(1) /*c*/ {\n    d()\n}\n",
    ] {
        let hits = match_pattern(Language::Swift, src, "if $X { $B }")
            .unwrap_or_else(|e| panic!("swift {src:?}: {e}"));
        assert_eq!(lines_of(&hits), vec![1], "swift {src:?} must answer");
    }
    // Python: direct if-level trivia (inside/after the condition) and body
    // trivia keep answering; the condition capture stays sg-shaped.
    let py_cond = match_pattern(Language::Python, "if x # c\n:\n    b()\n", "if $X: $B").unwrap();
    assert_eq!(lines_of(&py_cond), vec![1], "py condition-adjacent trivia");
    let py_body =
        match_pattern(Language::Python, "if x:\n    # c\n    b()\n", "if $X: $B").unwrap();
    assert_eq!(lines_of(&py_body), vec![1], "py body-start comment");
    // Structural grammars keep the refusal (f122a + f3 controls).
    for (lang, src) in [
        (Language::JavaScript, "if (a) /*c*/ { b(); }\n"),
        (Language::JavaScript, "if /*c*/ (a) { b(); }\n"),
        (Language::Kotlin, "fun m() {\n    if (c) /*c*/ { d() }\n}\n"),
        (
            Language::Go,
            "package main\nfunc main() {\n\tif a /*c*/ { b() }\n}\n",
        ),
    ] {
        let hits = match_pattern(lang, src, "if ($X) { $B }").unwrap();
        assert!(
            hits.is_empty(),
            "{lang} {src:?} must stay structural-refused"
        );
    }
    // js body/pre-else transparency byte-frozen.
    let js_body = match_pattern(
        Language::JavaScript,
        "if (a) { /*c*/ b(); }\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&js_body), vec![1]);
    let js_preelse = match_pattern(
        Language::JavaScript,
        "if (a) { b(); } /*c*/ else { c(); }\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&js_preelse), vec![1]);
    // swift clean faces byte-frozen.
    let sw_clean_else = match_pattern(
        Language::Swift,
        "if c {\n    d()\n} /*c*/ else {\n    e()\n}\n",
        "if $X { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&sw_clean_else), vec![1]);
}

/// 123B-F1 (f124d): sg's py colon-suite rule — `if $X: $B` answers EVERY
/// python if and binds $B to the WHOLE suite text regardless of statement
/// count (oracle /tmp/phase124 fpy: two/three-statement indented suites and
/// the one-line `a(); b()` suite all answer n1 with B = suite text). The
/// pre-124 Exactly(1) statement count refused every multi-statement suite.
/// The f122e bare-colon EMPTY-suite pin is preserved (Exactly(0) refuses).
#[test]
fn f124d_py_colon_suite_binds_whole_suite_any_count() {
    for (src, b) in [
        ("if x:\n    a()\n    b()\n", "a()\n    b()"),
        (
            "if x:\n    a()\n    b()\n    c()\n",
            "a()\n    b()\n    c()",
        ),
        ("if x: a(); b()\n", "a(); b()"),
    ] {
        let hits = match_pattern(Language::Python, src, "if $X: $B")
            .unwrap_or_else(|e| panic!("py {src:?}: {e}"));
        assert_eq!(lines_of(&hits), vec![1], "py {src:?} must answer");
        assert_eq!(
            hits[0].captures.get("B").map(String::as_str),
            Some(b),
            "py {src:?}: $B must bind the whole suite text"
        );
    }
    // Trivia-carrying suites answer like clean ones (sg n1).
    let triv = match_pattern(
        Language::Python,
        "if x:\n    a()\n    # c\n    b()\n",
        "if $X: $B",
    )
    .unwrap();
    assert_eq!(lines_of(&triv), vec![1]);
    // Single-statement and nested faces keep their agreeing answers.
    let single = match_pattern(Language::Python, "if x:\n    a()\n", "if $X: $B").unwrap();
    assert_eq!(lines_of(&single), vec![1]);
    assert_eq!(single[0].captures.get("B").map(String::as_str), Some("a()"));
    let nested = match_pattern(
        Language::Python,
        "if x:\n    if y:\n        a()\n",
        "if $X: $B",
    )
    .unwrap();
    assert_eq!(lines_of(&nested), vec![1, 2]);
    // The paren-spelled py condition spelling carries the same rule.
    let ps = match_pattern(
        Language::Python,
        "if (x):\n    a()\n    b()\n",
        "if ($X): $B",
    )
    .unwrap();
    assert_eq!(lines_of(&ps), vec![1]);
    // f122e pin: the bare-colon EMPTY-suite pattern still refuses.
    let bare = match_pattern(Language::Python, "if x:\n    a()\n    b()\n", "if $X:").unwrap();
    assert!(bare.is_empty(), "bare-colon empty-suite pin must hold");
}

/// F4 (f124e): the 13 census-loud faces where sg 0.45.2 answers (oracle
/// /tmp/phase124 f4) keep their registered fail-closed loud class. All are
/// fail-closed genus (zero wrong hits); each stays pinned so the census
/// cannot silently widen. Form-1 predicates live in CNR §42.
#[test]
fn f124e_census_loud_faces_where_sg_answers_stay_registered() {
    for pat in [
        // switch/match heads, do-while, static block,
        // object-method shorthand, template roots.
        "switch ($X) { $$$B }",
        "match $X { $$$B }",
        "do { $$$B } while ($X);",
        // PASS 135 SUPERSESSION: the `q: while ($X) { $B }` labeled row left
        // this list — the labeled_statement root admission (grids
        // F_js_labeled/X_js_label_concrete/Y_java_labeled, f135e) gives the
        // walk sg-exact machinery for the face, so it answers instead of
        // staying census-loud (corrected of record in CNR §45).
        "class q { static { $$$B } }",
        "{ q($X) { $$$B } }",
        "`${q($X)}`",
        "`$X`",
        // concrete-condition ifs and dollars-in-condition (§30.6 genus).
        "if (q($A)) x();",
        "if (q($A)) { x(); }",
        "if ($$$B) { $A }",
        // else-arm meta.
        "if ($X) { $B } else $C",
    ] {
        assert!(
            needs_ast_grep_fallback(pat),
            "{pat} must keep the registered census-loud class"
        );
    }
    // jsx face: sg supports jsx; the subject keeps its loud class until the
    // jsx root kind gains walk machinery.
    assert!(
        !native_pattern_answerable(Language::JavaScript, "<div q={$X} />")
            || needs_ast_grep_fallback("<div q={$X} />")
    );
}

/// F5 (f124f): the same-line else-if containment envelope. The WALK emits
/// both hits of a ONE-LINE chain (n2 — sg-agreeing at this layer); the
/// registered subject-vs-sg n1 divergence lives in the CLI search layer's
/// same-line hit dedup (registered F66a-7/§41.3, mechanism corrected from
/// "outermost-wins span containment" in CNR §42: multi-line chains present
/// n2 at the CLI too). Pinned so the envelope's cell set cannot drift
/// unnoticed.
#[test]
fn f124f_one_line_else_if_containment_registered_envelope() {
    // Registered divergence cell: the walk emits n2 (both line 1); the CLI
    // dedup presents the registered n1-vs-sg-n2 row.
    let oneline = match_pattern(
        Language::JavaScript,
        "if (a) { b(); } else if (c) { d(); }\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(oneline.len(), 2, "walk emits both same-line hits");
    assert_eq!(
        klass_lines(&oneline),
        vec![1],
        "registered §41.3 cell: same-line dedup at CLI"
    );
    // Agreeing cells byte-frozen: multi-line chain answers both; the inner
    // brace-less else-if refuses under the braced pattern (brace-ness).
    let multiline = match_pattern(
        Language::JavaScript,
        "if (a) { b(); }\nelse if (c) { d(); }\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&multiline), vec![1, 2]);
    let inner_braceless = match_pattern(
        Language::JavaScript,
        "if (a) { b(); }\nelse if (c) d();\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&inner_braceless),
        vec![1],
        "brace-ness structural on the else-if arm"
    );
    let chain3 = match_pattern(
        Language::JavaScript,
        "if (a) { b(); } else if (c) { d(); } else if (e) { f(); }\n",
        "if ($X) { $B }",
    )
    .unwrap();
    assert_eq!(chain3.len(), 3, "three same-line hits at the walk layer");
    assert_eq!(klass_lines(&chain3), vec![1], "registered §41.3 cell");
}

/// F3 fold (f124g): the keyword-literal root list is closed and must stay
/// mutation-covered — `False` and `super` were missing from the f122c loop
/// (a mutant deleting either arm survived the suite).
#[test]
fn f124g_keyword_literal_root_list_includes_false_and_super() {
    assert!(ast_sgrep_lang::pattern_is_keyword_literal_root("False"));
    assert!(ast_sgrep_lang::pattern_is_keyword_literal_root("super"));
    // The py False / js super faces answer through the walk (F3's closed
    // class, sg n1 each).
    let py_false = match_pattern(Language::Python, "x = False\n", "False").unwrap();
    assert_eq!(lines_of(&py_false), vec![1]);
    let js_super = match_pattern(
        Language::JavaScript,
        "class A extends B {\n  m() { super.m(); }\n}\n",
        "super",
    )
    .unwrap();
    assert_eq!(lines_of(&js_super), vec![2]);
}

#[test]
fn tmp_kt_probe_124_converted_kt_link_trivia_pins() {
    // PASS 127 (125B-F3): the zero-assertion diagnostic probe is now a REAL
    // pinned assertion encoding the kt probe's intended face. sg 0.45.2
    // refuses the receiver-side link trivia (`a /*c*/ .b(1)` × `a.b(1)` →
    // [], the §39.13/§41.4 doctrine grids) and the subject's kt lanes agree
    // — pin both cells so a future over-refusal (clean face lost) or
    // over-answer (trivia face answered) fails here.
    let trivia = match_pattern(Language::Kotlin, "val r = a /*c*/ .b(1)\n", "a.b(1)").unwrap();
    assert!(
        trivia.is_empty(),
        "kt receiver-link trivia must refuse like sg (§39.13), got {trivia:?}"
    );
    let clean = match_pattern(Language::Kotlin, "val r = a.b(1)\n", "a.b(1)").unwrap();
    assert_eq!(lines_of(&clean), vec![1]);
    let prop_trivia = match_pattern(Language::Kotlin, "val r = a /*c*/ .b.c\n", "a.b.c").unwrap();
    assert!(
        prop_trivia.is_empty(),
        "kt receiver-link trivia on a property chain must refuse like sg, got {prop_trivia:?}"
    );
}

fn f127_capture<'a>(hits: &'a [ast_sgrep_lang::PatternMatch], name: &str) -> Option<&'a str> {
    hits[0].captures.get(name).map(String::as_str)
}

/// PASS 127 (125A-F5, f127a): php assignment targets carrying canonical
/// metavariables BIND like sg 0.45.2 (oracle grid /tmp/phase127/g1).
/// Failure-first: every sg-binding cell answered silent `ok:true []` (whole
/// meta) or rc2-loud before the meta-target admission.
#[test]
fn f127a_php_assignment_meta_targets_bind_like_sg() {
    // (a) meta member LINK — the F5a SILENT-under cells; sg binds A to the
    // link name text (`a` literal / `$a` dynamic).
    let lit = match_pattern(Language::Php, "<?php\n$o->a = 1;\n", "$o->$A = $Y").unwrap();
    assert_eq!(lines_of(&lit), vec![2]);
    assert_eq!(f127_capture(&lit, "A"), Some("a"));
    assert_eq!(f127_capture(&lit, "Y"), Some("1"));
    let dyn_link = match_pattern(Language::Php, "<?php\n$o->$a = 1;\n", "$o->$A = $Y").unwrap();
    assert_eq!(lines_of(&dyn_link), vec![2]);
    assert_eq!(f127_capture(&dyn_link, "A"), Some("$a"));
    // (b) whole-meta LHS, `;`-LESS spellings (sg ANSWERS; the §26.2
    // "no metas" premise is `;`-parent-scoped and corrected in CNR).
    let whole = match_pattern(Language::Php, "<?php\n$x = q(1);\n", "$X = $Y").unwrap();
    assert_eq!(lines_of(&whole), vec![2]);
    assert_eq!(f127_capture(&whole, "X"), Some("$x"));
    assert_eq!(f127_capture(&whole, "Y"), Some("q(1)"));
    let alpha = match_pattern(Language::Php, "<?php\n$x = q(1);\n", "$ALPHA = $V").unwrap();
    assert_eq!(f127_capture(&alpha, "ALPHA"), Some("$x"));
    assert_eq!(f127_capture(&alpha, "V"), Some("q(1)"));
    // (c) the registered loud classes are UNMOVED: the `;`-ful whole-meta
    // spellings sg refuses (f5c_semi/f5c_alpha_semi sg n0) stay
    // unanswerable (census loud), and the `list()` LHS (no admission,
    // sg answers — registered form-1 pending the list-target machinery)
    // stays loud.
    assert!(!native_pattern_answerable(Language::Php, "$X = $Y;"));
    assert!(!native_pattern_answerable(Language::Php, "$Alpha = $V;"));
    assert!(!native_pattern_answerable(
        Language::Php,
        "list($X, $Y) = $Z"
    ));
    // (d) go short-var declarations answer like sg (px_go cells).
    let go = "package main\n\nfunc f() {\n\tx := q()\n\t_ = x\n}\n";
    let go_short = match_pattern(Language::Go, go, "$X := $Y").unwrap();
    assert_eq!(lines_of(&go_short), vec![4]);
    assert_eq!(f127_capture(&go_short, "X"), Some("x"));
    assert_eq!(f127_capture(&go_short, "Y"), Some("q()"));
    let go_lhs = match_pattern(Language::Go, go, "x := $Y").unwrap();
    assert_eq!(f127_capture(&go_lhs, "Y"), Some("q()"));
}

/// PASS 127 (125A-F2, f127b): the loop/for-family template roots whose
/// sg 0.45.2 answers (grids /tmp/phase127/g1 + cells2) answer sg-exactly
/// through the kind-exact general lane. Failure-first: every cell was
/// census-loud rc2 before the root-kind + head admissions.
#[test]
fn f127b_loop_roots_answer_like_sg() {
    let js_for = match_pattern(
        Language::JavaScript,
        "for (let i = 0; i < 10; i++) { q(i); }\n",
        "for (let $I = 0; $I < 10; $I++) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&js_for), vec![1]);
    assert_eq!(f127_capture(&js_for, "I"), Some("i"));
    assert_eq!(f127_capture(&js_for, "B"), Some("q(i);"));
    let js_for_of = match_pattern(
        Language::JavaScript,
        "for (const x of xs) { q(x); }\n",
        "for (const $X of $Y) { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&js_for_of, "X"), Some("x"));
    assert_eq!(f127_capture(&js_for_of, "Y"), Some("xs"));
    let js_for_in = match_pattern(
        Language::JavaScript,
        "for (const k in o) { q(k); }\n",
        "for (const $X in $Y) { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&js_for_in, "X"), Some("k"));
    let ts_for_of = match_pattern(
        Language::TypeScript,
        "for (const x of xs) { q(x); }\n",
        "for (const $X of $Y) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&ts_for_of), vec![1]);
    // js while — the §30.6 registered row CORRECTED: sg answers n1 braced
    // AND brace-less (oracle g1 f2_js_while_meta + cells2 o1).
    let js_while = match_pattern(
        Language::JavaScript,
        "while (a) { b(); }\n",
        "while ($X) { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&js_while, "X"), Some("a"));
    assert_eq!(f127_capture(&js_while, "B"), Some("b();"));
    let js_while_bare =
        match_pattern(Language::JavaScript, "while (a) b();\n", "while ($X) $B").unwrap();
    assert_eq!(lines_of(&js_while_bare), vec![1]);
    // js do-while single-meta body (oracle o2).
    let js_do = match_pattern(
        Language::JavaScript,
        "do { b(); } while (a);\n",
        "do { $B } while ($X);",
    )
    .unwrap();
    assert_eq!(f127_capture(&js_do, "B"), Some("b();"));
    // go: all four head forms (oracle g1 go cells).
    let go_wrap = |body: &str| format!("package main\n\nfunc f() {{\n\t{body}\n}}\n");
    let go_for = match_pattern(
        Language::Go,
        &go_wrap("for i := 0; i < 10; i++ {\n\t\tq(i)\n\t}"),
        "for $I := 0; $I < 10; $I++ { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&go_for, "I"), Some("i"));
    assert_eq!(f127_capture(&go_for, "B"), Some("q(i)\n"));
    let go_range = match_pattern(
        Language::Go,
        "package main\n\nfunc f(xs []int) {\n\tfor i := range xs {\n\t\tq(i)\n\t}\n}\n",
        "for $X := range $Y { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&go_range, "X"), Some("i"));
    assert_eq!(f127_capture(&go_range, "Y"), Some("xs"));
    let go_bare =
        match_pattern(Language::Go, &go_wrap("for {\n\t\tq()\n\t}"), "for { $B }").unwrap();
    assert_eq!(f127_capture(&go_bare, "B"), Some("q()\n"));
    let go_while_form = match_pattern(
        Language::Go,
        &go_wrap("for a < 10 {\n\t\tb()\n\t}"),
        "for $X { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&go_while_form, "X"), Some("a < 10"));
    // rust: loop / for-in / while-let (oracle g1 rs cells).
    let rs = "fn main() {\n    loop {\n        q();\n    }\n}\n";
    let rs_loop = match_pattern(Language::Rust, rs, "loop { $B }").unwrap();
    assert_eq!(f127_capture(&rs_loop, "B"), Some("q();"));
    let rs_for = match_pattern(
        Language::Rust,
        "fn main() {\n    for x in xs {\n        q(x);\n    }\n}\n",
        "for $X in $Y { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&rs_for, "X"), Some("x"));
    assert_eq!(f127_capture(&rs_for, "Y"), Some("xs"));
    let rs_while_let = match_pattern(
        Language::Rust,
        "fn main() {\n    while let Some(x) = it.next() {\n        q(x);\n    }\n}\n",
        "while let $P = $E { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&rs_while_let, "P"), Some("Some(x)"));
    assert_eq!(f127_capture(&rs_while_let, "E"), Some("it.next()"));
    // php: the brace-less while spelling stays census-loud (registered
    // form-1, see f127b_boundaries — capture-text divergence vs sg).
}

/// PASS 127 (125A-F2 boundaries): the registered multi-span / suite faces
/// KEEP their loud classes — `$$$`-bodied statement templates are refused
/// upstream of the general lane (the `$$$` text gate), and the python
/// `:`-suite alignment the general lane cannot express stays census-loud
/// (registered form-1: suite-text binding machinery per the 123B-F1 model).
#[test]
fn f127b_loop_registered_loud_boundaries_hold() {
    assert!(needs_ast_grep_fallback("do { $$$B } while ($X);"));
    assert!(needs_ast_grep_fallback("for { $$$B }"));
    assert!(needs_ast_grep_fallback("(function() { $$$B })()"));
    assert!(needs_ast_grep_fallback("for ($$$A) { $B }"));
    assert!(!native_pattern_answerable(
        Language::Python,
        "for $X in $Y: $B"
    ));
    assert!(!native_pattern_answerable(Language::Python, "while $X: $B"));
    // php: the loop faces whose metas land in php VARIABLE positions
    // (foreach `as`-targets, braced `{ $B }` bodies) and the brace-less
    // while (capture-text divergence: the subject binds the php
    // expression-statement text `b()` where sg binds the full statement
    // `b();`) stay census-loud — registered form-1 rows: the general
    // lane's `$`-stripping substitution cannot spell php variable
    // positions, and sg binds the whole suite/statement text (cells2
    // o3/o4/o7/o8/o20). Retry predicate: a php variable-preserving meta
    // substitution (or a dedicated foreach/while lane with sg's
    // statement-span binding) for the php rows; the 123B-F1 suite-text
    // binding model generalized to loop suites for the py rows.
    assert!(!native_pattern_answerable(
        Language::Php,
        "foreach ($X as $Y) { $B }"
    ));
    assert!(!native_pattern_answerable(
        Language::Php,
        "foreach ($X as $K => $V) { $B }"
    ));
    assert!(!native_pattern_answerable(
        Language::Php,
        "foreach ($X as $Y) $B"
    ));
    assert!(!native_pattern_answerable(
        Language::Php,
        "while ($X) { $B }"
    ));
    assert!(!native_pattern_answerable(Language::Php, "while ($X) $B"));
    assert!(!native_pattern_answerable(
        Language::Php,
        "do { $B } while ($X);"
    ));
    // ruby end-terminated roots stay census-loud (125A-F1 registered
    // form-1: multi-line template lane per the 123B-F1 suite-text model).
    assert!(!native_pattern_answerable(
        Language::Ruby,
        "if $X\n  $B\nend"
    ));
    assert!(!native_pattern_answerable(
        Language::Ruby,
        "while $X\n  $B\nend"
    ));
    // rust `let` census contract and the brace-less if refusal boundary
    // (§30.6's truly-sg-refused cells) are unmoved.
    assert!(!native_pattern_answerable(Language::Rust, "let $A = $B"));
    assert!(!native_pattern_answerable(
        Language::JavaScript,
        "if ($X) $B"
    ));
}

/// PASS 127 (125A-F4, f127c): keyword/unary operator template roots answer
/// like sg (grids /tmp/phase127/g1 + cells2 o6/o11/o19). Failure-first:
/// every cell was census-loud before the keyword-operator/head admissions.
#[test]
fn f127c_keyword_operator_roots_answer_like_sg() {
    let py_and = match_pattern(Language::Python, "if a and b:\n    q()\n", "$X and $Y").unwrap();
    assert_eq!(f127_capture(&py_and, "X"), Some("a"));
    assert_eq!(f127_capture(&py_and, "Y"), Some("b"));
    let py_or = match_pattern(Language::Python, "if a or b:\n    q()\n", "$X or $Y").unwrap();
    assert_eq!(lines_of(&py_or), vec![1]);
    let py_not = match_pattern(Language::Python, "if not a:\n    q()\n", "not $X").unwrap();
    assert_eq!(f127_capture(&py_not, "X"), Some("a"));
    let rb_and = match_pattern(Language::Ruby, "if a and b\n  q()\nend\n", "$X and $Y").unwrap();
    assert_eq!(f127_capture(&rb_and, "X"), Some("a"));
    assert_eq!(f127_capture(&rb_and, "Y"), Some("b"));
    let rb_or = match_pattern(Language::Ruby, "if a or b\n  q()\nend\n", "$X or $Y").unwrap();
    assert_eq!(lines_of(&rb_or), vec![1]);
    // rb `!$X`: PASS 129 CORRECTS the 125A-F4 registered premise — the
    // oracle re-grid answers `!$X` n1 X=flag (g_rb_bang), so the face is
    // answerable, NOT census-loud; the `unary` root admission carries it
    // (f129f). Retry predicate retired: the dedicated rb unary arm exists.
    let rb_bang = match_pattern(Language::Ruby, "if !flag\n  q()\nend\n", "!$X").unwrap();
    assert_eq!(f127_capture(&rb_bang, "X"), Some("flag"));
    let js_typeof =
        match_pattern(Language::JavaScript, "var a = typeof b;\n", "typeof $X").unwrap();
    assert_eq!(f127_capture(&js_typeof, "X"), Some("b"));
    let ts_as = match_pattern(Language::TypeScript, "var a = b as string;\n", "$X as $Y").unwrap();
    assert_eq!(f127_capture(&ts_as, "X"), Some("b"));
    assert_eq!(f127_capture(&ts_as, "Y"), Some("string"));
    // ts has no `or` keyword operator: sg semantic-empty (cells2 o10) and
    // the subject stays census-loud — the registered loudness-skew genus.
    assert!(!native_pattern_answerable(Language::TypeScript, "$X or $Y"));
    // M-127f kill: the keyword-operator exemption lives at the INGRESS gate
    // only (the library walk does not consult general_lane_supported), so
    // the sg-answerable no-punctuation spellings must be ingress-admitted.
    assert!(!needs_ast_grep_fallback("$X and $Y"));
    assert!(!needs_ast_grep_fallback("not $X"));
    assert!(!needs_ast_grep_fallback("typeof $X"));
    assert!(!needs_ast_grep_fallback("$X := $Y"));
}

/// PASS 127 (125A-F3, f127d): php/rb statement-head siblings answer like sg
/// (oracle b2 cells + o15/o16/o18). Failure-first: all three were
/// census-loud before the php wrapped-lane / rb `return` root admissions.
#[test]
fn f127d_php_rb_statement_heads_answer_like_sg() {
    let php_return = match_pattern(
        Language::Php,
        "<?php\nfunction f() { return q(1); }\n",
        "return $X;",
    )
    .unwrap();
    assert_eq!(f127_capture(&php_return, "X"), Some("q(1)"));
    let php_throw = match_pattern(
        Language::Php,
        "<?php\nthrow new Exception('x');\n",
        "throw $X;",
    )
    .unwrap();
    assert_eq!(f127_capture(&php_throw, "X"), Some("new Exception('x')"));
    let rb_return =
        match_pattern(Language::Ruby, "def f\n  return q(1)\nend\n", "return $X").unwrap();
    assert_eq!(lines_of(&rb_return), vec![2]);
    assert_eq!(f127_capture(&rb_return, "X"), Some("q(1)"));
}

/// PASS 127 (125B-F1, f127h): `is_if_prefixed` accepts the whitespace
/// spellings `classify_if_template` always tolerated. Failure-first: the
/// newline-spelled pattern classified as If yet SKIPPED the argument-capture
/// skip, so `capture_arguments` misbound `$X` from the condition's inner
/// call and the `bind_capture` conflict silently dropped every candidate
/// whose condition holds a call (`if (q(1)) …` → n0); the paren-free py
/// spelling lost its `: $B` body binding.
#[test]
fn f127h_if_whitespace_spellings_bind_like_sg() {
    // The original F1 silent-drop trigger, one newline away.
    let drop_face = match_pattern(
        Language::JavaScript,
        "if (q(1)) { b(); }\n",
        "if\n($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&drop_face), vec![1]);
    assert_eq!(f127_capture(&drop_face, "X"), Some("q(1)"));
    assert_eq!(f127_capture(&drop_face, "B"), Some("b();"));
    let tab_face =
        match_pattern(Language::JavaScript, "if (a) { b(); }\n", "if\t($X) { $B }").unwrap();
    assert_eq!(f127_capture(&tab_face, "X"), Some("a"));
    assert_eq!(f127_capture(&tab_face, "B"), Some("b();"));
    let py_face = match_pattern(Language::Python, "if a:\n    q()\n", "if\n$X: $B").unwrap();
    assert_eq!(lines_of(&py_face), vec![1]);
    assert_eq!(f127_capture(&py_face, "X"), Some("a"));
    assert_eq!(f127_capture(&py_face, "B"), Some("q()"));
}

/// PASS 129 (128A-F4, f129a), CORRECTED by PASS 131 (130A-F1, CNR §44.1):
/// fully-literal STATIC scoped assignment targets (`C::$s`, `parent::$v`,
/// `static::$v`, literal + meta-index subscript tails, namespaced heads,
/// binary faces) BIND like sg 0.45.2. The pass-129 closing claim — "the
/// ONLY refused RHS shape is the single-bare-meta-arg call `f($V)`" — was
/// REFUTED by the r68 oracle re-probe (`C::$s = f($V)` sg n1 V=5, oracle
/// cell a_fcall); the pin below is flipped to the binding receipt.
#[test]
fn f129a_php_static_scope_targets_bind_like_sg() {
    let hit = match_pattern(
        Language::Php,
        "<?php\nclass C { public static $s = 0; }\nC::$s = 5;\n",
        "C::$s = $V",
    )
    .unwrap();
    assert_eq!(lines_of(&hit), vec![3]);
    assert_eq!(f127_capture(&hit, "V"), Some("5"));
    // The `;`-terminated pattern binds too (sg n1 — the `;`-refusal is a
    // WHOLE-META-LHS-only rule; 128B-F1's oracle adjudication).
    let semi = match_pattern(
        Language::Php,
        "<?php\nclass C { public static $s = 0; }\nC::$s = 5;\n",
        "C::$s = $V;",
    )
    .unwrap();
    assert_eq!(lines_of(&semi), vec![3]);
    // Multi-instance sources bind per-instance (sg n2: V=5, V=6).
    let multi = match_pattern(
        Language::Php,
        "<?php\nclass D { public static $cnt = 0; }\nD::$cnt = 5;\nD::$cnt = 6;\n",
        "D::$cnt = $V",
    )
    .unwrap();
    assert_eq!(lines_of(&multi), vec![3, 4]);
    assert_eq!(f127_capture(&multi, "V"), Some("5"));
    // In-class `parent::`/`static::` byte-compare bindings (sg n1 inside the
    // class body).
    let in_class = match_pattern(
        Language::Php,
        "<?php\nclass E { public static $v = 0; function f() { parent::$v = 8; static::$v = 9; } }\n",
        "parent::$v = $V",
    )
    .unwrap();
    assert_eq!(f127_capture(&in_class, "V"), Some("8"));
    let static_kw = match_pattern(
        Language::Php,
        "<?php\nclass E { public static $v = 0; function f() { parent::$v = 8; static::$v = 9; } }\n",
        "static::$v = $V",
    )
    .unwrap();
    assert_eq!(f127_capture(&static_kw, "V"), Some("9"));
    // Literal-subscript tails bind (sg n1 V=3); meta subscripts stay out.
    let sub = match_pattern(
        Language::Php,
        "<?php\nclass C { public static $s = []; }\nC::$s[0] = 3;\n",
        "C::$s[0] = $V",
    )
    .unwrap();
    assert_eq!(f127_capture(&sub, "V"), Some("3"));
    // Compound ops bind sg-exact (+= / -= / .= oracle cells f4e/ops).
    let compound = match_pattern(
        Language::Php,
        "<?php\nclass C { public static $s = 0; }\nC::$s += 5;\n",
        "C::$s += $V",
    )
    .unwrap();
    assert_eq!(f127_capture(&compound, "V"), Some("5"));
    // - PASS 131 (130A-F1, f131a): the pass-129 `C::$s = f($V)` EMPTY pin is
    //   FLIPPED — the r68 oracle re-probe REFUTED its receipt (sg n1 V=5,
    //   oracle cell a_fcall; CNR §44.1 corrected of record). The face now
    //   binds through the structural rhs_expr machinery, while the admitted
    //   expression-RHS shapes still bind.
    let call_rhs = match_pattern(
        Language::Php,
        "<?php\nclass C { public static $s = 0; }\nC::$s = f(5);\n",
        "C::$s = f($V)",
    )
    .unwrap();
    assert_eq!(f127_capture(&call_rhs, "V"), Some("5"));
    let expr_rhs = match_pattern(
        Language::Php,
        "<?php\nclass C { public static $s = 0; }\nC::$s = 5 + 1;\n",
        "C::$s = $V + 1",
    )
    .unwrap();
    assert_eq!(f127_capture(&expr_rhs, "V"), Some("5"));
    // - wrong-class / whole-meta-with-static-RHS faces answer empty exactly
    //   like sg. The literal-index face `C::$s[$k]` vs a `[0]` candidate
    //   keeps its honest empty too — but for a CORRECTED reason: the
    //   lowercase `$k` spelling is sg's LITERAL variable read (oracle
    //   a2_lowsub binds the text-equal candidate, no capture — the pass-129
    //   "sg refuses `C::$s[$k]`" receipt is refuted, CNR §44.1), so the
    //   mismatch here is the verbatim index byte compare, not a spelling
    //   refusal.
    assert!(match_pattern(
        Language::Php,
        "<?php\nclass C { public static $s = 0; }\nD::$s = 6;\n",
        "C::$s = $V"
    )
    .unwrap()
    .is_empty());
    assert!(match_pattern(
        Language::Php,
        "<?php\nclass C { public static $s = []; }\nC::$s[0] = 3;\n",
        "C::$s[$k] = $V"
    )
    .unwrap()
    .is_empty());
    // `$X = E::$v`: answerable (whole-meta LHS admission, 127) and composes
    // honest empty on a literal LHS source — AGREE with sg's [] (probe
    // f4b_wholemeta). The face is a census-loud-when-empty pin, not a
    // native-answer pin.
    assert!(match_pattern(
        Language::Php,
        "<?php\nclass E { public static $v = 0; }\nE::$v = 1;\n",
        "$X = E::$v"
    )
    .unwrap()
    .is_empty());
    // 128B-F2 census pins: the whole-meta compound faces sg REFUSES stay
    // census-loud (the H-CONF-029 loud-when-empty posture), and the semi
    // whole-meta control is unmoved.
    assert!(!native_pattern_answerable(Language::Php, "$X += $Y"));
    assert!(!native_pattern_answerable(Language::Php, "$X -= $Y"));
    assert!(!native_pattern_answerable(Language::Php, "$X = $Y;"));
}

/// PASS 129 (128A-F5, f129b): js/ts member-call chain decomposition is
/// comment-TRANSPARENT at every link position (receiver, between links,
/// callee-internal — oracle grid /tmp/phase129/cells/f5*) while the
/// callee→`(` junction veto holds at EVERY consumed call level (sg refuses
/// head AND interior junction comments). kt/swift trivia chains keep their
/// registered refusals.
#[test]
fn f129b_chain_interior_link_trivia_binds_like_sg() {
    // The 128A-F5 repro: trivia between link 1 and link 2.
    let link2 =
        match_pattern(Language::JavaScript, "a.b(1)/*m*/.c(2);\n", "a.b($X).c($Y)").unwrap();
    assert_eq!(lines_of(&link2), vec![1]);
    assert_eq!(f127_capture(&link2, "X"), Some("1"));
    assert_eq!(f127_capture(&link2, "Y"), Some("2"));
    // Receiver-position trivia on a 2-link chain (§39.6's single-link fix
    // never covered the chain lane).
    let recv = match_pattern(
        Language::JavaScript,
        "a /*r*/ .b(1).c(2);\n",
        "a.b($X).c($Y)",
    )
    .unwrap();
    assert_eq!(f127_capture(&recv, "X"), Some("1"));
    // Meta head binds the clean identifier text (sg A=a, X=1, Y=2).
    let metahead = match_pattern(
        Language::JavaScript,
        "a.b(1)/*m*/.c(2);\n",
        "$A.b($X).c($Y)",
    )
    .unwrap();
    assert_eq!(f127_capture(&metahead, "A"), Some("a"));
    // 3-link chains: interior comments at either boundary.
    let three = match_pattern(
        Language::JavaScript,
        "a.b(1)/*m*/.c(2)/*n*/.d(3);\n",
        "a.b($X).c($Y).d($Z)",
    )
    .unwrap();
    assert_eq!(f127_capture(&three, "Z"), Some("3"));
    // ts twin.
    let ts = match_pattern(Language::TypeScript, "a.b(1)/*m*/.c(2);\n", "a.b($X).c($Y)").unwrap();
    assert_eq!(lines_of(&ts), vec![1]);
    // Callee-internal position stays transparent (§39.6 chain twin).
    let internal = match_pattern(
        Language::JavaScript,
        "a.b(1). /*m*/ c(2);\n",
        "a.b($X).c($Y)",
    )
    .unwrap();
    assert_eq!(lines_of(&internal), vec![1]);
    // The junction veto holds at EVERY consumed call level (sg n0 both).
    let head_junction = match_pattern(
        Language::JavaScript,
        "a.b(1).c /*j*/ (2);\n",
        "a.b($X).c($Y)",
    )
    .unwrap();
    assert!(head_junction.is_empty());
    let mid_junction = match_pattern(
        Language::JavaScript,
        "a.b /*j*/ (1).c(2);\n",
        "a.b($X).c($Y)",
    )
    .unwrap();
    assert!(mid_junction.is_empty());
    // Arg-list trivia stays transparent (sg n1).
    let arg_trivia = match_pattern(
        Language::JavaScript,
        "a.b( /*i*/ 1).c(2);\n",
        "a.b($X).c($Y)",
    )
    .unwrap();
    assert_eq!(lines_of(&arg_trivia), vec![1]);
    // Clean chains and the clean 2-on-3 prefix containment hold.
    let clean = match_pattern(Language::JavaScript, "a.b(1).c(2);\n", "a.b($X).c($Y)").unwrap();
    assert_eq!(lines_of(&clean), vec![1]);
    let prefix =
        match_pattern(Language::JavaScript, "a.b(1).c(2).d(3);\n", "a.b($X).c($Y)").unwrap();
    assert_eq!(lines_of(&prefix), vec![1]);
    // kt/swift trivia chains REFUSE identically to sg (registered doctrines).
    let kt = match_pattern(Language::Kotlin, "a.b(1)/*m*/.c(2)\n", "a.b($X).c($Y)").unwrap();
    assert!(kt.is_empty());
    let sw = match_pattern(Language::Swift, "a.b(1)/*m*/.c(2)\n", "a.b($X).c($Y)").unwrap();
    assert!(sw.is_empty());
}

/// PASS 129 (128B-F1/F3, f129c): the semi meta-LINK admission pin (oracle
/// `$o->$A = $Y;` sg n1 A=$k — the admission was correct, only unpinned and
/// mis-documented) plus the missing `as`/`or` ingress pins completing the
/// keyword-operator token-set mutation surface.
#[test]
fn f129c_semi_link_and_ingress_token_pins() {
    let semi_link = match_pattern(
        Language::Php,
        "<?php\n$o = new C();\n$o->$k = 5;\n",
        "$o->$A = $Y;",
    )
    .unwrap();
    assert_eq!(lines_of(&semi_link), vec![3]);
    assert_eq!(f127_capture(&semi_link, "A"), Some("$k"));
    assert_eq!(f127_capture(&semi_link, "Y"), Some("5"));
    // The whole-meta semi spelling keeps its sg-agreed refusal.
    assert!(!native_pattern_answerable(Language::Php, "$X = $Y;"));
    // 128B-F3: a token-set-shrinking mutant must not survive ingress.
    assert!(!needs_ast_grep_fallback("$X as $Y"));
    assert!(!needs_ast_grep_fallback("$X or $Y"));
}

/// PASS 129 (128A-F6, f129d): sg's loop-head comment discipline is
/// PER-COMMENT-POSITION (oracle grids /tmp/phase129/cells/f6* + re-grid
/// /tmp/phase129/verify). Refused: comments leading the init declaration,
/// comments after `(` or in the after-`;` gap, the `)`→`{` junction, while
/// leading/junction comments, the go/rs junction comments, comments inside
/// the go clause after its first element, and rs `loop` root comments.
/// Answered: comments TRAILING a header element before `;`/`)` (including
/// the for-of `xs /* of */ )` position — sg binds V=v/X=xs), `for`-to-`(`
/// leading trivia, go/rs head-leading trivia, and everything comment-free
/// (the 127 unwalled cells hold byte-stable).
#[test]
fn f129d_loop_head_trivia_refuses_like_sg() {
    let js_for = "for (let $A = 0; $A < $B; $A++) { $C }";
    // Leading comment inside the init declaration — sg REFUSES.
    let init_lead = match_pattern(
        Language::JavaScript,
        "for (let /* c0 */ i = 0; i < 10; i++) { q(i); }\n",
        js_for,
    )
    .unwrap();
    assert!(init_lead.is_empty());
    // `)`→`{` junction comment — sg REFUSES.
    let junction = match_pattern(
        Language::JavaScript,
        "for (let i = 0; i < 10; i++) /* c7 */ { q(i); }\n",
        js_for,
    )
    .unwrap();
    assert!(junction.is_empty());
    // After-`;` root-zone comment — sg REFUSES.
    let after_semi = match_pattern(
        Language::JavaScript,
        "for (let i = 0; /* q */ i < 10; i++) { q(i); }\n",
        js_for,
    )
    .unwrap();
    assert!(after_semi.is_empty());
    // `(`→`let` zone comment — sg REFUSES.
    let paren_to_let = match_pattern(
        Language::JavaScript,
        "for ( /* p */ let i = 0; i < 10; i++) { q(i); }\n",
        js_for,
    )
    .unwrap();
    assert!(paren_to_let.is_empty());
    // Trailing comments INSIDE the header expressions stay transparent (the
    // 127 unwalled discipline): A/B/C bind sg-exact.
    let in_expr = match_pattern(
        Language::JavaScript,
        "for (let i = 0 /* s */; i < n /* t */; i++ /* u */) { q(i); }\n",
        "for (let $A = 0; $A < $B; $A++) { $C }",
    )
    .unwrap();
    assert_eq!(lines_of(&in_expr), vec![1]);
    assert_eq!(f127_capture(&in_expr, "A"), Some("i"));
    assert_eq!(f127_capture(&in_expr, "C"), Some("q(i);"));
    // `for`-to-`(` leading trivia ANSWERS (sg n1).
    let for_lead = match_pattern(
        Language::JavaScript,
        "for /* f */ (let i = 0; i < 10; i++) { q(i); }\n",
        js_for,
    )
    .unwrap();
    assert_eq!(lines_of(&for_lead), vec![1]);
    // js/ts while: leading and junction comments refuse; the trailing
    // before-`)` comment answers like sg (oracle while_trail n1, A=`n > 0`
    // clean).
    let while_lead = match_pattern(
        Language::JavaScript,
        "while /* w */ (n > 0) { bar(); }\n",
        "while ($A) { $B }",
    )
    .unwrap();
    assert!(while_lead.is_empty());
    let while_junction = match_pattern(
        Language::JavaScript,
        "while (n > 0) /* x */ { bar(); }\n",
        "while ($A) { $B }",
    )
    .unwrap();
    assert!(while_junction.is_empty());
    let while_trail = match_pattern(
        Language::JavaScript,
        "while (n > 0 /* t */) { bar(); }\n",
        "while ($A) { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&while_trail, "A"), Some("n > 0"));
    // do heads refuse the pre-body comment, answer the post-body one.
    let do_lead = match_pattern(
        Language::JavaScript,
        "do /* d */ { bar(); } while (n > 0);\n",
        "do { $B } while ($A)",
    )
    .unwrap();
    assert!(do_lead.is_empty());
    let do_post = match_pattern(
        Language::JavaScript,
        "do { bar(); } /* z */ while (n > 0);\n",
        "do { $B } while ($A)",
    )
    .unwrap();
    assert_eq!(f127_capture(&do_post, "A"), Some("n > 0"));
    // go: junction + post-first-element clause comments refuse; head-leading
    // trivia answers (the protected 127 cell).
    let go_junction = match_pattern(
        Language::Go,
        "package main\n\nfunc bar(i int) {}\n\nfunc f(n int) {\n\tfor n > 0 /* u */ {\n\t\tbar(n)\n\t}\n}\n",
        "for $A { $B }",
    )
    .unwrap();
    assert!(go_junction.is_empty());
    let go_lead = match_pattern(
        Language::Go,
        "package main\n\nfunc bar(i int) {}\n\nfunc f(n int) {\n\tfor /* h */ n > 0 {\n\t\tbar(n)\n\t}\n}\n",
        "for $A { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&go_lead, "A"), Some("n > 0"));
    // go 3-clause for: the clause wraps init/;cond/;update, so its comments
    // never surface on the for root — judged per-position inside the clause
    // (trailing before `;` answers; the after-`;` gap refuses; oracle
    // f6d_go_after_semi n0 / f6_go_for_semi_trivia n1).
    let go3_gap = match_pattern(
        Language::Go,
        "package main\n\nfunc bar(i int) {}\n\nfunc f(n int) {\n\tfor i := 0; /* g */ i < n; i++ {\n\t\tbar(i)\n\t}\n}\n",
        "for $A := 0; $A < $B; $A++ { $C }",
    )
    .unwrap();
    assert!(go3_gap.is_empty());
    let go3_trail = match_pattern(
        Language::Go,
        "package main\n\nfunc bar(i int) {}\n\nfunc f(n int) {\n\tfor i := 0 /* c0 */; i < n /* c1 */; i++ {\n\t\tbar(i)\n\t}\n}\n",
        "for $A := 0; $A < $B; $A++ { $C }",
    )
    .unwrap();
    assert_eq!(f127_capture(&go3_trail, "A"), Some("i"));
    assert_eq!(f127_capture(&go3_trail, "C"), Some("bar(i)\n"));
    // The head-leading 3-clause comment and the post-update junction refuse
    // (oracle /tmp/phase129/unknowns n0 each).
    let go3_lead = match_pattern(
        Language::Go,
        "package main\n\nfunc bar(i int) {}\n\nfunc f(n int) {\n\tfor /* h */ i := 0; i < n; i++ {\n\t\tbar(i)\n\t}\n}\n",
        "for $A := 0; $A < $B; $A++ { $C }",
    )
    .unwrap();
    assert!(go3_lead.is_empty());
    let go3_junction = match_pattern(
        Language::Go,
        "package main\n\nfunc bar(i int) {}\n\nfunc f(n int) {\n\tfor i := 0; i < n; i++ /* j */ {\n\t\tbar(i)\n\t}\n}\n",
        "for $A := 0; $A < $B; $A++ { $C }",
    )
    .unwrap();
    assert!(go3_junction.is_empty());
    // rs: junction comments refuse on all three root kinds; head-leading
    // trivia answers.
    let rs_for_junction = match_pattern(
        Language::Rust,
        "fn f(n: i32) {\n\tfor i in 0..n /* t */ {\n\t\tbar(i);\n\t}\n}\n",
        "for $A in $Y { $B }",
    )
    .unwrap();
    assert!(rs_for_junction.is_empty());
    let rs_for_lead = match_pattern(
        Language::Rust,
        "fn f(n: i32) {\n\tfor /* a */ i in 0..n {\n\t\tbar(i);\n\t}\n}\n",
        "for $A in $Y { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&rs_for_lead, "Y"), Some("0..n"));
    let rs_loop = match_pattern(
        Language::Rust,
        "fn f() {\n\tloop /* l */ {\n\t\ttick();\n\t}\n}\n",
        "loop { $B }",
    )
    .unwrap();
    assert!(rs_loop.is_empty());
    let rs_while = match_pattern(
        Language::Rust,
        "fn f(n: i32) {\n\twhile n > 0 /* y */ {\n\t\tbar();\n\t}\n}\n",
        "while $A { $B }",
    )
    .unwrap();
    assert!(rs_while.is_empty());
    // js for-of trailing comment before `)` ANSWERS like sg (129 re-grid
    // correction: the original "junction refuses" claim was wrong-of-record
    // — oracle binds V=v, X=xs, B=use(v);).
    let for_of_trail = match_pattern(
        Language::JavaScript,
        "for (const v of xs /* of */) {\n  use(v);\n}\n",
        "for (const $V of $X) { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&for_of_trail, "V"), Some("v"));
    assert_eq!(f127_capture(&for_of_trail, "X"), Some("xs"));
    assert_eq!(f127_capture(&for_of_trail, "B"), Some("use(v);"));
}

/// PASS 129 (128A-F2, f129e): rb modifier statements (`x if/unless/until/
/// while $C`) bind the condition like sg 0.45.2 (oracle grid
/// /tmp/phase129/cells/f123_rb_mod_*). Failure-first: all four faces were
/// census-loud rc2 before the eligibility + root-kind admissions.
#[test]
fn f129e_rb_modifier_statements_bind_like_sg() {
    let mif = match_pattern(Language::Ruby, "cond = true\nx if cond\n", "x if $C").unwrap();
    assert_eq!(lines_of(&mif), vec![2]);
    assert_eq!(f127_capture(&mif, "C"), Some("cond"));
    let munless = match_pattern(
        Language::Ruby,
        "ready = false\nx unless ready\n",
        "x unless $C",
    )
    .unwrap();
    assert_eq!(f127_capture(&munless, "C"), Some("ready"));
    let muntil =
        match_pattern(Language::Ruby, "done = false\nx until done\n", "x until $C").unwrap();
    assert_eq!(f127_capture(&muntil, "C"), Some("done"));
    let mwhile = match_pattern(
        Language::Ruby,
        "waiting = true\nx while waiting\n",
        "x while $C",
    )
    .unwrap();
    assert_eq!(f127_capture(&mwhile, "C"), Some("waiting"));
    // Ingress pin (the M-127f lesson): the language-free support gate must
    // admit the modifier shape via the rb union arm, else the CLI rc2s on
    // match-bearing files even though the library binds.
    assert!(!needs_ast_grep_fallback("x if $C"));
    // Literal-condition mismatch answers honest empty (text-exact body).
    assert!(
        match_pattern(Language::Ruby, "cond = true\nx if other\n", "x if cond")
            .unwrap()
            .is_empty()
    );
}

/// PASS 129 (128B-F2, f129f): rb `not $X` binds like sg (X=`flag`, oracle
/// f2_rb_not n1). Failure-first: census-loud rc2 before the rb head+unary
/// admissions. The rb `!$X` spelling binds like sg too (X=`flag` — the
/// §43.7c loud row is CORRECTED-OF-RECORD by the g_rb_bang oracle cell;
/// f127c carries the flip). `not` is OPERATOR-TOKEN-EXACT: it must not
/// match a `!`-spelled source (sg oracle g_rb_not_again: 0 hits).
#[test]
fn f129f_rb_not_unary_binds_like_sg() {
    let hit = match_pattern(
        Language::Ruby,
        "flag = true\nif not flag\n  q\nend\n",
        "not $X",
    )
    .unwrap();
    assert_eq!(lines_of(&hit), vec![2]);
    assert_eq!(f127_capture(&hit, "X"), Some("flag"));
    // py `not` twin unaffected (127's fix holds).
    let py = match_pattern(Language::Python, "if not flag:\n    q()\n", "not $X").unwrap();
    assert_eq!(f127_capture(&py, "X"), Some("flag"));
    // rb `!$X` binds like sg (oracle g_rb_bang n1 X=flag; the 127
    // census-loud premise is refuted, see f127c).
    let bang = match_pattern(Language::Ruby, "flag = true\nif !flag\n  q\nend\n", "!$X").unwrap();
    assert_eq!(f127_capture(&bang, "X"), Some("flag"));
    // Token-exactness control: pattern `not $X` refuses a `!`-spelled
    // source (sg answers empty — g_rb_not_again).
    assert!(match_pattern(
        Language::Ruby,
        "flag = true\nif !flag\n  q\nend\n",
        "not $X"
    )
    .unwrap()
    .is_empty());
}

/// PASS 129 (128A-F1, f129g): rb `BEGIN {}` / `END {}` block roots bind the
/// braced body like sg (B=`init` / B=`cleanup`, oracle f123_rb_*). The
/// multi-line `begin … end while` face stays census-loud (the general lane
/// refuses newline-carrying patterns — §43.7c's registered lane gap).
#[test]
fn f129g_rb_begin_end_blocks_bind_like_sg() {
    let begin = match_pattern(
        Language::Ruby,
        "BEGIN {\n  init\n}\n\nputs 1\n",
        "BEGIN { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&begin, "B"), Some("init"));
    let end = match_pattern(Language::Ruby, "END {\n  cleanup\n}\n", "END { $B }").unwrap();
    assert_eq!(f127_capture(&end, "B"), Some("cleanup"));
    // Ingress pin: the BEGIN/END union arm keeps the CLI off rc2 for the
    // braced block faces.
    assert!(!needs_ast_grep_fallback("BEGIN { $B }"));
    // The multi-line begin-while modifier face keeps its registered class.
    assert!(!native_pattern_answerable(
        Language::Ruby,
        "begin\n  $B\nend while $C"
    ));
}

// ---------------------------------------------------------------------------
// PASS 131 (r68 remediation) — RED-first pins. Every oracle receipt below was
// re-probed fresh against sg 0.45.2 on 2026-09-08 (cells under
// /tmp/phase131/live/); several pass-129 receipts they supersede are
// refuted of record in CNR §44.1.
// ---------------------------------------------------------------------------

/// f131a (130A-F1/F2 + 130B-F2/F3): the php static scoped lane admits every
/// sg-ANSWERED shape the r68 grids re-probed — namespaced heads (mid-`\` AND
/// leading-`\` FQN spellings; the pass-129 "`\Foo::$s` sg refuses" receipt is
/// REFUTED, oracle a_ns/a2_lead1), single-canonical-meta subscript indexes
/// binding the WHOLE candidate index text (oracle a_msub: K='$i'; a2_augsub
/// rides `+=`; a2_idxbin binds K='$i + 1'), and binary-operator faces
/// (oracle a_bin/a_bin2/a_bin3). Refused classes keep their agreement:
/// `C::$$s` doubled-dollar props (sg rc1 empty, a2_metaprop), `C::$S`
/// canonical-meta props (sg rc1 empty), and non-single-meta `$`-carrying
/// index shapes stay unadmitted (fail-closed, CNR §45 form-1).
#[test]
fn f131a_php_static_lane_binds_namespaced_meta_index_and_binary_faces() {
    // Namespaced head binds (sg n1 V=5, oracle a_ns).
    let ns = match_pattern(
        Language::Php,
        "<?php\nFoo\\Bar::$s = 5;\n",
        "Foo\\Bar::$s = $V",
    )
    .unwrap();
    assert_eq!(f127_capture(&ns, "V"), Some("5"));
    // Leading-`\` FQN binds (sg n1 V=5, oracle a2_lead1 — refuted-receipt
    // correction).
    let lead = match_pattern(
        Language::Php,
        "<?php\n\\Foo\\Bar::$s = 5;\n",
        "\\Foo\\Bar::$s = $V",
    )
    .unwrap();
    assert_eq!(f127_capture(&lead, "V"), Some("5"));
    // Single-canonical-meta subscript binds K to the whole candidate index
    // text (sg n1 K='$i' V=7, oracle a_msub).
    let msub = match_pattern(Language::Php, "<?php\nC::$s[$i] = 7;\n", "C::$s[$K] = $V").unwrap();
    assert_eq!(f127_capture(&msub, "K"), Some("$i"));
    assert_eq!(f127_capture(&msub, "V"), Some("7"));
    // The meta subscript rides a compound op (sg n1, oracle a2_augsub).
    let aug = match_pattern(Language::Php, "<?php\nC::$s[$i] += 7;\n", "C::$s[$K] += $V").unwrap();
    assert_eq!(f127_capture(&aug, "K"), Some("$i"));
    // Binary faces bind (sg n1, oracle a_bin / a_bin2 / a_bin3).
    let eq = match_pattern(Language::Php, "<?php\nC::$s == 5;\n", "C::$s == $V").unwrap();
    assert_eq!(f127_capture(&eq, "V"), Some("5"));
    let same = match_pattern(Language::Php, "<?php\nC::$s === 5;\n", "C::$s === $V").unwrap();
    assert_eq!(f127_capture(&same, "V"), Some("5"));
    let concat = match_pattern(Language::Php, "<?php\nC::$s . 'x';\n", "C::$s . $V").unwrap();
    assert_eq!(f127_capture(&concat, "V"), Some("'x'"));
    // Wrong-class control keeps its honest empty (byte-exact LHS compare).
    assert!(match_pattern(
        Language::Php,
        "<?php\nclass C { public static $s = 0; }\nD::$s = 6;\n",
        "C::$s = $V"
    )
    .unwrap()
    .is_empty());
    // sg-REFUSED classes keep their agreement: doubled-dollar and canonical-
    // meta property slots (oracle a2_metaprop / a2_metaprop2).
    assert!(match_pattern(
        Language::Php,
        "<?php\nclass C { public $name = 0; }\nC::$name = 5;\n",
        "C::$$s = $V"
    )
    .unwrap()
    .is_empty());
    // Lowercase php-variable index spelling is a LITERAL index: the
    // text-equal candidate binds V only, no K capture (oracle a2_lowsub).
    let lit_idx =
        match_pattern(Language::Php, "<?php\nC::$s[$k] = 7;\n", "C::$s[$k] = $V").unwrap();
    assert_eq!(f127_capture(&lit_idx, "V"), Some("7"));
    assert_eq!(f127_capture(&lit_idx, "K"), None);
}

/// f131b (130A-F3): sg's exact-children junction doctrine extends to the
/// after-`>` gap of type-args-spelled calls — a trivia child fully inside the
/// typeargs→arguments gap refuses the candidate (sg rc1 empty, oracle bjunc)
/// while a whitespace-only gap stays admitted (oracle bws, sg n1).
#[test]
fn f131b_ts_typeargs_junction_trivia_refuses_like_sg() {
    let refused = match_pattern(Language::TypeScript, "g<T> /* c */ (1);\n", "g<T>($A)").unwrap();
    assert!(refused.is_empty(), "{:?}", refused);
    let refused_js =
        match_pattern(Language::JavaScript, "g<T> /* c */ (1);\n", "g<T>($A)").unwrap();
    assert!(refused_js.is_empty(), "{:?}", refused_js);
    // Whitespace-only gap stays sg-ANSWERED (oracle bws).
    let ws = match_pattern(Language::TypeScript, "g<T> (1);\n", "g<T>($A)").unwrap();
    assert_eq!(lines_of(&ws), vec![1]);
    assert_eq!(f127_capture(&ws, "A"), Some("1"));
}

/// f131d (130A-F5): the bare `debugger` statement head joins the kind
/// template table — sg answers the js/ts debugger_statement for both the
/// `;`-ful and bare spellings (oracle dbg_js / dbg_ts).
#[test]
fn f131d_bare_debugger_answers_debugger_statement() {
    let bare = match_pattern(
        Language::JavaScript,
        "function f() { debugger; }\n",
        "debugger",
    )
    .unwrap();
    assert_eq!(lines_of(&bare), vec![1]);
    let semi = match_pattern(
        Language::JavaScript,
        "function f() { debugger; }\n",
        "debugger;",
    )
    .unwrap();
    assert_eq!(lines_of(&semi), vec![1]);
    let ts = match_pattern(
        Language::TypeScript,
        "function f() { debugger; }\n",
        "debugger",
    )
    .unwrap();
    assert_eq!(lines_of(&ts), vec![1]);
}

/// f131e (130A-F6): concrete-condition if templates ride the if lane when
/// the condition is a general-lane admissible expression; cond metas bind
/// and the body capture keeps its registered text. Receipts: d2_js_cond_call
/// (X=1), d2_js_cond_bin (X=b), d2_ts_concrete, d2_js_two_stmt (sg REFUSES a
/// two-statement Exactly(1) body), d2_ts_concrete_meta (meta-cond control),
/// d2_py_concrete, d2_rs_concrete.
#[test]
fn f131e_concrete_cond_if_binds_like_sg() {
    let call = match_pattern(
        Language::JavaScript,
        "if (f(1)) { b(); }\n",
        "if (f($X)) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&call), vec![1]);
    assert_eq!(f127_capture(&call, "X"), Some("1"));
    assert_eq!(f127_capture(&call, "B"), Some("b();"));
    let bin = match_pattern(
        Language::JavaScript,
        "if (a && b) { c(); }\n",
        "if (a && $X) { $B }",
    )
    .unwrap();
    assert_eq!(f127_capture(&bin, "X"), Some("b"));
    let conc = match_pattern(Language::TypeScript, "if (a) { b(); }\n", "if (a) { $B }").unwrap();
    assert_eq!(lines_of(&conc), vec![1]);
    assert_eq!(f127_capture(&conc, "B"), Some("b();"));
    // Statement-count discipline carries over (sg refuses the 2-stmt body).
    assert!(match_pattern(
        Language::JavaScript,
        "if (a) { b(); c(); }\n",
        "if (a) { $B }"
    )
    .unwrap()
    .is_empty());
    // Meta-cond control keeps its registered binding byte-stable.
    let meta = match_pattern(Language::TypeScript, "if (a) { b(); }\n", "if ($X) { $B }").unwrap();
    assert_eq!(f127_capture(&meta, "X"), Some("a"));
    // Paren-free grammars bind through their own condition spellings
    // (sg n1: one hit rooted at the if line, oracle d2_py_concrete).
    let py = match_pattern(Language::Python, "if c:\n    w()\n", "if c:\n    $B").unwrap();
    assert_eq!(lines_of(&py), vec![1]);
    assert_eq!(f127_capture(&py, "B"), Some("w()"));
    let rs = match_pattern(Language::Rust, "fn m() { if a { w(); } }\n", "if a { $B }").unwrap();
    assert_eq!(f127_capture(&rs, "B"), Some("w();"));
}

/// f131f (130A-F8): `interface` declaration templates join the Class lane
/// with the member-count body discipline on ts/java (sg oracle d2_iface
/// cells: 1-member binds N and B, 2-member refuses). Non-receipted grammars
/// keep the fail-closed loud class (CNR §45).
#[test]
fn f131f_interface_member_count_body_binds_like_sg() {
    let ts = match_pattern(
        Language::TypeScript,
        "interface I { a: string; }\n",
        "interface $N { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&ts), vec![1]);
    assert_eq!(f127_capture(&ts, "N"), Some("I"));
    assert_eq!(f127_capture(&ts, "B"), Some("a: string;"));
    // Two members refuse like sg (d2_iface_type_member sg rc1 empty).
    assert!(match_pattern(
        Language::TypeScript,
        "interface I { a: string; b: number; }\n",
        "interface $N { $B }"
    )
    .unwrap()
    .is_empty());
    // java receipt (d4 grid): sg n1 B='void m();'.
    let java = match_pattern(
        Language::Java,
        "interface I { void m(); }\n",
        "interface $N { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&java), vec![1]);
    assert_eq!(f127_capture(&java, "N"), Some("I"));
    assert_eq!(f127_capture(&java, "B"), Some("void m();"));
}

// ---------------------------------------------------------------------------
// PASS 135 (r69 remediation: 134A/134B findings) — RED-first pins. Every
// expected set below is a live probe of the pinned oracle (ast-grep 0.45.2,
// probed 2026-09-12, grids /tmp/phase135/cells/*.jsonl).
// ---------------------------------------------------------------------------

/// f135a (134A-F2 + 134A-F3): sg's empty-operand discipline on the `;`-ful
/// return spelling — pattern `return;` answers ONLY the operand-less
/// return_statement (grid B_js_return; sg n1 vs subject n3; B_ts_return;
/// sg n1 vs subject n2), while the bare spelling keeps answering the whole
/// family and rs keeps its all-returns `;`-ful answer (grid B_rs_return;
/// sg n2). python joins the scoped return arms: sg answers the py bare
/// `return` family n2 (grid B_py_return: sg n2 vs subject n1 — the 133
/// grid omitted the py row; the "py return invalid" premise is REFUTED).
/// java/c/cpp/csharp keep their agreeing general-lane faces (regression
/// guard; grid B_java_return; sg n1 == subject n1).
#[test]
fn f135a_return_semi_empty_operand_and_py_return_answer_sg_aligned() {
    let js = "function f() {\n  return;\n}\nfunction g() {\n  return 1;\n}\n";
    let semi = match_pattern(Language::JavaScript, js, "return;").unwrap();
    assert_eq!(
        lines_of(&semi),
        vec![2],
        "sg `return;` answers the operand-less return only, got {semi:?}"
    );
    // Bare keeps the family answer (133 receipt) and `return $X` the
    // operand-ful-only answer (grid B_js_return / B_js_return $X).
    let bare = match_pattern(Language::JavaScript, js, "return").unwrap();
    assert_eq!(lines_of(&bare), vec![2, 5]);
    let meta = match_pattern(Language::JavaScript, js, "return $X").unwrap();
    assert_eq!(lines_of(&meta), vec![5]);
    // The trivia-carrying empty return stays operand-less (grid
    // Z_js_return_semi trivia cell: sg n2 answers both empty spellings).
    let trivia = match_pattern(
        Language::JavaScript,
        "function f() {\n  return;\n}\nfunction g() {\n  return /* c */;\n}\n",
        "return;",
    )
    .unwrap();
    assert_eq!(lines_of(&trivia), vec![2, 5]);
    let ts_semi = match_pattern(
        Language::TypeScript,
        "function f() {\n  return;\n}\nfunction g() {\n  return 1;\n}\n",
        "return;",
    )
    .unwrap();
    assert_eq!(lines_of(&ts_semi), vec![2]);
    // rs `return;` answers BOTH returns like sg (return_expression arm; the
    // `;` is not an operand marker in rust — oracle re-probe 2026-09-10:
    // sg rows line0 {1,4} = the operand-less AND the operand-ful spelling,
    // `return 1;` included; the first draft's [2,4] mis-converted sg's
    // 0-based lines — corrected of record).
    let rs_semi = match_pattern(
        Language::Rust,
        "fn f() {\n    return;\n}\nfn g() -> i32 {\n    return 1;\n}\n",
        "return;",
    )
    .unwrap();
    assert_eq!(lines_of(&rs_semi), vec![2, 5]);
    // py bare `return` answers the whole family like sg (return_statement
    // arm; oracle rows line0 {1,4} = both spellings); `return $X` stays
    // operand-ful-only (sg line0=4).
    let py = "def f():\n    return\n\ndef g():\n    return 1\n";
    let py_bare = match_pattern(Language::Python, py, "return").unwrap();
    assert_eq!(lines_of(&py_bare), vec![2, 5]);
    let py_meta = match_pattern(Language::Python, py, "return $X").unwrap();
    assert_eq!(lines_of(&py_meta), vec![5]);
    // java general-lane face unchanged (grid B_java_return; sg n1 == n1).
    let java_semi = match_pattern(
        Language::Java,
        "class K {\n  void f() {\n    return;\n  }\n}\n",
        "return;",
    )
    .unwrap();
    assert_eq!(lines_of(&java_semi), vec![3]);
}

/// f135b (134A-F4 + the §45.6 residual predicate firing): sg REFUSES
/// `interface $N { $B }` on extends-carrying ts/java interfaces, on
/// modifier-carrying java/csharp interfaces, and when trivia sits in the
/// name→body gap (grids D_ts_ext1/ext2, D_ts_triv_after_name,
/// D_ts_triv_before_brace, D_java_ext1/extlist, D_java_public, D_java_triv,
/// X_cs_iface_ext, X_cs_iface_pub, Y_cs_iface_triv — all sg rc1 n0).
/// Trivia BEFORE the name keeps answering (D_ts_triv_before_name sg n1) and
/// csharp joins the receipted member-count grammars (F_cs_iface1 sg n1
/// N/B binds; F_cs_iface2 sg rc1).
#[test]
fn f135b_interface_candidates_refuse_extends_modifiers_and_gap_trivia() {
    let iface = "interface $N { $B }";
    // ts extends single + list → sg rc1 empty.
    for src in [
        "interface J extends K {\n  a: string;\n}\n",
        "interface L extends M, N {\n  b: number;\n}\n",
    ] {
        assert!(
            match_pattern(Language::TypeScript, src, iface)
                .unwrap()
                .is_empty(),
            "ts extends-carrying interface must refuse like sg: {src}"
        );
    }
    // ts trivia AFTER the name / immediately before the brace → refuse.
    for src in [
        "interface I /* c */ {\n  a: string;\n}\n",
        "interface I\n /* c */ {\n  a: string;\n}\n",
    ] {
        assert!(
            match_pattern(Language::TypeScript, src, iface)
                .unwrap()
                .is_empty(),
            "ts gap-trivia interface must refuse like sg: {src}"
        );
    }
    // Trivia BEFORE the name keeps answering (D_ts_triv_before_name sg n1
    // at 0-based line 0 = line 1; the first draft's [2] mis-converted).
    let before = match_pattern(
        Language::TypeScript,
        "/* c */ interface I {\n  a: string;\n}\n",
        iface,
    )
    .unwrap();
    assert_eq!(lines_of(&before), vec![1]);
    assert_eq!(f127_capture(&before, "N"), Some("I"));
    // java extends + modifier + trivia → refuse; plain keeps binding (f131f).
    for src in [
        "interface I extends J {\n  void m();\n}\n",
        "interface I extends J, K {\n  void m();\n}\n",
        "public interface I {\n  void m();\n}\n",
        "interface I /* c */ {\n  void m();\n}\n",
    ] {
        assert!(
            match_pattern(Language::Java, src, iface)
                .unwrap()
                .is_empty(),
            "java extended/modified/trivia interface must refuse like sg: {src}"
        );
    }
    // csharp joins the receipted grammars: 1-member binds, everything the
    // ts/java doctrine refuses refuses.
    let cs = match_pattern(Language::CSharp, "interface I {\n  void M();\n}\n", iface).unwrap();
    assert_eq!(lines_of(&cs), vec![1]);
    assert_eq!(f127_capture(&cs, "N"), Some("I"));
    assert_eq!(f127_capture(&cs, "B"), Some("void M();"));
    for src in [
        "interface I {\n  void M();\n  void N();\n}\n",
        "interface I : J {\n  void M();\n}\n",
        "public interface I {\n  void M();\n}\n",
        "interface I /* c */ {\n  void M();\n}\n",
    ] {
        assert!(
            match_pattern(Language::CSharp, src, iface)
                .unwrap()
                .is_empty(),
            "csharp 2-member/extends/modified/trivia interface must refuse like sg: {src}"
        );
    }
}

/// f135c (134A-F1): csharp `lock`/`using` statement roots answer
/// layout-insensitively like sg (grids A9/A10: sg n1 on the multi-line
/// body, subject n0) and the meta shapes sg binds bind here: the meta
/// resource `using ($R) { g(); }` (A13, R = the whole resource text), the
/// meta lock condition `lock ($X) { x(); }` (grid Y: X='o'), and the using
/// declaration `using $T $N = $E;` (A8: T/N/E bind). Meta-BODY faces stay
/// refused (sg ERROR-node empty: grids A2/A3/A5/A6/Y_cs_using_meta_body —
/// subject keeps its registered loud class at the ingress, empty here).
#[test]
fn f135c_csharp_lock_using_statement_roots_answer_sg_aligned() {
    let lock_src = "class C {\n  void M() {\n    lock (o)\n    {\n      x();\n    }\n  }\n}\n";
    let lock = match_pattern(Language::CSharp, lock_src, "lock (o) { x(); }").unwrap();
    assert_eq!(
        lines_of(&lock),
        vec![3],
        "sg binds the lock_statement layout-insensitively"
    );
    let using_src =
        "class C {\n  void M() {\n    using (var d = f())\n    {\n      g();\n    }\n  }\n}\n";
    let using = match_pattern(Language::CSharp, using_src, "using (var d = f()) { g(); }").unwrap();
    assert_eq!(lines_of(&using), vec![3]);
    // Meta resource binds the whole resource text (sg R='var d = f()').
    let rmeta = match_pattern(
        Language::CSharp,
        "class C {\n  void M() {\n    using (var d = f()) { g(); }\n  }\n}\n",
        "using ($R) { g(); }",
    )
    .unwrap();
    assert_eq!(lines_of(&rmeta), vec![3]);
    assert_eq!(f127_capture(&rmeta, "R"), Some("var d = f()"));
    // Meta lock condition binds (grid Y_cs_lock_meta_cond_conc_body: X='o').
    let xmeta = match_pattern(
        Language::CSharp,
        "class C {\n  void M() {\n    lock (o) { x(); }\n  }\n}\n",
        "lock ($X) { x(); }",
    )
    .unwrap();
    assert_eq!(lines_of(&xmeta), vec![3]);
    assert_eq!(f127_capture(&xmeta, "X"), Some("o"));
    // Using declaration: type/name/init bind (grid A8; `var` is the literal
    // type — grid Y_cs_var_decl answers `var $N = $E;` n1 only).
    let decl = match_pattern(
        Language::CSharp,
        "class C {\n  void M() {\n    using var x = y();\n  }\n}\n",
        "using $T $N = $E;",
    )
    .unwrap();
    assert_eq!(lines_of(&decl), vec![3]);
    assert_eq!(f127_capture(&decl, "T"), Some("var"));
    assert_eq!(f127_capture(&decl, "N"), Some("x"));
    assert_eq!(f127_capture(&decl, "E"), Some("y()"));
    let var_decl = match_pattern(
        Language::CSharp,
        "class C {\n  void M() {\n    var x = f();\n    int y = 2;\n  }\n}\n",
        "var $N = $E;",
    )
    .unwrap();
    assert_eq!(lines_of(&var_decl), vec![3]);
    // Meta-BODY faces refuse like sg's ERROR-empty (A2/A3/Y_cs_using_meta_body).
    for pat in [
        "lock ($X) { $B }",
        "lock ($X) { $$$B }",
        "using ($R) { $B }",
        "using ($R) { $$$B }",
    ] {
        assert!(
            match_pattern(
                Language::CSharp,
                "class C {\n  void M() {\n    using (var d = f()) { g(); }\n    lock (o) { x(); y(); }\n  }\n}\n",
                pat,
            )
            .unwrap()
            .is_empty(),
            "meta-body lock/using template must refuse like sg: {pat}"
        );
    }
}

/// f135d (134A-F5/F6 + 134B-F4): php static-lane meta edges. The meta
/// member name on a static-prop chain binds (grid E_php_arrow_meta sg n1
/// M='m'; self head too); the dynamic-class head `$$C`/`$C` binds the whole
/// candidate head text WITH the dollar (E_php_dyn_class_lit C='$c';
/// E_php_dyn_prop C='$name'); the whitespace-padded candidate LHS binds a
/// tight pattern (E_php_padded2 sg n1 vs subject n0). A dynamic-head
/// pattern does not answer subscript candidates (Y_php_dynsub sg rc1).
#[test]
fn f135d_php_static_meta_edges_answer_sg_aligned() {
    let arrow = match_pattern(
        Language::Php,
        "<?php\nclass C { public static $s; }\nC::$s->m();\nC::$s->n();\n",
        "C::$s->$M();",
    )
    .unwrap();
    assert_eq!(
        lines_of(&arrow),
        vec![3, 4],
        "sg binds the meta member name per site"
    );
    assert_eq!(f127_capture(&arrow, "M"), Some("m"));
    let self_arrow = match_pattern(
        Language::Php,
        "<?php\nclass C { public static $s; function f() { self::$s->m(); } }\n",
        "self::$s->$M();",
    )
    .unwrap();
    assert_eq!(lines_of(&self_arrow), vec![2]);
    assert_eq!(f127_capture(&self_arrow, "M"), Some("m"));
    let dyn_lit = match_pattern(
        Language::Php,
        "<?php\n$c = 'C';\n$c::$s = 5;\n",
        "$$C::$s = $V",
    )
    .unwrap();
    assert_eq!(lines_of(&dyn_lit), vec![3]);
    assert_eq!(f127_capture(&dyn_lit, "C"), Some("$c"));
    assert_eq!(f127_capture(&dyn_lit, "V"), Some("5"));
    let dyn_prop = match_pattern(
        Language::Php,
        "<?php\n$name = 'C';\n$name::$s = 5;\n",
        "$C::$s = $V",
    )
    .unwrap();
    assert_eq!(lines_of(&dyn_prop), vec![3]);
    assert_eq!(f127_capture(&dyn_prop, "C"), Some("$name"));
    // Dynamic-head pattern vs subscript candidate: sg rc1 (Y_php_dynsub).
    assert!(
        match_pattern(Language::Php, "<?php\n$c::$s[0] = 5;\n", "$$C::$s = $V")
            .unwrap()
            .is_empty()
    );
    // Padded candidate LHS binds the tight pattern (E_php_padded2).
    let padded = match_pattern(Language::Php, "<?php\nC :: $s = 5;\n", "C::$s = $V").unwrap();
    assert_eq!(lines_of(&padded), vec![2]);
    assert_eq!(f127_capture(&padded, "V"), Some("5"));
    // Tight control keeps binding (f131a receipt re-pin).
    let tight = match_pattern(Language::Php, "<?php\nC::$s = 5;\n", "C::$s = $V").unwrap();
    assert_eq!(lines_of(&tight), vec![2]);
}

/// f135f (134B-F4, oracle grid /tmp/phase135/iso2/f.php 2026-09-11): the php
/// static-scope family must answer sg's STRUCTURAL rows, not the literal
/// lane's byte-identical rows. (a) A concrete-scope pattern whose `$`-tokens
/// are all lowercase-literal (`C::$s = 5`, `C::$s = f(5)`, `C::$s == 5`)
/// rides dollar_literal_lane, whose exact-text arm misses the padded
/// candidate sg answers (rows 4/7). (b) A dynamic-head pattern
/// (`$C::$s = $V` / `$$C::$s = $V`, `=`-only) binds the WHOLE candidate
/// scope text on concrete heads too (`C` row 2/4, `Foo\Bar` row 3), not
/// only variable heads (row 5) — and the prop stays literal-exact (row 6
/// `$name` never answers `$s` patterns; `$C::$name = $V` answers row 6).
#[test]
fn f135f_php_static_scope_rows_answer_sg_aligned() {
    let src = "<?php\nC::$s = f(5);\nFoo\\Bar::$s = 5;\nC :: $s = 5;\n$c::$s = 9;\nC::$name = 5;\nC :: $s == 5;\nC::$s == 5;\n";
    let pin = |pat: &str, want: Vec<u32>| {
        let got = lines_of(&match_pattern(Language::Php, src, pat).unwrap());
        assert_eq!(got, want, "sg rows for {pat:?} on the f.php fixture");
    };
    pin("C::$s = f(5)", vec![2]);
    pin("C::$s = 5", vec![4]);
    pin("C::$s == 5", vec![7, 8]);
    pin("C::$s = $V", vec![2, 4]);
    pin("$C::$s = $V", vec![2, 3, 4, 5]);
    pin("$$C::$s = $V", vec![2, 3, 4, 5]);
    pin("$C::$name = $V", vec![6]);
    // Capture disciplines on the dynamic-head family (sg metaVariables):
    // the scope binds `C` (rows 2/4), `Foo\Bar` (row 3), and `$c` (row 5).
    let dyn_all = match_pattern(Language::Php, src, "$$C::$s = $V").unwrap();
    let scopes: Vec<_> = dyn_all
        .iter()
        .filter_map(|hit| hit.captures.get("C").map(String::as_str))
        .collect();
    assert!(
        scopes.contains(&"C") && scopes.contains(&"Foo\\Bar") && scopes.contains(&"$c"),
        "sg binds every candidate scope text: {scopes:?}"
    );
}

/// f135e (134A-F7 FIX subset): the loud cells whose sg admission is cheap
/// and structural — ts type aliases (`type $N = { $B }` one-member binds
/// N/B, two-member refuses — F_ts_alias_*; `type $N = $V` binds V across
/// union/fn/literal spellings), the keyword-operator unary faces
/// (`delete $X`/`void $X` js, `del $X` py — X binds, grids F_js_delete/
/// F_js_void/F_py_del/X_py_del_two), js `with ($X) { $B }` (X/B bind) and
/// the labeled-statement faces (meta label binds L/X, mismatched label
/// refuses, concrete label binds — F_js_labeled/X_js_label_mismatch/
/// X_js_label_concrete/Y_java_labeled).
#[test]
fn f135e_ts_type_alias_and_keyword_operator_faces_answer_sg_aligned() {
    let alias = "type $N = { $B }";
    let one = match_pattern(Language::TypeScript, "type T = { a: string; };\n", alias).unwrap();
    assert_eq!(lines_of(&one), vec![1]);
    assert_eq!(f127_capture(&one, "N"), Some("T"));
    assert_eq!(f127_capture(&one, "B"), Some("a: string;"));
    let multi = match_pattern(
        Language::TypeScript,
        "type T = {\n  a: string;\n};\n",
        alias,
    )
    .unwrap();
    assert_eq!(lines_of(&multi), vec![1]);
    assert!(match_pattern(
        Language::TypeScript,
        "type T = { a: string; b: number; };\n",
        alias
    )
    .unwrap()
    .is_empty());
    let iface_mix = match_pattern(
        Language::TypeScript,
        "interface Q {\n  a: string;\n}\ntype T = { a: string; };\n",
        alias,
    )
    .unwrap();
    assert_eq!(lines_of(&iface_mix), vec![4]);
    let union = match_pattern(
        Language::TypeScript,
        "type T = string | number;\n",
        "type $N = $V",
    )
    .unwrap();
    assert_eq!(lines_of(&union), vec![1]);
    assert_eq!(f127_capture(&union, "N"), Some("T"));
    assert_eq!(f127_capture(&union, "V"), Some("string | number"));
    let fnval = match_pattern(
        Language::TypeScript,
        "type F = (x: number) => string;\n",
        "type $N = $V",
    )
    .unwrap();
    assert_eq!(f127_capture(&fnval, "V"), Some("(x: number) => string"));
    let concrete = match_pattern(
        Language::TypeScript,
        "type T = string | number;\n",
        "type T = $V",
    )
    .unwrap();
    assert_eq!(f127_capture(&concrete, "V"), Some("string | number"));
    // Keyword-operator unary faces.
    let del = match_pattern(
        Language::JavaScript,
        "const o = {};\ndelete o.k;\ndelete o['j'];\n",
        "delete $X",
    )
    .unwrap();
    assert_eq!(lines_of(&del), vec![2, 3]);
    assert_eq!(f127_capture(&del, "X"), Some("o.k"));
    let voi = match_pattern(Language::JavaScript, "void f();\nvoid 0;\n", "void $X").unwrap();
    assert_eq!(lines_of(&voi), vec![1, 2]);
    let pydel = match_pattern(Language::Python, "x = 1\ny = 2\ndel x, y\n", "del $X").unwrap();
    assert_eq!(lines_of(&pydel), vec![3]);
    assert_eq!(f127_capture(&pydel, "X"), Some("x, y"));
    let with = match_pattern(
        Language::JavaScript,
        "with (obj) {\n  a();\n}\n",
        "with ($X) { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&with), vec![1]);
    assert_eq!(f127_capture(&with, "X"), Some("obj"));
    assert_eq!(f127_capture(&with, "B"), Some("a();"));
    // Labeled statements: meta label binds L and X; mismatch refuses;
    // concrete label binds.
    let labeled = match_pattern(
        Language::JavaScript,
        "outer: while (c) {\n  break outer;\n}\n",
        "$L: while ($X) { break $L; }",
    )
    .unwrap();
    assert_eq!(lines_of(&labeled), vec![1]);
    assert_eq!(f127_capture(&labeled, "L"), Some("outer"));
    assert_eq!(f127_capture(&labeled, "X"), Some("c"));
    assert!(match_pattern(
        Language::JavaScript,
        "outer: while (c) {\n  break inner;\n}\n",
        "$L: while ($X) { break $L; }"
    )
    .unwrap()
    .is_empty());
    let concrete_label = match_pattern(
        Language::JavaScript,
        "outer: while (c) {\n  break outer;\n}\n",
        "outer: while ($X) { break outer; }",
    )
    .unwrap();
    assert_eq!(f127_capture(&concrete_label, "X"), Some("c"));
    let java_labeled = match_pattern(
        Language::Java,
        "class A {\n  void m() {\n    outer: while (c) {\n      break outer;\n    }\n  }\n}\n",
        "$L: while ($X) { break $L; }",
    )
    .unwrap();
    assert_eq!(lines_of(&java_labeled), vec![3]);
}

/// f136a (F-135E-1): the modifier-carrying interface ROOT pattern must bind
/// the modifier-matching candidate sg-exactly. sg 0.45.2 (135E grids,
/// /tmp/phase136/grid136.json + sgprobe; semantics CORRECTED PASS 137,
/// 137A-F5 grid137a D1 — the java rule is an ORDER-PRESERVING SUBSEQUENCE,
/// not the leading-token prefix this doc first recorded): `public interface
/// $N { $B }` answers `public interface K` n1 (N/B) on csharp AND java; the
/// F4 candidate refusal is SEQUENCE-scoped, not blanket —
///   * extends/base heritage, gap trivia, and the 2-member count refuse
///     regardless of modifiers (sg rc1 probes);
///   * java: the pattern modifier sequence must appear IN ORDER as a
///     subsequence of the candidate's modifier tokens (`public` answers
///     `public abstract`/`public static`; reordered `abstract public`
///     against `public abstract` refuses — subsequence order, not set
///     equality, not prefix). Annotations are NOT subsequence members:
///     they are POSITIONAL and never skippable — an annotation must sit
///     exactly at the scan cursor and blocks keyword passage (`public`
///     refuses `@Deprecated public`; `@Deprecated` refuses
///     `public @Deprecated`) — the PASS 137 correction, f137d pins the
///     annotation cells.
///   * csharp: the leaf modifier sequence must byte-equal EXACTLY (`public`
///     refuses `public sealed`; `public sealed` answers `public sealed`);
///   * mismatched or missing candidate modifiers refuse in both grammars;
///   * the PLAIN pattern keeps the blanket refusal (f135b) and leading
///     trivia before the modifier keeps answering (sg n1).
///
/// RED 2026-09-11: every bind cell answered silent-empty (the F4 fix's
/// `interface_candidate_refused` refused ANY modifier-carrying candidate
/// regardless of the pattern side).
#[test]
fn f136a_modifier_carrying_interface_roots_bind_sg_exact() {
    let pat = "public interface $N { $B }";
    // Modifier-matching candidates bind (sg n1, N/B).
    let cs = match_pattern(
        Language::CSharp,
        "public interface K {\n    void P();\n}\n",
        pat,
    )
    .unwrap();
    assert_eq!(
        lines_of(&cs),
        vec![1],
        "sg binds the modifier-matching csharp candidate: {cs:?}"
    );
    assert_eq!(f127_capture(&cs, "N"), Some("K"));
    assert_eq!(f127_capture(&cs, "B"), Some("void P();"));
    let ja = match_pattern(
        Language::Java,
        "public interface B {\n    void N();\n}\n",
        pat,
    )
    .unwrap();
    assert_eq!(lines_of(&ja), vec![1], "sg binds the java twin: {ja:?}");
    let ja_nested = match_pattern(
        Language::Java,
        "public class K { public interface Inner { void M(); } }\n",
        pat,
    )
    .unwrap();
    assert_eq!(
        lines_of(&ja_nested),
        vec![1],
        "135E: the nested public interface binds: {ja_nested:?}"
    );
    assert_eq!(f127_capture(&ja_nested, "N"), Some("Inner"));
    // java subsequence rule (CORRECTED PASS 137, 137A-F5): the modifiers
    // wrapper absorbs extra candidate tokens IN ORDER.
    let ja_abs = match_pattern(
        Language::Java,
        "public abstract interface K {\n    void M();\n}\n",
        pat,
    )
    .unwrap();
    assert_eq!(
        lines_of(&ja_abs),
        vec![1],
        "sg binds `public` against `public abstract`"
    );
    // java sequence ORDER matters, not set equality.
    assert!(
        match_pattern(
            Language::Java,
            "abstract public interface K {\n    void M();\n}\n",
            "public abstract interface $N { $B }",
        )
        .unwrap()
        .is_empty(),
        "sg refuses the reordered modifier sequence"
    );
    // csharp exact-sequence rule.
    assert!(
        match_pattern(
            Language::CSharp,
            "public sealed interface K {\n    void P();\n}\n",
            pat
        )
        .unwrap()
        .is_empty(),
        "sg refuses `public` against the `public sealed` leaf sequence"
    );
    // REGISTERED f136 residual (receipt, not an admission): sg 0.45.2 binds
    // `public sealed interface $N { $B }` on `public sealed interface K`
    // n1 (probe /tmp/phase136/sgprobe c2), but the two-modifier PATTERN
    // spelling cannot reach the member-count lane — `sealed` is not in
    // strip_declaration_modifiers' pattern-side modifier list, so the
    // pattern never classifies Class{interface}. Extending the global
    // strip list would admit unprobed java/ts `sealed` faces; form-1: join
    // `sealed` with a java/csharp face grid. The refusal-direction
    // semantics above are the probed, shipped half.
    // Mismatched / missing candidate modifiers refuse in both grammars.
    for src in [
        "internal interface K {\n    void P();\n}\n",
        "interface K {\n    void P();\n}\n",
    ] {
        assert!(
            match_pattern(Language::CSharp, src, pat)
                .unwrap()
                .is_empty(),
            "sg refuses the modifier-mismatched csharp candidate: {src}"
        );
    }
    for src in [
        "private interface K {\n    void M();\n}\n",
        "interface K {\n    void M();\n}\n",
    ] {
        assert!(
            match_pattern(Language::Java, src, pat).unwrap().is_empty(),
            "sg refuses the modifier-mismatched java candidate: {src}"
        );
    }
    // Refusals INDEPENDENT of modifiers: heritage, gap trivia, 2 members.
    for (lang, src) in [
        (
            Language::CSharp,
            "public interface K : J {\n    void P();\n}\n",
        ),
        (
            Language::CSharp,
            "public interface K /* c */ {\n    void P();\n}\n",
        ),
        (
            Language::CSharp,
            "public interface K {\n    void P();\n    void Q();\n}\n",
        ),
        (
            Language::Java,
            "public interface K extends J {\n    void M();\n}\n",
        ),
        (
            Language::Java,
            "public interface K /* c */ {\n    void M();\n}\n",
        ),
        (
            Language::Java,
            "public interface K {\n    void M();\n    void N();\n}\n",
        ),
    ] {
        assert!(
            match_pattern(lang, src, pat).unwrap().is_empty(),
            "extends/gap-trivia/2-member refuse despite modifiers: {src}"
        );
    }
    // The plain pattern's blanket refusal keeps holding (f135b guard).
    assert!(match_pattern(
        Language::CSharp,
        "public interface K {\n    void P();\n}\n",
        "interface $N { $B }"
    )
    .unwrap()
    .is_empty());
    assert!(match_pattern(
        Language::Java,
        "public interface K {\n    void M();\n}\n",
        "interface $N { $B }"
    )
    .unwrap()
    .is_empty());
    // Trivia BEFORE the modifier attaches outside the declaration (sg n1).
    let lead = match_pattern(
        Language::CSharp,
        "/* lead */ public interface K {\n    void P();\n}\n",
        pat,
    )
    .unwrap();
    assert_eq!(lines_of(&lead), vec![1], "leading trivia keeps answering");
    // ts keeps the export face binding through the wrapper-parent arm —
    // the refusal scan must NOT leak onto ts (the f136 grid's ts_exppat
    // guard flipped when the exact-sequence arm was first shared with ts).
    let ts_exp = match_pattern(
        Language::TypeScript,
        "interface A {\n    a: string;\n}\n\nexport interface B {\n    b: number;\n}\n",
        "export interface $N { $B }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&ts_exp),
        vec![5],
        "the ts export face keeps binding"
    );
    assert_eq!(f127_capture(&ts_exp, "N"), Some("B"));
}

/// f136b (F-135E-2): sg binds `del $X` to the WHOLE operand of EVERY
/// delete_statement shape — name, subscript, attribute, in-function, and
/// the multi-operand list (the f135e tuple pin). sg 0.45.2 135E receipts:
/// `del x`+`del d[k]` → n2 (X=`x`, X=`d[k]`), `del o.a` → n1 (X=`o.a`),
/// `def f(): del x` → n1 (X=`x` at line 2). RED 2026-09-11: the singles
/// answered silent-empty (the 135 admissions let the face ingest without
/// fallback but the serving arm bound only the multi-operand list) — the
/// loud→silent honesty regression this arm closes.
#[test]
fn f136b_py_del_meta_binds_every_operand_shape_sg_exact() {
    let del = match_pattern(Language::Python, "del x\ndel d[k]\n", "del $X").unwrap();
    assert_eq!(
        lines_of(&del),
        vec![1, 2],
        "sg n2: every delete_statement binds its operand: {del:?}"
    );
    assert_eq!(f127_capture(&del, "X"), Some("x"));
    let sub = match_pattern(Language::Python, "del d[k]\n", "del $X").unwrap();
    assert_eq!(lines_of(&sub), vec![1], "subscript operand binds: {sub:?}");
    assert_eq!(f127_capture(&sub, "X"), Some("d[k]"));
    let attr = match_pattern(Language::Python, "del o.a\n", "del $X").unwrap();
    assert_eq!(
        lines_of(&attr),
        vec![1],
        "attribute operand binds: {attr:?}"
    );
    assert_eq!(f127_capture(&attr, "X"), Some("o.a"));
    let infunc = match_pattern(Language::Python, "def f():\n    del x\n", "del $X").unwrap();
    assert_eq!(
        lines_of(&infunc),
        vec![2],
        "in-function delete binds: {infunc:?}"
    );
    assert_eq!(f127_capture(&infunc, "X"), Some("x"));
    // The tuple keeps its whole-list binding (f135e pin unchanged).
    let tuple = match_pattern(Language::Python, "del x, y\n", "del $X").unwrap();
    assert_eq!(lines_of(&tuple), vec![1]);
    assert_eq!(f127_capture(&tuple, "X"), Some("x, y"));
    // Concrete and bare faces keep their lanes (f133/f135 receipts).
    assert_eq!(
        lines_of(&match_pattern(Language::Python, "del x\ndel d[k]\n", "del x").unwrap()),
        vec![1]
    );
}

/// f136c (F-135E-2 grid sibling): js bare `delete` is a keyword-token head —
/// sg 0.45.2 answers the delete_expression family n1 (`delete o.k;`), the
/// subject silent-emptied it through the ident-serve trap (keyword tokens
/// are never `pattern_nodes` identifier rows — the F-131E-2/F-132E genus).
/// RED: empty before the STATEMENT_KEYWORDS escape; no walk arm was ever
/// added — arm-less escaped heads fall through to the literal lane, whose
/// keyword-token-leaf rows are byte-identical to sg's (the 133 doctrine;
/// wording corrected PASS 137, 137B-F5b — the "js kinds arm" this comment
/// first named does not exist).
#[test]
fn f136c_js_bare_delete_answers_the_expression_family() {
    let del = match_pattern(
        Language::JavaScript,
        "const o = {k: 1};\ndelete o.k;\n",
        "delete",
    )
    .unwrap();
    assert_eq!(
        lines_of(&del),
        vec![2],
        "sg n1 on the delete_expression: {del:?}"
    );
}

// ===========================================================================
// PASS 137 — remediation round for verification round r70 (phase137a
// F1-F12 + phase137b F1-F6). Every assertion pins a first-hand sg 0.45.2
// oracle receipt (grid137a /tmp/phase137A + the varprobe/lockprobe/
// pinprobe/javaprobe re-probes); the RED evidence for each pinned face is
// the pre-fix grid row (subject n0/rc2 where sg answers n1, recorded in
// grid137/grid137a BEFORE any fix code was written). Mutation kills are
// named M-137a…M-137k in the report's kill table.
// ===========================================================================

/// f137a (137A-F2/F12): the csharp statement-head lane binds the fixed/
/// checked/unchecked/unsafe faces sg answers. Grid receipts (grid137a B3 +
/// varprobe): `fixed ($D) { *p = 'x'; }` n1 D=`char* p = s`;
/// `checked { int v = a + b; }` n1; `unchecked { int v = a + $C; }` n1
/// C=`1`; `unsafe { $B }` n1 B=`int v = a;`; `fixed (char* p = s) {
/// *p = 'x'; }` n1; `checked { $B }` on a TWO-statement body is
/// accepted-empty (rc1 `[]`) — the single-statement law.
/// M-137a (lane dispatch deleted) re-silences every bind cell.
#[test]
fn f137a_csharp_statement_heads_bind_sg_exact() {
    let pat = "fixed ($D) {\n    *p = 'x';\n}";
    let src = "class K {\n    void m() {\n        fixed (char* p = s) {\n            *p = 'x';\n        }\n    }\n}\n";
    let fixed = match_pattern(Language::CSharp, src, pat).unwrap();
    assert_eq!(
        lines_of(&fixed),
        vec![3],
        "sg n1 D=`char* p = s`: {fixed:?}"
    );
    assert_eq!(f127_capture(&fixed, "D"), Some("char* p = s"));

    let checked = match_pattern(
        Language::CSharp,
        "class K {\n    void m() {\n        checked {\n            int v = a + b;\n        }\n    }\n}\n",
        "checked {\n    int v = a + b;\n}",
    )
    .unwrap();
    assert_eq!(
        lines_of(&checked),
        vec![3],
        "checked concrete n1: {checked:?}"
    );

    let unchecked = match_pattern(
        Language::CSharp,
        "class K {\n    void m() {\n        unchecked {\n            int v = a + 1;\n        }\n    }\n}\n",
        "unchecked {\n    int v = a + $C;\n}",
    )
    .unwrap();
    assert_eq!(
        lines_of(&unchecked),
        vec![3],
        "unchecked meta n1 C=`1`: {unchecked:?}"
    );
    assert_eq!(f127_capture(&unchecked, "C"), Some("1"));

    let unsafe_body = match_pattern(
        Language::CSharp,
        "class K {\n    void m() {\n        unsafe {\n            int v = a;\n        }\n    }\n}\n",
        "unsafe { $B }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&unsafe_body),
        vec![3],
        "unsafe meta-body binds: {unsafe_body:?}"
    );
    assert_eq!(f127_capture(&unsafe_body, "B"), Some("int v = a;"));

    // Two-statement body under the meta-body face: sg accepted-empty.
    let two = match_pattern(
        Language::CSharp,
        "class K {\n    void m() {\n        checked {\n            int q = 1;\n            int r = 2;\n        }\n    }\n}\n",
        "checked { $B }",
    )
    .unwrap();
    assert!(
        two.is_empty(),
        "sg accepted-empty on the 2-statement body: {two:?}"
    );

    // `fixed` meta-body: sg binds NOTHING (valid-empty law).
    let fixed_meta_body = match_pattern(Language::CSharp, src, "fixed ($D) { $B }").unwrap();
    assert!(
        fixed_meta_body.is_empty(),
        "fixed meta-body is sg valid-empty"
    );
}

/// f137b (137A-F2/137B-F4): lock/using meta-body faces are sg-ACCEPTED
/// bind-nothing faces — census-answerable and walk-empty (silent ok:true-0
/// parity), no longer the pass-135 loud class. Concrete bodies keep binding
/// (135 receipts unchanged). M-137b (force_empty reverted to a build
/// refusal) flips `answerable` back to false — the loud regression.
#[test]
fn f137b_lock_using_meta_body_valid_empty() {
    assert!(native_pattern_answerable(
        Language::CSharp,
        "lock ($X) {\n    $B\n}"
    ));
    assert!(!needs_ast_grep_fallback("lock ($X) {\n    $B\n}"));
    let src =
        "class K {\n    void m() {\n        lock (o) {\n            x();\n        }\n    }\n}\n";
    let empty = match_pattern(Language::CSharp, src, "lock ($X) {\n    $B\n}").unwrap();
    assert!(
        empty.is_empty(),
        "sg binds nothing for the meta body: {empty:?}"
    );
    // Concrete body keeps the 135 binding.
    let concrete = match_pattern(Language::CSharp, src, "lock ($X) {\n    x();\n}").unwrap();
    assert_eq!(lines_of(&concrete), vec![3]);
    assert_eq!(f127_capture(&concrete, "X"), Some("o"));
}

/// f137c (137A-F1): the py del lane's operand shapes. Grid receipts
/// (grid137a B2 + pinprobe): `del $A, $B` n1 A=`x` B=`y`; `del d[$K]` n1
/// K=`k`; `del $O.$A` n1 O=`o.a` A=`b`; `del ($X)` n1 X=`x`; `del ($X,
/// $Y)` n1 X=`x` Y=`y`; the whole-list `del $X` pins keep holding.
/// M-137c (elementwise prefix rule M>=N collapsed to M==N) re-silences the
/// 3-element absorptions; M-137c2 (paren unwrap removed) re-silences the
/// paren faces.
#[test]
fn f137c_py_del_operand_shapes_bind_sg_exact() {
    let multi = match_pattern(Language::Python, "del x, y\n", "del $A, $B").unwrap();
    assert_eq!(lines_of(&multi), vec![1], "sg n1 A=x B=y: {multi:?}");
    assert_eq!(f127_capture(&multi, "A"), Some("x"));
    assert_eq!(f127_capture(&multi, "B"), Some("y"));

    let sub = match_pattern(Language::Python, "del d[k]\n", "del d[$K]").unwrap();
    assert_eq!(lines_of(&sub), vec![1]);
    assert_eq!(f127_capture(&sub, "K"), Some("k"));

    let attr = match_pattern(Language::Python, "del o.a.b\n", "del $O.$A").unwrap();
    assert_eq!(lines_of(&attr), vec![1], "sg n1 O=`o.a` A=`b`: {attr:?}");
    assert_eq!(f127_capture(&attr, "O"), Some("o.a"));
    assert_eq!(f127_capture(&attr, "A"), Some("b"));

    let paren = match_pattern(Language::Python, "del (x)\n", "del ($X)").unwrap();
    assert_eq!(lines_of(&paren), vec![1], "sg n1 X=`x`: {paren:?}");
    assert_eq!(f127_capture(&paren, "X"), Some("x"));

    let paren_pair = match_pattern(Language::Python, "del (x, y)\n", "del ($X, $Y)").unwrap();
    assert_eq!(
        lines_of(&paren_pair),
        vec![1],
        "pinprobe n1 X=x Y=y: {paren_pair:?}"
    );
    assert_eq!(f127_capture(&paren_pair, "X"), Some("x"));
    assert_eq!(f127_capture(&paren_pair, "Y"), Some("y"));

    // M>=N prefix absorption (grid137 X_py_del_three): 3-element candidate.
    let three = match_pattern(Language::Python, "del x, y, z\n", "del $A, $B").unwrap();
    assert_eq!(
        lines_of(&three),
        vec![1],
        "sg absorbs the third element: {three:?}"
    );
    // Structural shapes demand the exact count (sg rc1 class).
    assert!(
        match_pattern(Language::Python, "del d[k], y\n", "del d[$K]")
            .unwrap()
            .is_empty(),
        "subscript + extra refuses (sg count law)"
    );
    // `;`-ful del template spellings are sg ACCEPTED-EMPTY in the
    // multi-language run (grid137 py_del_semi_loud: rc1 `[]` for the whole
    // family; the subject pre-fix rc2'd — the grid DIFF row is the RED).
    let semi = match_pattern(Language::Python, "del o.a.b\n", "del $O.$A;").unwrap();
    assert!(
        semi.is_empty(),
        "sg accepted-empty on `del $O.$A;`: {semi:?}"
    );
    assert!(native_pattern_answerable(Language::Python, "del $O.$A;"));

    // Whole-list pins keep holding (f135e/f136).
    let whole = match_pattern(Language::Python, "del x, y\n", "del $X").unwrap();
    assert_eq!(f127_capture(&whole, "X"), Some("x, y"));
    let paren_src = match_pattern(Language::Python, "del (x)\n", "del $X").unwrap();
    assert_eq!(f127_capture(&paren_src, "X"), Some("(x)"));
}

/// f137d (137A-F5, java semantics correction of record): the interface
/// modifier scan is a greedy keyword-hopping scan with POSITIONAL
/// annotations. javaprobe/pinprobe receipts: `public interface $N` binds
/// `abstract public`/`static public`/`static final public` (n1 each);
/// `public abstract interface $N` binds `public static abstract`; `public
/// abstract interface $N` refuses `abstract public` (order); `@SafeVarargs
/// interface $N` binds `@SafeVarargs public` and refuses `@Deprecated
/// @SafeVarargs public`; `@Deprecated interface $N` binds `@Deprecated
/// public` and refuses `public @Deprecated`. M-137d (keyword hopping
/// removed → prefix) re-refuses the `abstract public` bind cells.
#[test]
fn f137d_java_interface_modifier_scan_is_hopping_with_positional_annotations() {
    let binds = |src: &str, pat: &str| {
        let hits = match_pattern(Language::Java, src, pat).unwrap();
        assert_eq!(
            lines_of(&hits),
            vec![1],
            "sg binds {pat:?} on {src:?}: {hits:?}"
        );
    };
    let refuses = |src: &str, pat: &str| {
        assert!(
            match_pattern(Language::Java, src, pat).unwrap().is_empty(),
            "sg refuses {pat:?} on {src:?}"
        );
    };
    let wrap = |mods: &str| format!("{mods} interface K {{\n    void M();\n}}\n");
    binds(&wrap("abstract public"), "public interface $N { $B }");
    binds(&wrap("static public"), "public interface $N { $B }");
    binds(&wrap("static final public"), "public interface $N { $B }");
    binds(&wrap("public @Deprecated"), "public interface $N { $B }");
    binds(
        &wrap("public static abstract"),
        "public abstract interface $N { $B }",
    );
    refuses(
        &wrap("abstract public"),
        "public abstract interface $N { $B }",
    );
    refuses(&wrap("@Deprecated public"), "public interface $N { $B }");
    refuses(&wrap("private"), "public interface $N { $B }");
    // Annotations are positional.
    binds(
        &wrap("@SafeVarargs public"),
        "@SafeVarargs interface $N { $B }",
    );
    refuses(
        &wrap("@Deprecated @SafeVarargs public"),
        "@SafeVarargs interface $N { $B }",
    );
    binds(
        &wrap("@Deprecated public"),
        "@Deprecated interface $N { $B }",
    );
    refuses(
        &wrap("public @Deprecated"),
        "@Deprecated interface $N { $B }",
    );
    binds(
        &wrap("@Deprecated @SafeVarargs public"),
        "@Deprecated @SafeVarargs interface $N { $B }",
    );
}

/// f137e (137A-F12 + 137B-F6): the pattern-side strip list admits the
/// sealed/partial/strictfp interface faces sg answers (grid137a B1:
/// cs_sealedpat_bind / ja_strictpat_bind sg n1 vs subject rc2; grid137
/// cs_partialpat_bind sg n1 vs n0). M-137e (strip list reverted) re-louds
/// all three.
#[test]
fn f137e_sealed_partial_strictfp_interface_patterns_bind() {
    let cs = match_pattern(
        Language::CSharp,
        "sealed interface K {\n    void P();\n}\n",
        "sealed interface $N { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&cs), vec![1], "grid137a B1: sg n1: {cs:?}");
    assert_eq!(f127_capture(&cs, "N"), Some("K"));
    let partial = match_pattern(
        Language::CSharp,
        "partial interface K {\n    void P();\n}\n",
        "partial interface $N { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&partial), vec![1], "grid137: sg n1: {partial:?}");
    let ja = match_pattern(
        Language::Java,
        "strictfp interface K {\n    void M();\n}\n",
        "strictfp interface $N { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&ja), vec![1], "grid137a B1: sg n1: {ja:?}");
    assert_eq!(f127_capture(&ja, "N"), Some("K"));
}

/// f137f (137B-F2/F3): the php member-slot metas — the `$A->y` flat
/// property receipt (pinprobe n1 A=`$x`, the general lane's placeholder
/// binding serving the face) and the nullsafe TOKEN-EXACTNESS correction
/// of record. M-137f / M-137f2 recorded INERT in the kill table (2026-09-11
/// mutation cycle): (f) the cell's serving route is the general lane's
/// shared placeholder machinery — no single-site mutant isolates it without
/// gutting the lane every other cell rides; (f2) re-adding a `?->`
/// normalization inside `bind_slot_face_callee_metas` is behaviorally dead
/// — the walk's token-exact candidate gates refuse the cross spellings
/// before any callee-meta binding runs, which IS the refutation's
/// mechanism.
#[test]
fn f137f_php_member_slot_absorption_and_nullsafe() {
    let absorb = match_pattern(Language::Php, "<?php\n$x->y;\n", "$A->y").unwrap();
    assert_eq!(lines_of(&absorb), vec![2], "pinprobe n1 A=`$x`: {absorb:?}");
    assert_eq!(f127_capture(&absorb, "A"), Some("$x"));
    // Nullsafe is TOKEN-EXACT both directions (oracle probe 2026-09-11,
    // 137B-F2 refuted): `$O->c1($U)` × `$o?->c1($u);` is sg rc1 [] and
    // `$O?->c1($U)` × `$o->c1($u);` is sg rc1 [] — the accepted-empty class
    // (subject answers ok:true-0), never a `?->` normalization.
    let nullsafe = match_pattern(Language::Php, "<?php\n$o?->c1($u);\n", "$O->c1($U)").unwrap();
    assert!(
        nullsafe.is_empty(),
        "sg accepted-empty on ?-> vs ->: {nullsafe:?}"
    );
}

/// f137g (137A-D3 grid): `return ($X)` / `return($X)` answer the
/// parenthesized return operand (X = the INNER text), not the Call lane's
/// never-firing callee face. M-137g (carve deleted) re-silences both.
#[test]
fn f137g_return_paren_meta_binds_the_inner_operand() {
    let js = match_pattern(
        Language::JavaScript,
        "function f() {\n  return (1);\n}\n",
        "return ($X)",
    )
    .unwrap();
    assert_eq!(lines_of(&js), vec![2], "sg n1 X=`1`: {js:?}");
    assert_eq!(f127_capture(&js, "X"), Some("1"));
    let tight = match_pattern(
        Language::JavaScript,
        "function f() {\n  return (1);\n}\n",
        "return($X)",
    )
    .unwrap();
    assert_eq!(
        lines_of(&tight),
        vec![2],
        "sg n1 for the tight spelling: {tight:?}"
    );
    assert_eq!(f127_capture(&tight, "X"), Some("1"));
    let ts = match_pattern(
        Language::TypeScript,
        "function f() {\n  return (2);\n}\n",
        "return ($X)",
    )
    .unwrap();
    assert_eq!(lines_of(&ts), vec![2]);
    // Paren-free and operand-less candidates refuse (structural).
    assert!(match_pattern(
        Language::JavaScript,
        "function f() {\n  return 1;\n}\n",
        "return ($X)"
    )
    .unwrap()
    .is_empty());
    assert!(match_pattern(
        Language::JavaScript,
        "function f() {\n  return;\n}\n",
        "return ($X)"
    )
    .unwrap()
    .is_empty());
}

/// f137h (137A-D6/F5-corrected else-if): the if lane parses `else { B }`
/// and `else if (COND) { B }` tails and binds every chained capture. Oracle
/// receipts: elseif_same_line_F5 n1 X=a A=`f();` Y=b B=`g();`; else-only n1
/// (X/A/B); the 3-chain binds all six (pinprobe e.js). M-137h (tail parse
/// deleted → classify None) re-louds the face.
#[test]
fn f137h_if_else_and_else_if_tails_bind_every_capture() {
    let elseif = match_pattern(
        Language::JavaScript,
        "if (a) {\n  f();\n} else if (b) {\n  g();\n}\n",
        "if ($X) {\n  $A\n} else if ($Y) {\n  $B\n}",
    )
    .unwrap();
    assert_eq!(lines_of(&elseif), vec![1], "grid D6 n1: {elseif:?}");
    assert_eq!(f127_capture(&elseif, "X"), Some("a"));
    assert_eq!(f127_capture(&elseif, "A"), Some("f();"));
    assert_eq!(f127_capture(&elseif, "Y"), Some("b"));
    assert_eq!(f127_capture(&elseif, "B"), Some("g();"));

    let els = match_pattern(
        Language::JavaScript,
        "if (a) {\n  f();\n} else {\n  g();\n}\n",
        "if ($X) { $A } else { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&els), vec![1], "else-only n1: {els:?}");
    assert_eq!(f127_capture(&els, "B"), Some("g();"));

    let chain = match_pattern(
        Language::JavaScript,
        "if (a) {\n  f();\n} else if (b) {\n  g();\n} else if (c) {\n  k();\n}\n",
        "if ($X) { $A } else if ($Y) { $B } else if ($Z) { $C }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&chain),
        vec![1],
        "3-chain binds all six: {chain:?}"
    );
    assert_eq!(f127_capture(&chain, "Z"), Some("c"));
    assert_eq!(f127_capture(&chain, "C"), Some("k();"));

    // An else-less candidate refuses the else-carrying pattern.
    assert!(
        match_pattern(
            Language::JavaScript,
            "if (a) {\n  f();\n}\n",
            "if ($X) { $A } else { $B }",
        )
        .unwrap()
        .is_empty(),
        "missing else tail refuses"
    );
}

/// f137i (137A-B3/B5/B7 admissions): the sg-answered faces whose heads/roots
/// joined the general lane — js `export default $X;`, java
/// `assert $X : $M;` + `synchronized ($X) { … }`, cpp `using namespace
/// $N;`, php `namespace $N;` + `goto $L;`. M-137i (root-kind admission
/// deleted) re-louds the class.
#[test]
fn f137i_general_lane_statement_root_admissions_bind_sg_exact() {
    let export = match_pattern(
        Language::JavaScript,
        "export default 42;\n",
        "export default $X;",
    )
    .unwrap();
    assert_eq!(
        lines_of(&export),
        vec![1],
        "grid B5: sg n1 X=`42`: {export:?}"
    );
    assert_eq!(f127_capture(&export, "X"), Some("42"));

    let assert_msg = match_pattern(
        Language::Java,
        "class K {\n    void m() {\n        assert b : \"msg\";\n    }\n}\n",
        "assert $X : $M;",
    )
    .unwrap();
    assert_eq!(
        lines_of(&assert_msg),
        vec![3],
        "grid B3: sg n1: {assert_msg:?}"
    );
    assert_eq!(f127_capture(&assert_msg, "X"), Some("b"));
    assert_eq!(f127_capture(&assert_msg, "M"), Some("\"msg\""));

    let sync = match_pattern(
        Language::Java,
        "class K {\n    void m() {\n        synchronized (lock) {\n            doIt();\n        }\n    }\n}\n",
        "synchronized ($X) {\n    doIt();\n}",
    )
    .unwrap();
    assert_eq!(lines_of(&sync), vec![3], "grid B3: sg n1: {sync:?}");
    assert_eq!(f127_capture(&sync, "X"), Some("lock"));

    let cpp_using = match_pattern(
        Language::Cpp,
        "using namespace std;\n",
        "using namespace $N;",
    )
    .unwrap();
    assert_eq!(
        lines_of(&cpp_using),
        vec![1],
        "grid B7: sg n1 N=`std`: {cpp_using:?}"
    );

    // Ingress parity (grid137 DIFF rows → OK): the language-free
    // needs_ast_grep_fallback must admit the faces the per-language lanes
    // answer (pre-fix the CLI rc2'd them before any walk ran).
    assert!(!needs_ast_grep_fallback("assert $X : $M;"));
    assert!(!needs_ast_grep_fallback(
        "synchronized ($X) {\n    doIt();\n}"
    ));
    assert!(!needs_ast_grep_fallback("namespace $N;"));
    assert!(!needs_ast_grep_fallback("goto $L;"));
    assert_eq!(f127_capture(&cpp_using, "N"), Some("std"));

    let ns = match_pattern(Language::Php, "<?php\nnamespace App;\n", "namespace $N;").unwrap();
    assert_eq!(lines_of(&ns), vec![2], "grid B7: sg n1 N=`App`: {ns:?}");
    assert_eq!(f127_capture(&ns, "N"), Some("App"));

    let goto = match_pattern(Language::Php, "<?php\ngoto a;\na:\n", "goto $L;").unwrap();
    assert_eq!(lines_of(&goto), vec![2], "grid B7: sg n1 L=`a`: {goto:?}");
    assert_eq!(f127_capture(&goto, "L"), Some("a"));
}

/// f137j (137A-D3): go/rb `return;` is an sg-ACCEPTED valid-empty face —
/// census-answerable, walk-empty (the go/rb grammars spell no `;`).
/// M-137j (census arm deleted) re-louds the class.
#[test]
fn f137j_go_rb_return_semi_valid_empty() {
    assert!(native_pattern_answerable(Language::Go, "return;"));
    assert!(native_pattern_answerable(Language::Ruby, "return;"));
    let go = match_pattern(Language::Go, "func f() {\n\treturn\n}\n", "return;").unwrap();
    assert!(
        go.is_empty(),
        "sg accepted-empty on the bare go return: {go:?}"
    );
    let rb = match_pattern(Language::Ruby, "def f\n  return\nend\n", "return;").unwrap();
    assert!(
        rb.is_empty(),
        "sg accepted-empty on the bare rb return: {rb:?}"
    );
}

/// f137k (137A-F11, §45.11 receipt refuted): py bare `await` ANSWERS n1
/// (grid137 F + reg_py_bare_await) through the literal lane's keyword-token
/// route. M-137k (route reverted to silent-empty) re-silences the face.
#[test]
fn f137k_py_bare_await_answers() {
    let hits = match_pattern(Language::Python, "await\n", "await").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "grid F: sg n1: {hits:?}");
}

/// f137l (137A-F2 bare cells): the csharp statement-keyword escapes serve
/// the bare spellings sg answers per site (grid137 A-lane: bare cs
/// fixed/checked/unchecked/unsafe/lock/using n1, var n2; js var n2; rs
/// unsafe n1 — all subject n0 pre-fix). M-137l (escape deleted) re-traps
/// the class in the ident-serve silence.
#[test]
fn f137l_bare_csharp_statement_keywords_answer_per_site() {
    // Each bare spelling answers the keyword-token row on a fixture that
    // holds its statement (grid137 A-lane cells: sg n1 per site).
    let cs_src = "class K {\n    void m() {\n        fixed (char* p = s) {\n            *p = 'x';\n        }\n        lock (o) {\n            x();\n        }\n        using (var f = G()) {\n            f.Close();\n        }\n    }\n}\n";
    for pat in ["fixed", "lock", "using"] {
        let hits = match_pattern(Language::CSharp, cs_src, pat).unwrap();
        assert!(
            !hits.is_empty(),
            "bare {pat:?} answers keyword-token rows (grid A-lane): {hits:?}"
        );
    }
    let js_var = match_pattern(Language::JavaScript, "var a = 1;\nvar b = 2;\n", "var").unwrap();
    assert_eq!(
        lines_of(&js_var),
        vec![1, 2],
        "bare js `var` sg n2: {js_var:?}"
    );
    let rs_unsafe = match_pattern(
        Language::Rust,
        "fn f() {\n    unsafe {\n        let x = 1;\n    }\n}\n",
        "unsafe",
    )
    .unwrap();
    assert_eq!(
        lines_of(&rs_unsafe),
        vec![2],
        "bare rs `unsafe` sg n1: {rs_unsafe:?}"
    );
}

/// f138a (F-137E-1): the py bare-`await` route REFUSES operand-bearing
/// await_expression lines. sg's bare pattern binds only the bare keyword
/// subtree — the 138 grid (/tmp/phase138, parse probe /tmp/phase138/
/// pyparse_probe: a bare `await` token error-recovers to a plain
/// `identifier`, while `await <expr>` yields an `await`-kind node with a
/// named operand child sg's pattern does not match): the async-def call
/// (the F-137E-1 cell), top-level call, assignment, async comprehension,
/// and paren-operand faces are sg rc1 `[]`, and the mixed fixture answers
/// ONLY the bare statement line. M-138a (operand filter deleted)
/// re-over-serves every cell.
#[test]
fn f138a_py_bare_await_refuses_operand_await_expressions() {
    let def = match_pattern(Language::Python, "async def f():\n    await g()\n", "await").unwrap();
    assert!(
        def.is_empty(),
        "grid S2: sg rc1 [] on the async-def call: {def:?}"
    );
    let top = match_pattern(Language::Python, "await g()\n", "await").unwrap();
    assert!(
        top.is_empty(),
        "grid S3: sg rc1 [] on the top-level call: {top:?}"
    );
    let assign = match_pattern(
        Language::Python,
        "async def f():\n    x = await g()\n",
        "await",
    )
    .unwrap();
    assert!(
        assign.is_empty(),
        "grid S4: sg rc1 [] on the assignment: {assign:?}"
    );
    let comp = match_pattern(
        Language::Python,
        "async def f():\n    r = [await g(i) async for i in items]\n",
        "await",
    )
    .unwrap();
    assert!(
        comp.is_empty(),
        "grid S5: sg rc1 [] on the async comprehension: {comp:?}"
    );
    let paren = match_pattern(
        Language::Python,
        "async def f():\n    x = (await g())\n",
        "await",
    )
    .unwrap();
    assert!(
        paren.is_empty(),
        "grid S15: sg rc1 [] on the paren operand: {paren:?}"
    );
    let mixed = match_pattern(
        Language::Python,
        "async def f():\n    await\n    await g()\n    x = await h()\n",
        "await",
    )
    .unwrap();
    assert_eq!(
        lines_of(&mixed),
        vec![2],
        "grid S9: only the bare statement line binds: {mixed:?}"
    );
}

/// f138b (F-137E-1 keep side): the operand-free bare-`await` statement
/// faces KEEP the §46-refuted binding (grid138 S1/S6/S7/S8/S10/S11/S13/S16 +
/// edges E1/E2: sg answers n per site — bare, `;`-terminated,
/// `await ;`, trailing comment, `await; g()` where `;` ends the statement,
/// two bare rows on one line, and the ident-recovery `await = 5`). The
/// `;`-ful PATTERN spelling stays sg valid-empty on every fixture.
/// M-138b (py arm reverted to the pre-137 identifier-only route)
/// re-silences the class (the f137k kill).
#[test]
fn f138b_py_bare_await_statement_faces_keep_binding() {
    assert_eq!(
        lines_of(&match_pattern(Language::Python, "await\n", "await").unwrap()),
        vec![1],
        "S1: the §46-refutation receipt cell"
    );
    let in_def = match_pattern(Language::Python, "async def f():\n    await\n", "await").unwrap();
    assert_eq!(
        lines_of(&in_def),
        vec![2],
        "S6: sg n1 at the bare statement: {in_def:?}"
    );
    let semi = match_pattern(Language::Python, "async def f():\n    await;\n", "await").unwrap();
    assert_eq!(
        lines_of(&semi),
        vec![2],
        "S7: `;`-terminated statement binds: {semi:?}"
    );
    let space_semi = match_pattern(Language::Python, "await ;\n", "await").unwrap();
    assert_eq!(
        lines_of(&space_semi),
        vec![1],
        "S13: spaced `;` binds: {space_semi:?}"
    );
    let comment = match_pattern(Language::Python, "await  # hi\n", "await").unwrap();
    assert_eq!(
        lines_of(&comment),
        vec![1],
        "S11: trailing comment is trivia: {comment:?}"
    );
    let semi_stmt = match_pattern(
        Language::Python,
        "async def f():\n    await; g()\n",
        "await",
    )
    .unwrap();
    assert_eq!(
        lines_of(&semi_stmt),
        vec![2],
        "S16: `;` ends the bare statement: {semi_stmt:?}"
    );
    let two = match_pattern(
        Language::Python,
        "async def f():\n    await\n    await\n",
        "await",
    )
    .unwrap();
    assert_eq!(lines_of(&two), vec![2, 3], "S10: n per site: {two:?}");
    let oneline = match_pattern(Language::Python, "await; await\n", "await").unwrap();
    assert_eq!(
        lines_of(&oneline),
        vec![1, 1],
        "E1: one row per token: {oneline:?}"
    );
    let ident = match_pattern(Language::Python, "await = 5\n", "await").unwrap();
    assert_eq!(
        lines_of(&ident),
        vec![1],
        "E2: ident-recovery spelling binds: {ident:?}"
    );
    for src in ["await\n", "async def f():\n    await g()\n"] {
        let hits = match_pattern(Language::Python, src, "await;").unwrap();
        assert!(
            hits.is_empty(),
            "pattern `await;` is sg-rc1 empty: {hits:?}"
        );
    }
}

/// f139a (139A-F1, grid139 A /tmp/phase139R/gridA): the csharp statement-
/// head lane serves NESTED head compositions sg-exactly — sg binds ONE
/// match at the OUTERMOST statement and every level's meta binds its own
/// node (`fixed ($D) { checked { $B } }` → D=`int* p = arr`,
/// B=`x = 1;` — oracle metaVariables receipt). The fixed/lock/using
/// binds-nothing law holds at every level but ONLY for the BARE-meta body
/// (nested-head bodies under those heads BIND: grid A01/A02/A08/A09/A24/
/// A29/A30 n1; bare-meta-under-fixed/lock/using at any depth rc1 `[]`:
/// A05/A07/A19/A20/A22). One-statement law per pattern body slot (A06/
/// A15/A16/A26/A27 empty). 0-hits-silent pre-fix on every bind cell.
#[test]
fn f139a_cs_nested_statement_head_compositions_bind() {
    let nested = "class C {\n    void M() {\n        fixed (int* p = arr) {\n            checked { x = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, nested, "fixed ($D) { checked { $B } }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![3],
        "grid A01: sg n1 @ the fixed stmt: {hits:?}"
    );
    assert_eq!(
        hits[0].captures.get("D").map(String::as_str),
        Some("int* p = arr"),
        "grid A01: D binds the resource: {hits:?}"
    );
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("x = 1;"),
        "grid A01: B binds the INNERMOST body statement: {hits:?}"
    );
    let unchk = match_pattern(Language::CSharp, nested, "fixed ($D) { unchecked { $B } }").unwrap();
    assert!(
        unchk.is_empty(),
        "grid A02 twin on a checked candidate: {unchk:?}"
    );
    let unsafe_checked = "class C {\n    void M() {\n        unsafe {\n            checked { x = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(
        Language::CSharp,
        unsafe_checked,
        "unsafe { checked { $B } }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![3],
        "grid A03: sg n1 @ the unsafe stmt: {hits:?}"
    );
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("x = 1;"),
        "grid A03: B = the inner body: {hits:?}"
    );
    let checked_unchecked = "class C {\n    void M() {\n        checked {\n            unchecked { x = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(
        Language::CSharp,
        checked_unchecked,
        "checked { unchecked { $B } }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid A04: sg n1: {hits:?}");
    // The binds-nothing law composes at depth (grid A05/A07/A19/A20/A22:
    // sg rc1 `[]` = the walk's empty).
    let unchecked_fixed = "class C {\n    void M() {\n        unchecked {\n            fixed (int* p = arr) { x = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(
        Language::CSharp,
        unchecked_fixed,
        "unchecked { fixed ($D) { $B } }",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "grid A05: fixed meta-body under nesting: {hits:?}"
    );
    let unsafe_fixed = "class C {\n    void M() {\n        unsafe {\n            fixed (int* p = arr) { *p = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(
        Language::CSharp,
        unsafe_fixed,
        "unsafe { fixed ($D) { $B } }",
    )
    .unwrap();
    assert!(hits.is_empty(), "grid A07: {hits:?}");
    let unsafe_lock = "class C {\n    void M() {\n        unsafe {\n            lock (o) { x = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, unsafe_lock, "unsafe { lock ($L) { $B } }").unwrap();
    assert!(hits.is_empty(), "grid A19: {hits:?}");
    let fixed_fixed = "class C {\n    void M() {\n        fixed (int* p = arr) {\n            fixed (char* q = b) { *q = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(
        Language::CSharp,
        fixed_fixed,
        "fixed ($D1) { fixed ($D2) { $B } }",
    )
    .unwrap();
    assert!(hits.is_empty(), "grid A22: {hits:?}");
    // lock/using with nested or placeholder-carrying bodies BIND (grid
    // A08/A09/A24/A29/A30: sg n1 — L/R bind the resource, B/V the body).
    let lock_checked = "class C {\n    void M() {\n        lock (obj) {\n            checked { x = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(
        Language::CSharp,
        lock_checked,
        "lock ($L) { checked { $B } }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![3],
        "grid A08: sg n1 @ the lock stmt: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("L").map(String::as_str), Some("obj"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("x = 1;")
    );
    let lock_conc = "class C {\n    void M() {\n        lock (o) { x = 1; }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, lock_conc, "lock ($L) { x = $V; }").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid A29: sg n1: {hits:?}");
    assert_eq!(hits[0].captures.get("L").map(String::as_str), Some("o"));
    assert_eq!(hits[0].captures.get("V").map(String::as_str), Some("1"));
    let using_checked = "class C {\n    void M() {\n        using (var d = Open()) {\n            checked { x = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(
        Language::CSharp,
        using_checked,
        "using ($R) { checked { x = $V; } }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid A09-family: sg n1: {hits:?}");
    assert_eq!(
        hits[0].captures.get("R").map(String::as_str),
        Some("var d = Open()")
    );
    assert_eq!(hits[0].captures.get("V").map(String::as_str), Some("1"));
    // Multi-statement bodies refuse at every level (grid A06/A15/A16/A26/
    // A27: sg rc1 `[]`).
    let two_body = "class C {\n    void M() {\n        fixed (int* p = arr) {\n            checked { x = 1; y = 2; }\n        }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, two_body, "fixed ($D) { checked { $B } }").unwrap();
    assert!(hits.is_empty(), "grid A06: inner 2-stmt body: {hits:?}");
    let outer_two = "class C {\n    void M() {\n        unsafe {\n            y = 0;\n            checked { x = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, outer_two, "unsafe { checked { $B } }").unwrap();
    assert!(hits.is_empty(), "grid A26: outer 2-stmt body: {hits:?}");
    // Depth-3 binds at the outermost (grid A10/A11).
    let depth3 = "class C {\n    void M() {\n        unsafe {\n            checked {\n                unchecked { x = 1; }\n            }\n        }\n    }\n}\n";
    let hits = match_pattern(
        Language::CSharp,
        depth3,
        "unsafe { checked { unchecked { $B } } }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![3],
        "grid A10: sg n1 @ the outer stmt: {hits:?}"
    );
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("x = 1;")
    );
    // Controls: the 137 cells hold (A12/A13/A14/A17/A18/A21).
    let ctl = match_pattern(Language::CSharp, nested, "checked { $B }").unwrap();
    assert_eq!(
        lines_of(&ctl),
        vec![4],
        "grid A21: 1-level on a nested fixture: {ctl:?}"
    );
    let conc = match_pattern(
        Language::CSharp,
        nested,
        "fixed (int* p = arr) { checked { x = 1; } }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&conc),
        vec![3],
        "grid A12: full-concrete nested: {conc:?}"
    );
    let inner_meta = match_pattern(
        Language::CSharp,
        nested,
        "fixed ($D) { checked { x = $V; } }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&inner_meta),
        vec![3],
        "grid A17: concrete inner meta: {inner_meta:?}"
    );
}

/// f139b (139A-F2, grid139 B /tmp/phase139R/gridBCDEF): the java
/// synchronized META-body face binds the block statement sg-exactly —
/// `synchronized ($X) { $B }` × the block candidate n1 (X=`lock`,
/// B=`doIt();` — oracle metaVariables receipt); literal resource B05 n1;
/// 0/2+-statement bodies refuse (B02/B06 sg rc1 `[]`); the concrete-body
/// spellings keep their 137 general-lane route (B03/B04 controls). rc2
/// census-loud pre-fix on every meta-body cell.
#[test]
fn f139b_java_synchronized_meta_body_binds_block() {
    let src = "class S {\n    synchronized void m() {}\n    void n() {\n        synchronized (lock) {\n            doIt();\n        }\n    }\n}\n";
    let hits = match_pattern(Language::Java, src, "synchronized ($X) { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![4],
        "grid B01: sg n1 @ the block ONLY: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("lock"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("doIt();")
    );
    let two = "class S {\n    void n() {\n        synchronized (lock) {\n            doIt();\n            more();\n        }\n    }\n}\n";
    let hits = match_pattern(Language::Java, two, "synchronized ($X) { $B }").unwrap();
    assert!(
        hits.is_empty(),
        "grid B02: 2-stmt body is sg rc1 []: {hits:?}"
    );
    let empty = "class S {\n    void n() {\n        synchronized (lock) {\n        }\n    }\n}\n";
    let hits = match_pattern(Language::Java, empty, "synchronized ($X) { $B }").unwrap();
    assert!(hits.is_empty(), "grid B06: empty body refuses: {hits:?}");
    let hits = match_pattern(Language::Java, src, "synchronized (lock) { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![4],
        "grid B05: literal resource binds: {hits:?}"
    );
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("doIt();")
    );
    // Controls: the concrete-body 137 cells (grid B03/B04 AGREE).
    let hits = match_pattern(Language::Java, src, "synchronized ($X) {\n doIt();\n}").unwrap();
    assert_eq!(lines_of(&hits), vec![4], "grid B03 control: {hits:?}");
}

/// f139c (139A-F3, grid139 C): the py del lane unifies postfix chains
/// LEVEL BY LEVEL (sg's structural law: metas bind their level's node
/// text, literals byte-match, kinds must agree per level). Chained
/// spellings bound nothing / census-louded pre-fix.
#[test]
fn f139c_py_del_postfix_chains_unify_per_level() {
    let hits = match_pattern(Language::Python, "del d[k1][k2]\n", "del $O[$K1][$K2]").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "grid C01: sg n1: {hits:?}");
    assert_eq!(hits[0].captures.get("O").map(String::as_str), Some("d"));
    assert_eq!(hits[0].captures.get("K1").map(String::as_str), Some("k1"));
    assert_eq!(hits[0].captures.get("K2").map(String::as_str), Some("k2"));
    let hits = match_pattern(Language::Python, "del d[k].b\n", "del $O[$K].$A").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid C02: sub→attr binds: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("O").map(String::as_str), Some("d"));
    assert_eq!(hits[0].captures.get("K").map(String::as_str), Some("k"));
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("b"));
    let hits = match_pattern(Language::Python, "del d.b[k]\n", "del $O.$A[$K]").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid C03: attr→sub binds: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("O").map(String::as_str), Some("d"));
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("b"));
    assert_eq!(hits[0].captures.get("K").map(String::as_str), Some("k"));
    let hits = match_pattern(
        Language::Python,
        "del d[k1][k2][k3]\n",
        "del $O[$K1][$K2][$K3]",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid C04: 3-level binds: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del x.y.z\n", "del $O.$A.$B").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid C06: 3-attr chain binds: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("O").map(String::as_str), Some("x"));
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("y"));
    assert_eq!(hits[0].captures.get("B").map(String::as_str), Some("z"));
    let hits = match_pattern(Language::Python, "del x.y.z\n", "del $A.$B.$C").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid C19: all-meta chain binds: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del d[k1][k2]\n", "del d[$K][$J]").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid C15: literal head + meta idxs: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("K").map(String::as_str), Some("k1"));
    let hits = match_pattern(Language::Python, "del d[k]\n", "del $O[k]").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid C10: meta head + literal idx: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("O").map(String::as_str), Some("d"));
    // Literals byte-match per level; kinds must agree per level.
    let hits = match_pattern(Language::Python, "del d[k1][k2]\n", "del $O[$K][j]").unwrap();
    assert!(
        hits.is_empty(),
        "grid C14: literal `j` != `k2` refuses: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del x.y.z\n", "del m.n.$A").unwrap();
    assert!(
        hits.is_empty(),
        "grid C11: literal `m` != `x` refuses: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del d.b[k]\n", "del m.n[$K]").unwrap();
    assert!(
        hits.is_empty(),
        "grid C13: literal receiver mismatch: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del x.y\n", "del $O[$K]").unwrap();
    assert!(
        hits.is_empty(),
        "grid C18: attribute candidate vs subscript: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del d[k], y\n", "del $O[$K]").unwrap();
    assert!(
        hits.is_empty(),
        "grid C17: structural demands M==N: {hits:?}"
    );
    // The 137 family holds (controls).
    let hits = match_pattern(Language::Python, "del d[k]\n", "del d[$K]").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "137 control C09: {hits:?}");
    let hits = match_pattern(Language::Python, "del x.y.z\n", "del $O.$A").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "137 control C08: {hits:?}");
    let hits = match_pattern(Language::Python, "del x.y.z\n", "del $O.b.$A").unwrap();
    assert!(
        hits.is_empty(),
        "grid C12: literal `b` != `y` refuses: {hits:?}"
    );
}

/// f139d (139B-F1, grid139 G): the del paren wrap is structurally
/// significant in BOTH directions — the paren-required pattern refuses
/// the paren-free candidate (G01/G03 sg rc1 `[]`; the subject bound G01
/// pre-fix — the 137 comment claimed a refusal the code never performed),
/// and `del (x)` × `del (x)` BINDS (G07 sg n1; silent-0 pre-fix).
#[test]
fn f139d_py_del_paren_wrap_is_structural() {
    let hits = match_pattern(Language::Python, "del x, y\n", "del ($X, $Y)").unwrap();
    assert!(
        hits.is_empty(),
        "grid G01: paren pattern vs free candidate: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del x\n", "del ($X)").unwrap();
    assert!(
        hits.is_empty(),
        "grid G03: single paren meta vs free: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del x\n", "del (x)").unwrap();
    assert!(hits.is_empty(), "grid G02: literal paren vs free: {hits:?}");
    let hits = match_pattern(Language::Python, "del (x, y)\n", "del $X, $Y").unwrap();
    assert!(
        hits.is_empty(),
        "grid G04: free pattern vs paren candidate: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del (x, y)\n", "del ($X, $Y)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid G05 ctl: paren pair binds: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("x"));
    let hits = match_pattern(Language::Python, "del (x)\n", "del (x)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid G07: paren literal binds: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del (y)\n", "del (x)").unwrap();
    assert!(
        hits.is_empty(),
        "grid G07-twin: literal mismatch refuses: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del (x)\n", "del $X").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid G08 ctl: whole absorbs parens: {hits:?}"
    );
    // The paren-structural spelling is sg-ACCEPTED binds-nothing (C16).
    let hits = match_pattern(Language::Python, "del d[k1][k2]\n", "del ($O[$K])").unwrap();
    assert!(
        hits.is_empty(),
        "grid C16: paren structural binds nothing: {hits:?}"
    );
}

/// f139e (139A-F4 REGISTERED, grid139 D): the go `select` meta comm-clause
/// family stays census-LOUD — sg binds the bind-clause/recv-only/send/
/// default forms n1 (D01–D05) and rc1-refuses the comma-ok spelling (D06),
/// but the per-clause comm machinery (recv vs recv-bind kinds, $$$ multi
/// statement bodies, per-clause captures) is beyond this round's admitted
/// surface: the faces keep the registered fail-closed loud class (zero
/// wrong answers; the form-1 retry predicate is grid-satisfied — CNR §48).
/// The pin guards the envelope: the faces must NOT degrade to silent empty
/// and must not be answered by a partial admission.
#[test]
fn f139e_go_select_meta_comm_clauses_stay_registered_loud() {
    use ast_sgrep_lang::native_pattern_answerable;
    let bind_clause = "select {\ncase $V := <-$C:\n$$$B\ncase <-$D:\n$$$E\n}";
    assert!(
        !native_pattern_answerable(Language::Go, bind_clause),
        "grid D01: the registered loud class holds (census-unanswerable)"
    );
    let recv_only = "select {\ncase <-$C:\n$$$B\n}";
    assert!(
        !native_pattern_answerable(Language::Go, recv_only),
        "grid D02: registered loud (sg n1 — retry when admitted)"
    );
    let comma_ok = "select {\ncase $V, $OK := <-$C:\n$$$B\n}";
    assert!(
        !native_pattern_answerable(Language::Go, comma_ok),
        "grid D06: sg itself refuses the comma-ok spelling"
    );
}

/// f139f (139A-F5, grid139 E): the java CLASS member-count face binds
/// sg-exactly — `public abstract class $N { $B }` binds the single-member
/// class (E01: N=`A`, B=`void f();` — oracle metaVariables receipt), hops
/// candidate modifier keywords (E02 static-hop n1), refuses empty (E03)
/// and multi-member (E04) bodies and heritage clauses (E07); the plain
/// pattern binds a plain candidate (E08); the interface control holds
/// (E06). Census-loud pre-fix (the 137 form-1 registration's fired
/// predicate).
#[test]
fn f139f_java_class_head_member_count_binds() {
    let src = "public abstract class A {\n    void f();\n}\n";
    let hits = match_pattern(Language::Java, src, "public abstract class $N { $B }").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "grid E01: sg n1: {hits:?}");
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("A"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("void f();")
    );
    let hop = "public static abstract class A {\n    void f();\n}\n";
    let hits = match_pattern(Language::Java, hop, "public abstract class $N { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid E02: keyword hop over `static`: {hits:?}"
    );
    let empty = "public abstract class A {\n}\n";
    let hits = match_pattern(Language::Java, empty, "public abstract class $N { $B }").unwrap();
    assert!(hits.is_empty(), "grid E03: empty body refuses: {hits:?}");
    let multi = "public abstract class A {\n    void f();\n    int g();\n}\n";
    let hits = match_pattern(Language::Java, multi, "public abstract class $N { $B }").unwrap();
    assert!(hits.is_empty(), "grid E04: 2-member body refuses: {hits:?}");
    let heritage = "class B {}\npublic abstract class A extends B {\n    void f();\n}\n";
    let hits = match_pattern(Language::Java, heritage, "public abstract class $N { $B }").unwrap();
    assert!(hits.is_empty(), "grid E07: extends refuses: {hits:?}");
    let plain = "class A {\n    void f();\n}\n";
    let hits = match_pattern(Language::Java, plain, "class $N { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid E08: plain pattern binds: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("A"));
    // Control: the interface face keeps its 137 answer.
    let iface = "public abstract interface I {\n    void f();\n}\n";
    let hits = match_pattern(Language::Java, iface, "public abstract interface $N { $B }").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "grid E06 control: {hits:?}");
}

/// f139g (139A-F6, grid139 F): the accepted-empty census-arm family
/// extends to the `;`-ful sibling spellings — go `break;`/`continue;`/
/// `fallthrough;` and rb `next;`/`redo;`/`retry;` are sg rc1 `[]`
/// (accepted-empty; the workspace gate's root-split refusal made them
/// census-loud pre-fix) while the BARE spellings keep answering per site
/// (controls) and the go/rb `return;` arm holds (137 control).
#[test]
fn f139g_semi_keyword_census_siblings_accepted_empty() {
    use ast_sgrep_lang::native_pattern_answerable;
    for (lang, pat) in [
        (Language::Go, "break;"),
        (Language::Go, "continue;"),
        (Language::Go, "fallthrough;"),
        (Language::Ruby, "next;"),
        (Language::Ruby, "redo;"),
        (Language::Ruby, "retry;"),
    ] {
        assert!(
            native_pattern_answerable(lang, pat),
            "grid F: {pat:?} is census-answerable ({lang:?})"
        );
        assert!(
            match_pattern(lang, "x\n", pat).unwrap().is_empty(),
            "grid F: {pat:?} answers empty on {lang:?}"
        );
    }
    let go_break = match_pattern(
        Language::Go,
        "package main\n\nfunc f() {\n\tfor {\n\t\tbreak;\n\t}\n}\n",
        "break;",
    )
    .unwrap();
    assert!(
        go_break.is_empty(),
        "grid F01 on the real fixture: {go_break:?}"
    );
    // Controls: bare spellings answer per site; the 137 `return;` arm holds.
    assert_eq!(
        lines_of(
            &match_pattern(
                Language::Go,
                "package main\n\nfunc f() {\n\tfor {\n\t\tbreak;\n\t}\n}\n",
                "break"
            )
            .unwrap()
        ),
        vec![5],
        "control F06: bare `break` answers"
    );
    assert!(
        match_pattern(
            Language::Go,
            "package main\n\nfunc f() {\n\tfor {\n\t\treturn;\n\t}\n}\n",
            "return;"
        )
        .unwrap()
        .is_empty(),
        "control F04: the 137 return; arm holds"
    );
}

/// f139h (139B-F2, grid139 H): the py bare-`await` filter drops only the
/// KEYWORD-TOKEN row of an operand-bearing await — a nested bare `await`
/// that error-recovers to an identifier INSIDE the operand survives, exactly
/// where sg answers (`x = await (await)` n1 @ the inner token, H01/H02).
/// The 138 whole-node containment dropped that row (silent-under).
#[test]
fn f139h_nested_bare_await_ident_survives_the_filter() {
    let hits = match_pattern(Language::Python, "x = await (await)\n", "await").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid H01: sg n1 @ the inner ident: {hits:?}"
    );
    let hits = match_pattern(
        Language::Python,
        "async def f():\n    x = await (await)\n",
        "await",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "grid H02: same in a def: {hits:?}"
    );
    // The operand rows stay dropped (the 138 law holds at the new spans).
    let hits = match_pattern(Language::Python, "async def f():\n    await g()\n", "await").unwrap();
    assert!(
        hits.is_empty(),
        "grid S2 control: operand rows still refuse: {hits:?}"
    );
    let hits = match_pattern(
        Language::Python,
        "async def f():\n    x = await g()\n",
        "await",
    )
    .unwrap();
    assert!(hits.is_empty(), "grid S4 control: {hits:?}");
}

/// f139i (FIRED predicate, grid139 I): the php braced-namespace block face
/// binds sg-exactly — `namespace $N { $B }` n1 (N=`App`, B=`function f()
/// {}`), the global `namespace { $B }` n1 (B only), multi-member bodies
/// refuse (I03 sg rc1 `[]`), and the global pattern refuses a named
/// candidate (I04). Census-loud pre-fix; the registered form-1 predicate's
/// retry condition is satisfied and the row is closed of record.
#[test]
fn f139i_php_braced_namespace_binds_block() {
    // Fixtures carry the `<?php` tag (a real php file's spelling —
    // tree-sitter-php roots the file on it; sg binds the same rows on the
    // tagged fixture, oracle grid I06).
    let named = "<?php\nnamespace App {\n    function f() {}\n}\n";
    let hits = match_pattern(Language::Php, named, "namespace $N { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "grid I01: sg n1 @ the block: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("App"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("function f() {}")
    );
    let global = "<?php\nnamespace {\n    function f() {}\n}\n";
    let hits = match_pattern(Language::Php, global, "namespace { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "grid I02: the global block binds: {hits:?}"
    );
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("function f() {}")
    );
    let multi = "<?php\nnamespace App {\n    function f() {}\n    function h() {}\n}\n";
    let hits = match_pattern(Language::Php, multi, "namespace $N { $B }").unwrap();
    assert!(
        hits.is_empty(),
        "grid I03: multi-member body refuses: {hits:?}"
    );
    let hits = match_pattern(Language::Php, named, "namespace { $B }").unwrap();
    assert!(
        hits.is_empty(),
        "grid I04: global pattern vs named candidate: {hits:?}"
    );
}

// ===========================================================================
// PASS 140 (r72 remediation) — grids /tmp/phase140R/grid_pre.json,
// grid_r2.json, grid_r3.json (oracle ast-grep 0.45.2 9585263377c1fc98 vs
// subject at 50db91ee69ffdd2d/5d518773785c18cb). Every assertion is a grid
// receipt; the RED run pins each face against the pre-fix tree.
// ===========================================================================

/// f140a (140A-F1, grid A): sg's nested-head trivia seam — a comment between
/// the enclosing body's `{` and the NESTED-head statement refuses the match
/// (A_v_inner/A_v_block/A_3lvl_c1/A_3lvl_c2 sg rc1 `[]`), while comments
/// before the outer head (A_v_outer), inside the innermost body (A_v_leaf,
/// A_meta_inner), and after the inner close (A_v_after) keep binding.
/// Pre-fix: the subject bound n1 through the seam (wrong hits).
#[test]
fn f140a_cs_nested_head_trivia_seam_is_tight() {
    let tpl = "fixed ($D) { checked { $B } }";
    let seam_line = "class C {\n    void M() {\n        fixed (int* p = arr) {\n            // inner\n            checked { x = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, seam_line, tpl).unwrap();
    assert!(
        hits.is_empty(),
        "grid A_v_inner: comment at the seam refuses: {hits:?}"
    );
    let seam_block = "class C {\n    void M() {\n        fixed (int* p = arr) { /* blk */ checked { x = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, seam_block, tpl).unwrap();
    assert!(
        hits.is_empty(),
        "grid A_v_block: inline comment at the seam refuses: {hits:?}"
    );
    // Controls: comments OUTSIDE the seam keep binding (grid A_v_outer/
    // A_v_leaf/A_v_after/A_meta_inner).
    let outer = "class C {\n    void M() {\n        // outer\n        fixed (int* p = arr) {\n            checked { x = 1; }\n        }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, outer, tpl).unwrap();
    assert_eq!(lines_of(&hits), vec![4], "grid A_v_outer: {hits:?}");
    let leaf = "class C {\n    void M() {\n        fixed (int* p = arr) {\n            checked {\n                // leaf\n                x = 1;\n            }\n        }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, leaf, tpl).unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid A_v_leaf: {hits:?}");
    let after = "class C {\n    void M() {\n        fixed (int* p = arr) {\n            checked { x = 1; }\n            // after\n        }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, after, tpl).unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid A_v_after: {hits:?}");
    let meta_inner = "class C {\n    void M() {\n        checked {\n            // c\n            x = 1;\n        }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, meta_inner, "checked { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![3],
        "grid A_meta_inner: meta body rides through trivia: {hits:?}"
    );
    // Depth-3: a comment at EITHER nested seam refuses (grid A_3lvl_c1/c2).
    let tpl3 = "unsafe { checked { unchecked { $B } } }";
    let c1 = "class C {\n    void M() {\n        unsafe {\n            // c1\n            checked {\n                unchecked { x = 1; }\n            }\n        }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, c1, tpl3).unwrap();
    assert!(hits.is_empty(), "grid A_3lvl_c1: {hits:?}");
    let c2 = "class C {\n    void M() {\n        unsafe {\n            checked {\n                // c2\n                unchecked { x = 1; }\n            }\n        }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, c2, tpl3).unwrap();
    assert!(hits.is_empty(), "grid A_3lvl_c2: {hits:?}");
    // A 1-level pattern rooted INSIDE the seam still binds (grid
    // A_inner_seam_lit) — the unchecked statement sits on line 6 (1-based;
    // the seam comment occupies line 5).
    let hits = match_pattern(Language::CSharp, c2, "unchecked { $B }").unwrap();
    assert_eq!(lines_of(&hits), vec![6], "grid A_inner_seam_lit: {hits:?}");
}

/// f140b (140A-F2, grid B): the operand-less `;`-ful `throw;` spelling is
/// sg ACCEPTED-EMPTY in js/ts/java (B_js_throw_semi_xop,
/// B_ts_throw_semi_xop/xbare, B_ja_throw_semi: sg rc1 `[]` on
/// operand-bearing candidates) — the subject bound operand-bearing
/// `throw e;` rows (js/java) or rc2'd (ts). The bare spelling keeps the
/// kind-lane answer (B_js_bare_xop n1).
#[test]
fn f140b_throw_semi_spelling_is_accepted_empty() {
    let js = "function f() {\n    throw e;\n}\n";
    assert!(
        match_pattern(Language::JavaScript, js, "throw;")
            .unwrap()
            .is_empty(),
        "grid B_js_throw_semi_xop: sg rc1 [] on an operand candidate"
    );
    assert!(
        match_pattern(Language::TypeScript, js, "throw;")
            .unwrap()
            .is_empty(),
        "grid B_ts_throw_semi_xop"
    );
    assert!(
        match_pattern(Language::TypeScript, js, "throw;")
            .unwrap()
            .is_empty(),
        "grid B_ts_throw_semi_xbare"
    );
    let ja = "class C {\n    void m() {\n        throw e;\n    }\n}\n";
    assert!(
        match_pattern(Language::Java, ja, "throw;")
            .unwrap()
            .is_empty(),
        "grid B_ja_throw_semi"
    );
    assert!(
        !match_pattern(Language::JavaScript, js, "throw")
            .unwrap()
            .is_empty(),
        "grid B_js_bare_xop: the bare spelling keeps answering"
    );
    // Census posture: sg accepts the spelling (answerable), never loud.
    assert!(native_pattern_answerable(Language::JavaScript, "throw;"));
    assert!(native_pattern_answerable(Language::TypeScript, "throw;"));
    assert!(native_pattern_answerable(Language::Java, "throw;"));
}

/// f140c (140A-F3, grid C): the cs checked/unchecked EXPRESSION root binds
/// (`checked($E)` × `x = checked(a + b);` n1, E=`a + b` — assignment,
/// return, and argument positions; `unchecked($E)` too). The STATEMENT
/// candidate refuses (C_checkedE_vs_stmt sg rc1 `[]`). Pre-fix: silent
/// wrong-empty everywhere.
#[test]
fn f140c_cs_checked_expression_root_binds() {
    let assign = "class C {\n    void M() {\n        x = checked(a + b);\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, assign, "checked($E)").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid C_checkedE_assign: {hits:?}");
    assert_eq!(hits[0].captures.get("E").map(String::as_str), Some("a + b"));
    let ret = "class C {\n    int M() {\n        return checked(a + b);\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, ret, "checked($E)").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid C_checkedE_ret: {hits:?}");
    let arg = "class C {\n    void M() {\n        M(checked(a + b));\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, arg, "checked($E)").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid C_checkedE_arg: {hits:?}");
    let unch = "class C {\n    void M() {\n        x = unchecked(a + b);\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, unch, "unchecked($E)").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid C_uncheckedE: {hits:?}");
    let stmt = "class C {\n    void M() {\n        checked { x = 1; }\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, stmt, "checked($E)").unwrap();
    assert!(
        hits.is_empty(),
        "grid C_checkedE_vs_stmt: statement candidate refuses: {hits:?}"
    );
}

/// f140d (140A-F4, grid D): java synchronized compositions — nested sync
/// blocks bind per level at the OUTERMOST statement (D_sync_nested n1
/// X=a Y=b B=doIt(); depth-3 too); the synchronized METHOD root binds
/// M/B with modifier hopping (D_sync_method n1; D_sync_static binds a
/// `static synchronized` candidate; D_sync_static_pat refuses a plain
/// candidate); the method-body lane binds M/X/B (D_sync_method_body n1);
/// one-statement laws (D_sync_method_2stmt/R2_ja_sync_method_0stmt rc1 []);
/// the block one-level lane keeps its faces (D_sync_plain_ctl,
/// D_sync_nested_inner_only n2). Pre-fix: rc2 census-loud on every cell.
#[test]
fn f140d_java_synchronized_compositions_bind() {
    let nested = "class C {\n    void m() {\n        synchronized (a) {\n            synchronized (b) { doIt(); }\n        }\n    }\n}\n";
    let hits = match_pattern(
        Language::Java,
        nested,
        "synchronized ($X) { synchronized ($Y) { $B } }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid D_sync_nested: {hits:?}");
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("a"));
    assert_eq!(hits[0].captures.get("Y").map(String::as_str), Some("b"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("doIt();")
    );
    let depth3 = "class C {\n    void m() {\n        synchronized (a) {\n            synchronized (b) {\n                synchronized (c) { doIt(); }\n            }\n        }\n    }\n}\n";
    let hits = match_pattern(
        Language::Java,
        depth3,
        "synchronized ($X) { synchronized ($Y) { synchronized ($Z) { $B } } }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid D_sync_nested_3: {hits:?}");
    assert_eq!(hits[0].captures.get("Z").map(String::as_str), Some("c"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("doIt();")
    );
    let lit = "class C {\n    void m() {\n        synchronized (a) {\n            synchronized (b) { doIt(); }\n        }\n    }\n}\n";
    let hits = match_pattern(
        Language::Java,
        lit,
        "synchronized (a) { synchronized (b) { $B } }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid D_sync_nested_lit: {hits:?}");
    // Method root + modifier hopping.
    let method = "class C {\n    synchronized void m() {\n        doIt();\n    }\n}\n";
    let hits = match_pattern(Language::Java, method, "synchronized void $M() { $B }").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "grid D_sync_method: {hits:?}");
    assert_eq!(hits[0].captures.get("M").map(String::as_str), Some("m"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("doIt();")
    );
    let stat = "class C {\n    static synchronized void m() {\n        doIt();\n    }\n}\n";
    let hits = match_pattern(Language::Java, stat, "synchronized void $M() { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "grid D_sync_static: hopping binds: {hits:?}"
    );
    let hits = match_pattern(
        Language::Java,
        method,
        "static synchronized void $M() { $B }",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "grid D_sync_static_pat: order enforced: {hits:?}"
    );
    let plain = "class C {\n    void m() {\n        doIt();\n    }\n}\n";
    let hits = match_pattern(Language::Java, plain, "synchronized void $M() { $B }").unwrap();
    assert!(hits.is_empty(), "grid D_sync_plain_neg: {hits:?}");
    let two =
        "class C {\n    synchronized void m() {\n        doIt();\n        more();\n    }\n}\n";
    let hits = match_pattern(Language::Java, two, "synchronized void $M() { $B }").unwrap();
    assert!(hits.is_empty(), "grid D_sync_method_2stmt: {hits:?}");
    let zero = "class C {\n    synchronized void m() {}\n}\n";
    let hits = match_pattern(Language::Java, zero, "synchronized void $M() { $B }").unwrap();
    assert!(hits.is_empty(), "grid R2_ja_sync_method_0stmt: {hits:?}");
    // Method-body lane: a plain method whose single statement is a sync block.
    let body = "class C {\n    void m() {\n        synchronized (a) {\n            doIt();\n        }\n    }\n}\n";
    let hits = match_pattern(
        Language::Java,
        body,
        "void $M() { synchronized ($X) { $B } }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "grid D_sync_method_body: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("M").map(String::as_str), Some("m"));
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("a"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("doIt();")
    );
    let hits = match_pattern(
        Language::Java,
        plain,
        "void $M() { synchronized ($X) { $B } }",
    )
    .unwrap();
    assert!(hits.is_empty(), "grid D_sync_method_body_neg: {hits:?}");
    // 139 one-level lane keeps its faces.
    let hits = match_pattern(Language::Java, nested, "synchronized ($X) { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![3, 4],
        "grid D_sync_plain_ctl + inner-only: {hits:?}"
    );
}

/// f140e (140A-F5, grid E/R2/R3): the import/directive root family binds
/// sg-exactly — py `import $X` (first child binds; dotted/aliased/multi),
/// `import $A, $B`, `from $M import $X`; java `import $X;` (per
/// declaration; static/star spellings demarcated; negatives refuse);
/// rs `use $X;` (whole-argument text incl. braces; alias form); php
/// `use $X;` (incl. `use function`); cs `using $N;` (directive kind only;
/// alias and using-STATEMENT candidates refuse; the LITERAL `using
/// System;` face answers exactly one row — R2_cs_using_lit over-served n2
/// pre-fix); go `import $X;` is accepted-empty (E_go_importblock).
#[test]
fn f140e_import_directive_roots_bind() {
    let hits = match_pattern(Language::Python, "import os\n", "import $X").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "grid E_py_import1: {hits:?}");
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("os"));
    let hits = match_pattern(Language::Python, "import os.path\n", "import $X").unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("os.path"),
        "grid E_py_import_dotted"
    );
    let hits = match_pattern(Language::Python, "import os, sys\n", "import $X").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid E_py_import_two: first child binds: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("os"));
    let hits = match_pattern(Language::Python, "import os, sys\n", "import $A, $B").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "grid E_py_importAB: {hits:?}");
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("os"));
    assert_eq!(hits[0].captures.get("B").map(String::as_str), Some("sys"));
    let hits = match_pattern(Language::Python, "import os as o\n", "import $X as $Y").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "grid E_py_import_alias: {hits:?}");
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("os"));
    assert_eq!(hits[0].captures.get("Y").map(String::as_str), Some("o"));
    let hits = match_pattern(Language::Python, "import os as o\n", "import $X").unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("os as o"),
        "grid R2_py_importX_alias_cand: the aliased child's whole text binds"
    );
    let hits = match_pattern(
        Language::Python,
        "from os import path\n",
        "from $M import $X",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "grid E_py_from: {hits:?}");
    assert_eq!(hits[0].captures.get("M").map(String::as_str), Some("os"));
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("path"));
    let hits = match_pattern(
        Language::Python,
        "from os import path, join\n",
        "from $M import $X",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("path"),
        "grid R2_py_from_multi: first name binds"
    );
    // java
    let hits = match_pattern(Language::Java, "import java.util.List;\n", "import $X;").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "grid E_ja_import1: {hits:?}");
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("java.util.List")
    );
    let two = "import java.util.List;\nimport java.io.File;\n";
    let hits = match_pattern(Language::Java, two, "import $X;").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1, 2],
        "grid E_ja_import_two: {hits:?}"
    );
    let hits = match_pattern(
        Language::Java,
        "import static java.lang.Math.abs;\n",
        "import static $X;",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("java.lang.Math.abs"),
        "grid E_ja_import_static"
    );
    let hits = match_pattern(
        Language::Java,
        "import static java.lang.Math.abs;\n",
        "import $X;",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "grid R2_ja_import_plain_pat_static_cand: {hits:?}"
    );
    let hits = match_pattern(Language::Java, "import java.util.*;\n", "import $X.*;").unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("java.util"),
        "grid E_ja_import_star"
    );
    let hits = match_pattern(Language::Java, "import java.util.List;\n", "import $X.*;").unwrap();
    assert!(
        hits.is_empty(),
        "grid R2_ja_import_star_pat_plain_cand: {hits:?}"
    );
    let hits = match_pattern(Language::Java, "import java.util.*;\n", "import $X;").unwrap();
    assert!(hits.is_empty(), "grid R3_ja_import_plain_star: {hits:?}");
    // rust
    let hits = match_pattern(Language::Rust, "use std::fmt;\n", "use $X;").unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("std::fmt"),
        "grid E_rs_use1"
    );
    let hits = match_pattern(
        Language::Rust,
        "use std::collections::{HashMap, HashSet};\n",
        "use $X;",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("std::collections::{HashMap, HashSet}"),
        "grid E_rs_use_braces"
    );
    let hits = match_pattern(Language::Rust, "use std::fmt as f;\n", "use $X as $Y;").unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("std::fmt"),
        "grid E_rs_use_alias"
    );
    assert_eq!(hits[0].captures.get("Y").map(String::as_str), Some("f"));
    // php
    let hits = match_pattern(Language::Php, "<?php\nuse Foo\\Bar;\n", "use $X;").unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("Foo\\Bar"),
        "grid E_php_use1"
    );
    let hits = match_pattern(Language::Php, "<?php\nuse function foo;\n", "use $X;").unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("function foo"),
        "grid R3_php_use_function"
    );
    // csharp
    let hits = match_pattern(Language::CSharp, "using System;\n", "using $N;").unwrap();
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("System"),
        "grid E_cs_using1"
    );
    let hits = match_pattern(Language::CSharp, "using S = System.Text;\n", "using $N;").unwrap();
    assert!(
        hits.is_empty(),
        "grid E_cs_using_alias: alias candidate refuses: {hits:?}"
    );
    let hits = match_pattern(
        Language::CSharp,
        "class C {\n    void M() {\n        using (var d = Open()) { x = 1; }\n    }\n}\n",
        "using $N;",
    )
    .unwrap();
    assert!(hits.is_empty(), "grid R3_cs_using_stmt_neg: {hits:?}");
    let hits = match_pattern(Language::CSharp, "using System;\n", "using System;").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid R2_cs_using_lit: the literal face answers ONE row (pre-fix n2)"
    );
    // go `import $X;` is accepted-empty.
    let go = "package main\n\nimport (\n    \"fmt\"\n)\n\nfunc main() { fmt.Println() }\n";
    assert!(
        match_pattern(Language::Go, go, "import $X;")
            .unwrap()
            .is_empty(),
        "grid E_go_importblock: sg rc1 []"
    );
    assert!(native_pattern_answerable(Language::Go, "import $X;"));
}

/// f140f (140A-F6/F7, grid F/R2): the remaining statement roots — kt
/// typealias (N/T), kt for (X/C/B, block-body law: 2-stmt bodies bind the
/// joined inner text, brace-less refuses), swift for-in (with and without
/// the where clause; where-pattern refuses a plain candidate), rs let-else
/// (P/E/B; 2-stmt else and no-else refuse), c goto (per site). go bare
/// `fallthrough` is the 140B-F4 control: the literal lane already agrees.
#[test]
fn f140f_remaining_statement_roots_bind() {
    let hits = match_pattern(
        Language::Kotlin,
        "typealias Foo = Bar\n",
        "typealias $N = $T",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "grid F_kt_typealias: {hits:?}");
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("Foo"));
    assert_eq!(hits[0].captures.get("T").map(String::as_str), Some("Bar"));
    let kt = "fun main() {\n    val xs = listOf(1)\n    for (x in xs) {\n        g(x)\n    }\n}\n";
    let hits = match_pattern(Language::Kotlin, kt, "for ($X in $C) { $B }").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid F_kt_for: {hits:?}");
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("x"));
    assert_eq!(hits[0].captures.get("C").map(String::as_str), Some("xs"));
    assert_eq!(hits[0].captures.get("B").map(String::as_str), Some("g(x)"));
    let kt2 = "fun main() {\n    val xs = listOf(1)\n    for (x in xs) {\n        g(x)\n        h(x)\n    }\n}\n";
    let hits = match_pattern(Language::Kotlin, kt2, "for ($X in $C) { $B }").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid R2_kt_for_2stmt: {hits:?}");
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("g(x)\n        h(x)")
    );
    let ktnb = "fun main() {\n    val xs = listOf(1)\n    for (x in xs) g(x)\n}\n";
    let hits = match_pattern(Language::Kotlin, ktnb, "for ($X in $C) { $B }").unwrap();
    assert!(hits.is_empty(), "grid R2_kt_for_nobrace: {hits:?}");
    // swift
    let sw =
        "func m() {\n    let xs = [1]\n    for x in xs where x > 1 {\n        g(x)\n    }\n}\n";
    let hits = match_pattern(Language::Swift, sw, "for $X in $C where $W { $B }").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid F_sw_for_where: {hits:?}");
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("x"));
    assert_eq!(hits[0].captures.get("C").map(String::as_str), Some("xs"));
    assert_eq!(hits[0].captures.get("W").map(String::as_str), Some("x > 1"));
    let swp = "func m() {\n    let xs = [1]\n    for x in xs {\n        g(x)\n    }\n}\n";
    let hits = match_pattern(Language::Swift, swp, "for $X in $C { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![3],
        "grid R2_sw_plain_pat_plain_src: {hits:?}"
    );
    let hits = match_pattern(Language::Swift, swp, "for $X in $C where $W { $B }").unwrap();
    assert!(hits.is_empty(), "grid R2_sw_where_pat_plain_src: {hits:?}");
    let hits = match_pattern(Language::Swift, sw, "for $X in $C { $B }").unwrap();
    assert!(
        hits.is_empty(),
        "grid F_sw_for_plain: where candidate vs plain pattern: {hits:?}"
    );
    // rust let-else
    let rs = "fn f(y: Option<i32>) {\n    let Some(x) = y else {\n        return;\n    };\n}\n";
    let hits = match_pattern(Language::Rust, rs, "let $P = $E else { $B };").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "grid F_rs_letelse: {hits:?}");
    assert_eq!(
        hits[0].captures.get("P").map(String::as_str),
        Some("Some(x)")
    );
    assert_eq!(hits[0].captures.get("E").map(String::as_str), Some("y"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("return;")
    );
    let rs2 = "fn f(y: Option<i32>) {\n    let Some(x) = y else {\n        a();\n        return;\n    };\n}\n";
    let hits = match_pattern(Language::Rust, rs2, "let $P = $E else { $B };").unwrap();
    assert!(hits.is_empty(), "grid R2_rs_letelse_2stmt: {hits:?}");
    let rsn = "fn f(y: Option<i32>) {\n    let Some(x) = y;\n}\n";
    let hits = match_pattern(Language::Rust, rsn, "let $P = $E else { $B };").unwrap();
    assert!(hits.is_empty(), "grid R2_rs_letelse_noelse: {hits:?}");
    // c goto
    let c = "void f(void) {\n    if (a) goto end;\n    goto end;\nend:\n    return;\n}\n";
    let hits = match_pattern(Language::C, c, "goto $L;").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2, 3],
        "grid R2_c_goto_2sites: {hits:?}"
    );
    assert!(hits
        .iter()
        .all(|h| h.captures.get("L").map(String::as_str) == Some("end")));
    // 140B-F4 control: go bare `fallthrough` already answers (receipt of record).
    let go = "package main\n\nfunc f(x int) {\n\tswitch x {\n\tcase 1:\n\t\tg()\n\t\tfallthrough\n\tcase 2:\n\t\tg()\n\t}\n}\nfunc g() {}\n";
    let hits = match_pattern(Language::Go, go, "fallthrough").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![7],
        "grid F_go_fallthrough_bare: literal-lane receipt"
    );
}

/// f140g (140A-F8, grid G/R2): the py del lane boundaries — a CALL operand
/// binds head+argument atoms (`del $F($A)` × `del f(1)` F=f A=1; empty-arg
/// and 2-operand candidates refuse; literal heads byte-match); the
/// PAREN-LEAD mixed list binds per element (`del ($X), $Y` × `del (a), b`
/// X=a Y=b; the paren meta binds the INNER text even around a structural
/// element; a paren-free candidate refuses; extra candidate elements
/// absorb as a prefix). The 139 G01/G03/G07 guard cells keep holding.
#[test]
fn f140g_py_del_call_and_paren_lead_bind() {
    let src = "a = 1\nb = 2\ndel f(1)\n";
    let hits = match_pattern(Language::Python, src, "del $F($A)").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid G_del_call_meta: {hits:?}");
    assert_eq!(hits[0].captures.get("F").map(String::as_str), Some("f"));
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("1"));
    let hits = match_pattern(Language::Python, src, "del f($A)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![3],
        "grid G_del_call_fmeta_src: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "a = 1\nb = 2\ndel g(1)\n", "del f($A)").unwrap();
    assert!(
        hits.is_empty(),
        "grid G_del_call_conca_neg: literal head byte-matches: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "a = 1\nb = 2\ndel f(x.y)\n", "del f($A)").unwrap();
    assert_eq!(
        hits[0].captures.get("A").map(String::as_str),
        Some("x.y"),
        "grid G_del_call_meta_src"
    );
    let hits = match_pattern(
        Language::Python,
        "a = 1\nb = 2\ndel f(1), g(2)\n",
        "del $F($A)",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "grid G_del_call_2op: exact count: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "a = 1\ndel f()\n", "del $F($A)").unwrap();
    assert!(hits.is_empty(), "grid R2_del_call_emptyargs: {hits:?}");
    // paren-lead mixed lists
    let pm = "a = 1\nb = 2\ndel (a), b\n";
    let hits = match_pattern(Language::Python, pm, "del ($X), $Y").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid G_del_pm2: {hits:?}");
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("a"));
    assert_eq!(hits[0].captures.get("Y").map(String::as_str), Some("b"));
    let pm3 = "a = 1\nb = 2\nc = 3\ndel (a), b, c\n";
    let hits = match_pattern(Language::Python, pm3, "del ($X), $Y, $Z").unwrap();
    assert_eq!(lines_of(&hits), vec![4], "grid G_del_pm3: {hits:?}");
    assert_eq!(hits[0].captures.get("Z").map(String::as_str), Some("c"));
    let hits = match_pattern(Language::Python, "a = 1\nb = 2\ndel a, b\n", "del ($X), $Y").unwrap();
    assert!(
        hits.is_empty(),
        "grid G_del_pm2_free_cand: paren demanded: {hits:?}"
    );
    let pms = "d = {}\ny = 2\ndel (d[k]), y\n";
    let hits = match_pattern(Language::Python, pms, "del ($X), $Y").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid G_del_pm2_struct: {hits:?}");
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("d[k]"));
    let hits = match_pattern(Language::Python, pms, "del ($O[$K]), $Y").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![3],
        "grid G_del_pm_structmix: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("O").map(String::as_str), Some("d"));
    assert_eq!(hits[0].captures.get("K").map(String::as_str), Some("k"));
    assert_eq!(hits[0].captures.get("Y").map(String::as_str), Some("y"));
    let bp = "a = 1\nb = 2\ndel (a), (b)\n";
    let hits = match_pattern(Language::Python, bp, "del ($X), ($Y)").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid G_del_bothparen: {hits:?}");
    let extra = "a = 1\nb = 2\nc = 3\ndel (a), b, c\n";
    let hits = match_pattern(Language::Python, extra, "del ($X), $Y").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![4],
        "grid R2_del_pm2_extra_cand: prefix absorb: {hits:?}"
    );
    // 139 guard cells keep holding.
    let free = "a = 1\nb = 2\ndel a, b\n";
    assert!(
        match_pattern(Language::Python, free, "del ($X, $Y)")
            .unwrap()
            .is_empty(),
        "grid G01 held"
    );
    assert!(
        match_pattern(Language::Python, free, "del ($X)")
            .unwrap()
            .is_empty(),
        "grid G03 held"
    );
    assert!(
        match_pattern(Language::Python, pm, "del (a)")
            .unwrap()
            .is_empty(),
        "grid G07-twin held: single paren vs 2-op candidate"
    );
    let hits = match_pattern(Language::Python, "a = 1\nb = 2\ndel (a)\n", "del (a)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![3],
        "grid G07 held: concrete paren binds"
    );
}

/// f140h (140A-F9, grid H): the accepted-empty `;`-ful spellings join the
/// 137/139 census arm — py pass/yield/global/import/raise/del/assert;,
/// rb break;, go goto-forms (incl. the `$`-carrying `goto $L;`). sg rc1
/// `[]` on every candidate; the census must be ANSWERABLE (pre-fix rc2
/// loud) and the walk empty.
#[test]
fn f140h_semi_keyword_accepted_empty_siblings() {
    let pykw = "def m():\n    pass;\n    yield;\n    global x;\n    import os;\n    raise;\n    del x;\n    assert x;\n    return;\n";
    for pattern in [
        "pass;", "yield;", "global;", "import;", "raise;", "del;", "assert;",
    ] {
        assert!(
            native_pattern_answerable(Language::Python, pattern),
            "census answerable: py {pattern}"
        );
        assert!(
            match_pattern(Language::Python, pykw, pattern)
                .unwrap()
                .is_empty(),
            "walk empty: py {pattern}"
        );
    }
    let rb = "loop do\n    break\nend\n";
    assert!(native_pattern_answerable(Language::Ruby, "break;"));
    assert!(
        match_pattern(Language::Ruby, rb, "break;")
            .unwrap()
            .is_empty(),
        "grid H_rb_break"
    );
    let rb2 = "loop do\n    break;\nend\n";
    assert!(
        match_pattern(Language::Ruby, rb2, "break;")
            .unwrap()
            .is_empty(),
        "grid H_rb_breaksemi: binds nothing even on the semi source"
    );
    let go = "package main\n\nfunc f() {\n    goto end\nend:\n    return\n}\n";
    assert!(native_pattern_answerable(Language::Go, "goto $L;"));
    assert!(native_pattern_answerable(Language::Go, "goto end;"));
    assert!(
        match_pattern(Language::Go, go, "goto $L;")
            .unwrap()
            .is_empty(),
        "grid H_go_goto_meta: the meta spelling is refused, not bound"
    );
    assert!(
        match_pattern(Language::Go, go, "goto end;")
            .unwrap()
            .is_empty(),
        "grid H_go_goto_plain"
    );
    // 139 siblings keep their census posture.
    assert!(native_pattern_answerable(Language::Go, "break;"));
    assert!(native_pattern_answerable(Language::Ruby, "retry;"));
}

/// f140i (140B-F2, grids I/R2/R3): the `$$`/`$$$` multi-prefix law — sg
/// binds `$$NAME` in the SINGLE namespace (PASS 75a) and `$$$NAME` in the
/// MULTI namespace. Gridded receipts: ja resource `$$$X` (n1, multi),
/// ja body `$$$B` (n1), php name `$$$N` (n1, multi), php BODY `$$B`/`$$$B`
/// (sg rc1 `[]` — the accepted-empty posture), java class heads `$$N`
/// (single) and `$$$N` (multi), py del atom `d[$$$K]` (multi), `del $$$X`
/// (multi).
#[test]
fn f140i_multi_prefix_atoms_follow_sg_namespaces() {
    let ja = "class C {\n    void m() {\n        synchronized (lock) {\n            doIt();\n        }\n    }\n}\n";
    let hits = match_pattern(Language::Java, ja, "synchronized ($$$X) { $B }").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "grid I_ja_dollarres3: {hits:?}");
    assert_eq!(
        hits[0].captures.get("$$$X").map(String::as_str),
        Some("lock"),
        "the multi-namespace key binds"
    );
    let hits = match_pattern(Language::Java, ja, "synchronized ($X) { $$$B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![3],
        "grid R3_ja_body_dollar3: {hits:?}"
    );
    let hits = match_pattern(Language::Java, ja, "synchronized ($$X) { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![3],
        "grid I_ja_dollarres held: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("lock"));
    // php: name multi-binds; the BODY double/multi-prefix is accepted-empty.
    let php = "<?php\nnamespace App {\n    function f() {}\n}\n";
    let hits = match_pattern(Language::Php, php, "namespace $$$N { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "grid R3_php_name_dollar3: {hits:?}"
    );
    assert_eq!(
        hits[0].captures.get("$$$N").map(String::as_str),
        Some("App")
    );
    let hits = match_pattern(Language::Php, php, "namespace { $$B }").unwrap();
    assert!(
        hits.is_empty(),
        "grid I_php_dollarbody: body double-prefix accepted-empty: {hits:?}"
    );
    let hits = match_pattern(Language::Php, php, "namespace { $$$B }").unwrap();
    assert!(hits.is_empty(), "grid I_php_dollarbody3: {hits:?}");
    // java class heads.
    let cls = "class C {\n    synchronized void m() {\n        doIt();\n    }\n}\n";
    let hits = match_pattern(Language::Java, cls, "class $$N { $B }").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "grid I_ja_class_dollar: {hits:?}");
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("C"));
    let hits = match_pattern(Language::Java, cls, "class $$$N { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "grid R2_ja_class_dollar3: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("$$$N").map(String::as_str), Some("C"));
    // py del atoms.
    let py = "d = {1: 2}\ndel d[1]\n";
    let hits = match_pattern(Language::Python, py, "del d[$$$K]").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "grid I_py_dollaratom: {hits:?}");
    assert_eq!(hits[0].captures.get("$$$K").map(String::as_str), Some("1"));
    let hits = match_pattern(Language::Python, py, "del d[$$K]").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "grid I_py_dollaratom2 held: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("K").map(String::as_str), Some("1"));
    let pyw = "x = 1\ndel x\n";
    let hits = match_pattern(Language::Python, pyw, "del $$$X").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "grid R3_py_del_whole_d3: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("$$$X").map(String::as_str), Some("x"));
    // The LIST-level paren wrap over a multi-meta slot binds NOTHING —
    // sg ACCEPTED-empty (R3_py_del_paren_d3 rc1 `[]`).
    let hits = match_pattern(Language::Python, pyw, "del ($$$X)").unwrap();
    assert!(hits.is_empty(), "grid R3_py_del_paren_d3: {hits:?}");
}

/// f140j (140B-F5, grid J): the php braced namespace with a LITERAL name
/// binds (`namespace App { $B }` n1, B=the member; a different literal
/// name refuses; the global pattern vs a named candidate still refuses).
#[test]
fn f140j_php_braced_namespace_literal_name_binds() {
    let src = "<?php\nnamespace App {\n    function f() {}\n}\n";
    let hits = match_pattern(Language::Php, src, "namespace App { $B }").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "grid J_php_litname: {hits:?}");
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("function f() {}")
    );
    let hits = match_pattern(Language::Php, src, "namespace Other { $B }").unwrap();
    assert!(hits.is_empty(), "grid J_php_litname_neg: {hits:?}");
    let hits = match_pattern(Language::Php, src, "namespace { $B }").unwrap();
    assert!(hits.is_empty(), "grid J_php_lit_global held: {hits:?}");
    let hits = match_pattern(Language::Php, src, "namespace $N { $B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "grid I01 held: the meta-name face keeps binding"
    );
}

// ===========================================================================
// PASS 141 (r73 remediation) — grids /tmp/phase141R/grid_A_E_cs_using.json,
// grid_B_C_throw_php.json, grid_D_G_seam_pyimport.json,
// grid_F_bundles.json, grid_R2_controls.json (oracle ast-grep 0.45.2
// 9585263377c1fc98 vs subject at fc523354b2397d09/672cc5248bfe45d9, the
// exact `--pattern` lane of record). Every assertion is a grid receipt;
// the RED run pins each face against the pre-fix tree.
// ===========================================================================

/// f141a (141A-F1/F5 + F6 cs rows 4-6, grids F1_*/F5_*/A_* / R2_cs_*):
/// the cs using-directive family demarcates sg-exactly. `global` is a
/// DEMAND-ONLY leading keyword (a pattern without it binds global
/// candidates too — F1_a_global sg n1; a pattern WITH it refuses
/// non-global candidates); the `static`/`unsafe` keyword run must EQUAL
/// the candidate's run (F1_a_static / F1_a_unsafe sg rc1 `[]` on the
/// plain pattern; R2_cs_static_pat_x_unsafe_cand n0); the alias face
/// splits name/type (A=`M`, T=`System.Math`); literal faces answer the
/// single directive node (F5_global_lit sg n1 — the double emit is
/// pre-fix).
#[test]
fn f141a_cs_using_directive_faces_demarcate() {
    let plain = "using System;\n";
    let staticc = "using static System.Math;\n";
    let global = "global using System.IO;\n";
    let unsafee = "using unsafe Foo.Bar;\n";
    // Plain pattern: binds plain AND global candidates, refuses the
    // static/unsafe runs.
    let hits = match_pattern(Language::CSharp, plain, "using $N;").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "F1_a_plain: {hits:?}");
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("System")
    );
    let hits = match_pattern(Language::CSharp, staticc, "using $N;").unwrap();
    assert!(
        hits.is_empty(),
        "F1_a_static: the static run refuses: {hits:?}"
    );
    let hits = match_pattern(Language::CSharp, unsafee, "using $N;").unwrap();
    assert!(
        hits.is_empty(),
        "F1_a_unsafe: the unsafe run refuses: {hits:?}"
    );
    let hits = match_pattern(Language::CSharp, global, "using $N;").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "F1_a_global: global candidate binds: {hits:?}"
    );
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("System.IO")
    );
    // Static pattern: demands the `static` run (never unsafe), global
    // skippable.
    let hits = match_pattern(Language::CSharp, staticc, "using static $N;").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "F6_cs_using_static_pat: {hits:?}");
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("System.Math")
    );
    let hits = match_pattern(Language::CSharp, plain, "using static $N;").unwrap();
    assert!(hits.is_empty(), "F6_cs_using_static_pat_x_plain: {hits:?}");
    let gstat = "global using static System.Text.Encoding;\n";
    let hits = match_pattern(Language::CSharp, gstat, "using static $N;").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "F1_all6_staticpat global face: {hits:?}"
    );
    let hits = match_pattern(Language::CSharp, unsafee, "using static $N;").unwrap();
    assert!(hits.is_empty(), "R2_cs_static_pat_x_unsafe_cand: {hits:?}");
    let hits = match_pattern(Language::CSharp, gstat, "global using static $N;").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "R2_cs_global_static_pat: {hits:?}"
    );
    // Unsafe pattern face (R2_cs_unsafe_pat).
    let hits = match_pattern(Language::CSharp, unsafee, "using unsafe $N;").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "R2_cs_unsafe_pat: {hits:?}");
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("Foo.Bar")
    );
    // Alias face: A/T split, global skippable.
    let alias = "using M = System.Math;\n";
    let hits = match_pattern(Language::CSharp, alias, "using $A = $T;").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "A_cs_using_alias_pat: {hits:?}");
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("M"));
    assert_eq!(
        hits[0].captures.get("T").map(String::as_str),
        Some("System.Math")
    );
    let galias = "global using M = System.Math;\n";
    let hits = match_pattern(Language::CSharp, galias, "using $A = $T;").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "R2_cs_global_alias_cand: {hits:?}"
    );
    let hits = match_pattern(Language::CSharp, plain, "using $A = $T;").unwrap();
    assert!(hits.is_empty(), "A_cs_using_alias_pat_x_plain: {hits:?}");
    // Global pattern: demand-only global + refusal of the static run.
    let hits = match_pattern(Language::CSharp, global, "global using $N;").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "F6_cs_global_pat: {hits:?}");
    let hits = match_pattern(Language::CSharp, plain, "global using $N;").unwrap();
    assert!(hits.is_empty(), "F6_cs_global_pat_x_plain: {hits:?}");
    let hits = match_pattern(Language::CSharp, gstat, "global using $N;").unwrap();
    assert!(hits.is_empty(), "F6_cs_global_pat_x_static: {hits:?}");
    // Literal faces answer the SINGLE directive node (the pre-fix double
    // emit spanned the trailing newline).
    let hits = match_pattern(
        Language::CSharp,
        "global using System;\n",
        "global using System;",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "F5_global_lit: exactly one hit: {hits:?}"
    );
    let hits = match_pattern(Language::CSharp, plain, "using System;").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "A_using_plain_lit_ctrl held: {hits:?}"
    );
    let hits = match_pattern(Language::CSharp, "using System;\n", "global using System;").unwrap();
    assert!(hits.is_empty(), "F5_global_lit_x_plain: {hits:?}");
    // Census posture: every new face is answerable (never loud).
    for pattern in [
        "using static $N;",
        "using $A = $T;",
        "global using $N;",
        "global using System;",
    ] {
        assert!(
            native_pattern_answerable(Language::CSharp, pattern),
            "census: {pattern} must be answerable"
        );
    }
}

/// f141b (141A-F2, grid F2_*): the cs `throw;` spelling family gates on the
/// CANDIDATE — C# rethrow (`throw;`, no operand) is a valid throw_statement
/// and BINDS, while operand-bearing candidates refuse (sg rc1 `[]`). The
/// bare `throw` spelling keeps the kind-lane answer on both shapes.
#[test]
fn f141b_cs_throw_semi_operand_gate() {
    let bare = "class C {\n    void M() {\n        try { } catch (System.Exception) {\n            throw;\n        }\n    }\n}\n";
    let operand = "class C {\n    void M() {\n        throw e;\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, bare, "throw;").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![4],
        "F2_bare_cand: rethrow binds: {hits:?}"
    );
    let hits = match_pattern(Language::CSharp, operand, "throw;").unwrap();
    assert!(
        hits.is_empty(),
        "F2_operand_cand: operand candidate refuses: {hits:?}"
    );
    let mixed = "class C {\n    void M() {\n        try { } catch (System.Exception) {\n            throw;\n        }\n        throw e;\n    }\n}\n";
    let hits = match_pattern(Language::CSharp, mixed, "throw;").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![4],
        "F2_mixed: exactly the bare site: {hits:?}"
    );
    let hits = match_pattern(Language::CSharp, operand, "throw ;").unwrap();
    assert!(hits.is_empty(), "F2_throw_semi_ws operand face: {hits:?}");
    // Operand patterns keep answering (unchanged faces).
    let hits = match_pattern(Language::CSharp, operand, "throw $X;").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "F2_throwX_operand held: {hits:?}");
    let hits = match_pattern(Language::CSharp, bare, "throw").unwrap();
    assert!(
        !hits.is_empty(),
        "F2_throwbare_bare held: the bare spelling keeps the kind lane"
    );
    // Census posture: answerable, never loud.
    assert!(native_pattern_answerable(Language::CSharp, "throw;"));
}

/// f141c (141A-F3, grid F3_*): sg's php `$$B`/`$$$B` namespace-body law —
/// `$$B` follows the ONE-member `$B` law (binds any single member), and
/// `$$$B` binds the MULTI list at ANY member count (0 members → the empty
/// list). The 140 RefusedEmpty reading is refuted of record. The subject's
/// MULTI capture encoding joins the member texts with '\n' (sg emits a
/// JSON array — encoding difference of record).
#[test]
fn f141c_php_dollar_body_single_member_law() {
    let one_fn = "<?php\nnamespace App {\n    function f() {}\n}\n";
    let hits = match_pattern(Language::Php, one_fn, "namespace App { $$B }").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "F3_app_d2_1mem: {hits:?}");
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("function f() {}")
    );
    let echo = "<?php\nnamespace App {\n    echo 1;\n}\n";
    let hits = match_pattern(Language::Php, echo, "namespace App { $$B }").unwrap();
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("echo 1;"),
        "F3_app_d2_echo"
    );
    let assign = "<?php\nnamespace App {\n    $x = 1;\n}\n";
    let hits = match_pattern(Language::Php, assign, "namespace App { $$B }").unwrap();
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("$x = 1;"),
        "F3_app_d2_assign"
    );
    let two = "<?php\nnamespace App {\n    function f() {}\n    function g() {}\n}\n";
    let hits = match_pattern(Language::Php, two, "namespace App { $$B }").unwrap();
    assert!(
        hits.is_empty(),
        "F3_app_d2_2mem: two members refuse: {hits:?}"
    );
    let zero = "<?php\nnamespace App {\n}\n";
    let hits = match_pattern(Language::Php, zero, "namespace App { $$B }").unwrap();
    assert!(hits.is_empty(), "F3_app_d2_0mem: {hits:?}");
    let hits = match_pattern(Language::Php, one_fn, "namespace $N { $$B }").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "F3_meta_d2_1mem: {hits:?}");
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("App"));
    let global_one = "<?php\nnamespace {\n    function f() {}\n}\n";
    let hits = match_pattern(Language::Php, global_one, "namespace { $$B }").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "F3_global_d2_1mem: {hits:?}");
    // `$$$B`: the MULTI list at any member count.
    let hits = match_pattern(Language::Php, one_fn, "namespace App { $$$B }").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "F3_app_d3_1mem: {hits:?}");
    assert_eq!(
        hits[0].captures.get("$$$B").map(String::as_str),
        Some("function f() {}"),
        "F3_app_d3_1mem multi capture"
    );
    let hits = match_pattern(Language::Php, two, "namespace App { $$$B }").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "F3_app_d3_2mem: {hits:?}");
    assert_eq!(
        hits[0].captures.get("$$$B").map(String::as_str),
        Some("function f() {}\nfunction g() {}"),
        "F3_app_d3_2mem multi capture joins members"
    );
    let hits = match_pattern(Language::Php, zero, "namespace App { $$$B }").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "F3_app_d3_0mem: binds the empty list: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("$$$B").map(String::as_str), Some(""));
    let mixed = "<?php\nnamespace App {\n    function f() {}\n    echo 1;\n}\n";
    let hits = match_pattern(Language::Php, mixed, "namespace App { $$$B }").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "F3_d3_mixed2: {hits:?}");
    assert_eq!(
        hits[0].captures.get("$$$B").map(String::as_str),
        Some("function f() {}\necho 1;")
    );
}

/// f141d (141A-F4 + 141B-F1, grid D_*): the cs nested-head seam trivia
/// class — sg admits ASCII whitespace plus the grammar's explicit extras
/// members U+00A0 (NBSP), U+FEFF, and U+3000; U+0085/U+2028/U+202F refuse;
/// a comment at the seam still refuses.
#[test]
fn f141d_cs_seam_trivia_class() {
    let tpl = "fixed ($D) { checked { $B } }";
    let seam_case = |seam: &str| {
        format!(
            "class C {{\n    void M() {{\n        fixed (int* p = arr) {{{seam} checked {{ x = 1; }}\n    }}\n}}\n"
        )
    };
    // Admitted seam trivia.
    for (cell, seam) in [
        ("D_d_vtab", "\u{000b} "),
        ("D_d_nbsp", "\u{00a0} "),
        ("D_d_u3000", "\u{3000} "),
        ("D_d_feff", "\u{feff} "),
        ("D_d_tab_ctrl", "\t "),
        ("D_d_crlf_ctrl", "\r\n "),
        ("D_d_ff_ctrl", "\u{000c} "),
    ] {
        let hits = match_pattern(Language::CSharp, &seam_case(seam), tpl).unwrap();
        assert_eq!(
            lines_of(&hits),
            vec![3],
            "{cell}: sg n1 binds through the seam: {hits:?}"
        );
        assert_eq!(
            hits[0].captures.get("B").map(String::as_str),
            Some("x = 1;"),
            "{cell}"
        );
    }
    // Refused seam trivia (outside the class of record).
    for (cell, seam) in [
        ("D_d_nel", "\u{0085} "),
        ("D_d_u2028", "\u{2028} "),
        ("D_d_u202f", "\u{202f} "),
        ("D_ctrl_comment", " /* c */ "),
    ] {
        let hits = match_pattern(Language::CSharp, &seam_case(seam), tpl).unwrap();
        assert!(hits.is_empty(), "{cell}: sg rc1 [] refuses: {hits:?}");
    }
}

/// f141e (141B-F2 + F6 py rows 1-3, grids G_*/F6_from_*/R2_from_*): the py
/// import name-list family — N-slot plain templates absorb trailing
/// candidate names (the zip law generalizes; a 2-name candidate under a
/// 3-slot template refuses), from-import carries comma name slots (meta or
/// literal; paren faces demarcate BOTH ways against the candidate's
/// parentheses; the `*` face binds the module only and a name-slot pattern
/// binds Y=`*`), and the alias face binds the FIRST child when it is an
/// aliased_import.
#[test]
fn f141e_py_import_name_list_faces() {
    let three = "import os, sys, json\n";
    let hits = match_pattern(Language::Python, three, "import $A, $B, $C").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "G_abc_x3name: {hits:?}");
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("os"));
    assert_eq!(hits[0].captures.get("B").map(String::as_str), Some("sys"));
    assert_eq!(hits[0].captures.get("C").map(String::as_str), Some("json"));
    let hits = match_pattern(Language::Python, "import os, sys\n", "import $A, $B, $C").unwrap();
    assert!(
        hits.is_empty(),
        "G_abc_x2name: fewer names refuse: {hits:?}"
    );
    // from-import comma names.
    let from3 = "from os import path, sep, curdir\n";
    let hits = match_pattern(Language::Python, from3, "from $X import $A, $B").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "G_from2_ab_x3name: {hits:?}");
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("os"));
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("path"));
    assert_eq!(hits[0].captures.get("B").map(String::as_str), Some("sep"));
    let hits = match_pattern(Language::Python, from3, "from $X import $A, $B, $C").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "G_from3_abc: {hits:?}");
    assert_eq!(
        hits[0].captures.get("C").map(String::as_str),
        Some("curdir")
    );
    let hits = match_pattern(Language::Python, from3, "from $X import $A, $B, $C, $D").unwrap();
    assert!(hits.is_empty(), "G_from3_ctrl: {hits:?}");
    let hits = match_pattern(Language::Python, from3, "from $X import path, sep").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "F6_from_lit_names: literal names unify: {hits:?}"
    );
    let hits = match_pattern(Language::Python, from3, "from $X import path, other").unwrap();
    assert!(hits.is_empty(), "literal names byte-match (sg unification)");
    // Paren faces demarcate both ways.
    let paren2 = "from os import (path, sep)\n";
    let hits = match_pattern(Language::Python, paren2, "from $X import ($A, $B)").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "F6_from_paren_ab: {hits:?}");
    assert_eq!(hits[0].captures.get("B").map(String::as_str), Some("sep"));
    let hits = match_pattern(
        Language::Python,
        "from os import path, sep\n",
        "from $X import ($A, $B)",
    )
    .unwrap();
    assert!(hits.is_empty(), "F6_from_paren_ab_x_plain_src: {hits:?}");
    let hits = match_pattern(Language::Python, paren2, "from $X import $A, $B").unwrap();
    assert!(hits.is_empty(), "R2_from_plain_x_paren_src: {hits:?}");
    let paren3 = "from os import (path, sep, curdir)\n";
    let hits = match_pattern(Language::Python, paren3, "from $X import ($A, $B, $C)").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "R2_from_abc_x_paren3: {hits:?}");
    // The star face.
    let star = "from os import *\n";
    let hits = match_pattern(Language::Python, star, "from $X import *").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "F6_from_star: {hits:?}");
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("os"));
    let hits = match_pattern(
        Language::Python,
        "from os import path, sep\n",
        "from $X import *",
    )
    .unwrap();
    assert!(hits.is_empty(), "F6_from_star_x_names: {hits:?}");
    let hits = match_pattern(Language::Python, star, "from $X import $Y").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "R2_from_nameY_x_star_src held: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("Y").map(String::as_str), Some("*"));
    // Alias face: the FIRST child must be aliased_import.
    let hits = match_pattern(Language::Python, "import o as a, p\n", "import $X as $Y").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "G_alias_x_secondplain: {hits:?}");
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("o"));
    assert_eq!(hits[0].captures.get("Y").map(String::as_str), Some("a"));
    let hits = match_pattern(Language::Python, "import os, p as b\n", "import $X as $Y").unwrap();
    assert!(hits.is_empty(), "G_alias_x_firstplain: {hits:?}");
    // Census posture for the new faces.
    for pattern in [
        "import $A, $B, $C",
        "from $X import $A, $B",
        "from $X import ($A, $B)",
        "from $X import *",
    ] {
        assert!(
            native_pattern_answerable(Language::Python, pattern),
            "census: {pattern} must be answerable"
        );
    }
}

/// f141f (F6 php rows 7-8, grid F6_php_use_*): the php `use function|const
/// $X;` kind faces — the literal kind keyword must agree with the
/// candidate's kind keyword; X binds the name; the plain `use $X;` face
/// keeps its whole-clause law (R3_php_use_function).
#[test]
fn f141f_php_use_kind_keywords() {
    let hits = match_pattern(
        Language::Php,
        "<?php\nuse function strlen;\n",
        "use function $X;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "F6_php_use_function: {hits:?}");
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("strlen")
    );
    let hits = match_pattern(
        Language::Php,
        "<?php\nuse const PHP_EOL;\n",
        "use const $X;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "F6_php_use_const: {hits:?}");
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("PHP_EOL")
    );
    let hits = match_pattern(Language::Php, "<?php\nuse App\\Foo;\n", "use function $X;").unwrap();
    assert!(hits.is_empty(), "F6_php_use_function_x_plain: {hits:?}");
    let hits = match_pattern(Language::Php, "<?php\nuse function strlen;\n", "use $X;").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "F6_php_use_plain_x_function held: whole clause: {hits:?}"
    );
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("function strlen")
    );
    assert!(native_pattern_answerable(Language::Php, "use function $X;"));
}

/// f141g (F6 ja row 14 + F7 ja row 1, grids F6_ja_annot_*/R2_ja_sync*): the
/// java method lane accepts a PATTERN-side leading annotation run (each
/// demanded annotation matches a candidate annotation in order; absent →
/// refuse) and the synchronized modifier may sit ANYWHERE in the modifier
/// run (`synchronized static` pattern demands in-order presence).
#[test]
fn f141g_ja_method_annotations_and_modifier_order() {
    let annotated = "class A {\n    @Override\n    public synchronized void o() { g(); }\n}\n";
    let hits = match_pattern(
        Language::Java,
        annotated,
        "@Override\n synchronized void $M() { $B }",
    )
    .unwrap();
    // sg anchors the hit at the pattern-demanded annotation (the modifiers
    // node start — grid F6_ja_annot_pat: byteOffset 14..65, line 2 1-based).
    assert_eq!(lines_of(&hits), vec![2], "F6_ja_annot_pat: {hits:?}");
    assert_eq!(hits[0].captures.get("M").map(String::as_str), Some("o"));
    assert_eq!(hits[0].captures.get("B").map(String::as_str), Some("g();"));
    let plain = "class A {\n    public synchronized void o() { g(); }\n}\n";
    let hits = match_pattern(
        Language::Java,
        plain,
        "@Override\n synchronized void $M() { $B }",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "F6_ja_annot_pat_x_noannot: the demand refuses: {hits:?}"
    );
    // `synchronized` anywhere in the run; the in-order demand governs.
    let sync_static = "class A {\n    synchronized static void m() { g(); }\n}\n";
    let hits = match_pattern(
        Language::Java,
        sync_static,
        "synchronized static void $M() { $B }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "R2_ja_syncstatic_patorder_cand (sg start line 1 0-based): {hits:?}"
    );
    assert_eq!(hits[0].captures.get("M").map(String::as_str), Some("m"));
    let static_sync = "class A {\n    static synchronized void m() { g(); }\n}\n";
    let hits = match_pattern(
        Language::Java,
        static_sync,
        "synchronized static void $M() { $B }",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "F7_ja_syncstatic_pat: out-of-order refuses: {hits:?}"
    );
    let hits = match_pattern(
        Language::Java,
        static_sync,
        "static synchronized void $M() { $B }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "R2_ja_staticsync_pat_staticfirst held (sg start line 1 0-based): {hits:?}"
    );
    assert!(native_pattern_answerable(
        Language::Java,
        "synchronized static void $M() { $B }"
    ));
    assert!(native_pattern_answerable(
        Language::Java,
        "@Override\n synchronized void $M() { $B }"
    ));
}

/// f141h (F6 py row 15, grid F6_py_del_chain*/R2_del_chain*): the py del
/// chain-tail call face — `$A.b($C)` binds the receiver BASE and the single
/// argument around the literal attr tail; a call without the attr tail and
/// a two-argument call refuse.
#[test]
fn f141h_py_del_chain_tail_call() {
    let hits = match_pattern(Language::Python, "del a.b(c)\n", "del $A.b($C)").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "F6_py_del_chain: {hits:?}");
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("a"));
    assert_eq!(hits[0].captures.get("C").map(String::as_str), Some("c"));
    let hits = match_pattern(Language::Python, "del f(c)\n", "del $A.b($C)").unwrap();
    assert!(
        hits.is_empty(),
        "F6_py_del_chain_x_plain: no attr tail refuses: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del a.b(c, d)\n", "del $A.b($C)").unwrap();
    assert!(
        hits.is_empty(),
        "R2_del_chain_2arg: exact-count holds: {hits:?}"
    );
    let hits = match_pattern(Language::Python, "del a.b(c.d)\n", "del $A.b($C)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "R2_del_chain_dotted_arg: {hits:?}"
    );
    assert_eq!(hits[0].captures.get("C").map(String::as_str), Some("c.d"));
    assert!(native_pattern_answerable(Language::Python, "del $A.b($C)"));
}

/// f141i (F7 kt rows 5-6 + swift row 7, grid F7_*): the kt
/// `companion object { $B }` / `init { $B }` and swift `deinit { $B }`
/// spellings are sg ACCEPTED-EMPTY (rc1 `[]` — the walk's empty IS the
/// agreement); census-answerable, never loud.
#[test]
fn f141i_kt_swift_accepted_empty_siblings() {
    let kt_companion = "class C {\n    companion object {\n        const val K = 1\n    }\n}\n";
    let hits = match_pattern(Language::Kotlin, kt_companion, "companion object { $B }").unwrap();
    assert!(hits.is_empty(), "F7_kt_companion: {hits:?}");
    let kt_init = "class C {\n    init {\n        g()\n    }\n}\n";
    let hits = match_pattern(Language::Kotlin, kt_init, "init { $B }").unwrap();
    assert!(hits.is_empty(), "F7_kt_init: {hits:?}");
    let sw_deinit = "class C {\n    deinit {\n        g()\n    }\n}\n";
    let hits = match_pattern(Language::Swift, sw_deinit, "deinit { $B }").unwrap();
    assert!(hits.is_empty(), "F7_sw_deinit: {hits:?}");
    assert!(native_pattern_answerable(
        Language::Kotlin,
        "companion object { $B }"
    ));
    assert!(native_pattern_answerable(Language::Kotlin, "init { $B }"));
    assert!(native_pattern_answerable(Language::Swift, "deinit { $B }"));
}

/// PASS 141 standing-face correction (S_del_d3_paren): `del ($$$X)` is NOT
/// sg ACCEPTED-binds-nothing — the oracle BINDS the single-element
/// paren-wrapped candidate (multi X=["x"]; receipts: `del (x)` n1,
/// `del (a, b)` rc1 `[]`, `del ()` rc1 `[]`). The 140 R3_py_del_paren_d3
/// "accepted-empty" reading was refuted the same way as §49's php
/// `$`-body law. The multi slot demands EXACTLY ONE inner operand.
#[test]
fn f141j_del_paren_multi_single_element_bind() {
    let hits = match_pattern(Language::Python, "del (x)\n", "del ($$$X)").unwrap();
    assert_eq!(hits.len(), 1, "S_del_d3_paren: {hits:?}");
    assert_eq!(
        hits[0].captures.get("$$$X").map(String::as_str),
        Some("x"),
        "multi namespace key binds the single inner operand: {hits:?}"
    );
    assert!(
        match_pattern(Language::Python, "del (a, b)\n", "del ($$$X)")
            .unwrap()
            .is_empty()
    );
    assert!(match_pattern(Language::Python, "del ()\n", "del ($$$X)")
        .unwrap()
        .is_empty());
    assert!(match_pattern(Language::Python, "del x\n", "del ($$$X)")
        .unwrap()
        .is_empty());
}

/// PASS 141 standing-face fix (S_del G03_chain): the MIXED postfix+meta
/// del list is sg-answering — receipts: `del $O[$K], $Y` × `del d[k], y`
/// n1 (O=d K=k Y=y), × `del d[k], y, z` n1 (TRAILING extras absorbed),
/// × `del z, d[k], y` rc1 `[]` (no leading absorb), × `del d[j], y` n1
/// (K=j), × `del d[k]` rc1 `[]` (short refuses). Previously the face was
/// unregistered and loud at the CLI (post-walk backstop rc2).
#[test]
fn f141k_del_mixed_postfix_meta_list() {
    let hits = match_pattern(Language::Python, "del d[k], y\n", "del $O[$K], $Y").unwrap();
    assert_eq!(hits.len(), 1, "G03_chain: {hits:?}");
    assert_eq!(hits[0].captures.get("O").map(String::as_str), Some("d"));
    assert_eq!(hits[0].captures.get("K").map(String::as_str), Some("k"));
    assert_eq!(hits[0].captures.get("Y").map(String::as_str), Some("y"));
    // Trailing extra operand absorbed.
    let hits = match_pattern(Language::Python, "del d[k], y, z\n", "del $O[$K], $Y").unwrap();
    assert_eq!(hits.len(), 1, "G03_chain_trail: {hits:?}");
    assert_eq!(hits[0].captures.get("Y").map(String::as_str), Some("y"));
    // No leading absorb; short candidate refuses; index meta binds any.
    assert!(
        match_pattern(Language::Python, "del z, d[k], y\n", "del $O[$K], $Y")
            .unwrap()
            .is_empty()
    );
    assert!(
        match_pattern(Language::Python, "del d[k]\n", "del $O[$K], $Y")
            .unwrap()
            .is_empty()
    );
    let hits = match_pattern(Language::Python, "del d[j], y\n", "del $O[$K], $Y").unwrap();
    assert_eq!(hits.len(), 1, "G03_chain_kbinds: {hits:?}");
    assert_eq!(hits[0].captures.get("K").map(String::as_str), Some("j"));
    // Structural-last and call-slot lists follow the same law (oracle
    // cells: `del $A, $O[$K]` x `del x, d[k], w` n1; `del f($G), $Y` x
    // `del f(x), y, z` n1; `del $O[$K].$A, $B` x `del d[k].c, y` n1).
    let hits = match_pattern(Language::Python, "del x, d[k], w\n", "del $A, $O[$K]").unwrap();
    assert_eq!(hits.len(), 1, "G03_chain_structlast: {hits:?}");
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("x"));
    let hits = match_pattern(Language::Python, "del f(x), y, z\n", "del f($G), $Y").unwrap();
    assert_eq!(hits.len(), 1, "G03_chain_call: {hits:?}");
    assert_eq!(hits[0].captures.get("G").map(String::as_str), Some("x"));
    assert_eq!(hits[0].captures.get("Y").map(String::as_str), Some("y"));
    let hits = match_pattern(Language::Python, "del d[k].c, y\n", "del $O[$K].$A, $B").unwrap();
    assert_eq!(hits.len(), 1, "G03_chain_attr: {hits:?}");
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("c"));
    assert_eq!(hits[0].captures.get("B").map(String::as_str), Some("y"));
}

// ===========================================================================
// PASS 142 — r74 remediation (agent 142). RED-first f142a–f142m; grid
// receipts /tmp/phase142R/grid1..6.json (oracle ast-grep 0.45.2 sha16
// 9585263377c1fc98, the `--pattern` exact lane of record).
// ===========================================================================

/// f142a (142A-F1, grids A_*): directive scanner comment transparency —
/// sg walks the AST where comments are trivia: cs using and php use-kind
/// candidates carrying interior comments BIND with trivia-free captures,
/// while PATTERN-side comment faces are sg ACCEPTED-EMPTY (rc1 `[]` —
/// census answerable, the walk's empty is the agreement, never loud).
#[test]
fn f142a_directive_comment_transparency() {
    let hits = match_pattern(Language::CSharp, "using /* c */ System;\n", "using $N;").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "A1: {hits:?}");
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("System")
    );
    let hits = match_pattern(Language::CSharp, "using System /* c */;\n", "using $N;").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "A2: {hits:?}");
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("System")
    );
    let hits = match_pattern(
        Language::CSharp,
        "using /* multi\nline */ System;\n",
        "using $N;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "A3: {hits:?}");
    let hits = match_pattern(
        Language::CSharp,
        "using static /* c */ System.Math;\n",
        "using static $N;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "A4: {hits:?}");
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("System.Math")
    );
    let hits = match_pattern(
        Language::CSharp,
        "global /* c */ using System;\n",
        "global using $N;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "A5: {hits:?}");
    let hits = match_pattern(
        Language::CSharp,
        "global using static /* c */ System.Math;\n",
        "global using static $N;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "A6: {hits:?}");
    // demand-only global: the global candidate binds the plain static pattern too.
    let hits = match_pattern(
        Language::CSharp,
        "global using static /* c */ System.Math;\n",
        "using static $N;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "A20: {hits:?}");
    // the alias face binds TRIVIA-FREE lhs/rhs (sg A='M', T='System.Math').
    let hits = match_pattern(
        Language::CSharp,
        "using M /* c */ = System.Math;\n",
        "using $A = $T;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "A10: {hits:?}");
    assert_eq!(
        hits[0].captures.get("A").map(String::as_str),
        Some("M"),
        "A10 capture strips trivia"
    );
    // php use-kind: comments AFTER the kind keyword are trivia (A7/A16); a
    // comment BEFORE the kind keyword still refuses (A13 — sg n0).
    let hits = match_pattern(
        Language::Php,
        "<?php\nuse function /* c */ strlen;\n",
        "use function $X;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "A7: {hits:?}");
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("strlen")
    );
    let hits = match_pattern(
        Language::Php,
        "<?php\nuse function strlen /* c */;\n",
        "use function $X;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "A16: {hits:?}");
    let hits = match_pattern(
        Language::Php,
        "<?php\nuse /* c */ function strlen;\n",
        "use function $X;",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "A13: comment before kind refuses: {hits:?}"
    );
    // Pattern-side comment faces: sg ACCEPTS the pattern and binds NOTHING.
    let accepted_empty: &[(&str, Language, &str, &str)] = &[
        (
            "using /* c */ $N;",
            Language::CSharp,
            "using System;\n",
            "A21",
        ),
        (
            "using static /* c */ $N;",
            Language::CSharp,
            "using static System.Math;\n",
            "A22",
        ),
        (
            "global /* c */ using $N;",
            Language::CSharp,
            "global using System.IO;\n",
            "I1",
        ),
        (
            "using $A = /* c */ $T;",
            Language::CSharp,
            "using M = System.Math;\n",
            "I4",
        ),
        (
            "use function /* c */ $X;",
            Language::Php,
            "<?php\nuse function strlen;\n",
            "A23",
        ),
        (
            "use const /* c */ $X;",
            Language::Php,
            "<?php\nuse const X;\n",
            "I8",
        ),
        ("use /* c */ $X;", Language::Php, "<?php\nuse Name;\n", "I3"),
        (
            "import /* c */ $X;",
            Language::Java,
            "import java.util.List;\n",
            "I2",
        ),
    ];
    for (pattern, lang, cand, tag) in accepted_empty {
        assert!(
            native_pattern_answerable(*lang, pattern),
            "census {tag}: {pattern} must be accepted-empty-answerable"
        );
        let hits = match_pattern(*lang, cand, pattern).unwrap();
        assert!(hits.is_empty(), "{tag} walk empty: {hits:?}");
    }
}

/// f142b (142A-F2, grids B_*): `::`-qualified LITERAL using patterns ride
/// the directive lane and emit the SINGLE directive node (span ends at the
/// `;` — the pre-fix double emit over-extended into the trailing newline).
#[test]
fn f142b_global_qualified_literal_single_emit() {
    for (cand, pattern, tag) in [
        (
            "using static global::System.Math;\n",
            "using static global::System.Math;",
            "B1",
        ),
        (
            "using static global::A.B;\n",
            "using static global::A.B;",
            "B2",
        ),
        ("using M = global::S;\n", "using M = global::S;", "B3"),
        (
            "using global::System.Math;\n",
            "using global::System.Math;",
            "B4",
        ),
        ("using System::Math;\n", "using System::Math;", "B14"),
    ] {
        let hits = match_pattern(Language::CSharp, cand, pattern).unwrap();
        assert_eq!(hits.len(), 1, "{tag}: exactly one hit: {hits:?}");
        assert_eq!(
            hits[0].byte_end,
            cand.len() - 1,
            "{tag}: span ends at the ';'"
        );
    }
    // followed-by-decl control: n1 unchanged (B5)
    let hits = match_pattern(
        Language::CSharp,
        "using static global::System.Math;\nclass C {}\n",
        "using static global::System.Math;",
    )
    .unwrap();
    assert_eq!(hits.len(), 1, "B5: {hits:?}");
    // the alias qualified-meta face binds T AFTER the qualifier (I7: T='S')
    let hits = match_pattern(
        Language::CSharp,
        "using M = global::S;\n",
        "using $A = global::$T;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "I7: {hits:?}");
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("M"));
    assert_eq!(hits[0].captures.get("T").map(String::as_str), Some("S"));
    // meta-target controls hold (B7/B8/B9 — the 141 fix).
    let hits = match_pattern(
        Language::CSharp,
        "using static global::System.Math;\n",
        "using static $N;",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("global::System.Math"),
        "B7"
    );
}

/// f142c (142B-F1, grids C_*): the pattern-side cs demarcation trivia gate
/// is the sg class of record — class-member trivia (U+FEFF/NBSP/VT) BINDS,
/// Rust-whitespace outsiders (U+2028/U+0085/U+2029/U+202F) are sg
/// ACCEPTED-EMPTY (never over-served, never loud).
#[test]
fn f142c_pattern_side_trivia_class() {
    let rethrow = "void M() {\n    try { f(); } catch (E e) { throw; }\n}\n";
    // accepted-empty outsiders: census answerable, walk binds NOTHING.
    for (pattern, tag) in [
        ("throw\u{2028};", "C1"),
        ("throw\u{0085};", "C2"),
        ("throw\u{202f};", "C3"),
        ("throw\u{2029};", "C6"),
    ] {
        assert!(
            native_pattern_answerable(Language::CSharp, pattern),
            "census {tag}: {pattern:?} must be accepted-empty-answerable"
        );
        let hits = match_pattern(Language::CSharp, rethrow, pattern).unwrap();
        assert!(hits.is_empty(), "{tag} walk empty: {hits:?}");
    }
    // class-member trivia BINDS.
    for (pattern, tag) in [
        ("throw\u{FEFF};", "C4"),
        ("throw\u{000B};", "C5"),
        ("throw ;", "C7"),
    ] {
        let hits = match_pattern(Language::CSharp, rethrow, pattern).unwrap();
        assert_eq!(lines_of(&hits), vec![2], "{tag}: {hits:?}");
    }
    // cs using demarcation: U+2028 accepted-empty; U+FEFF/NBSP/VT bind.
    assert!(
        native_pattern_answerable(Language::CSharp, "using\u{2028}$A = $T;"),
        "census C8"
    );
    let hits = match_pattern(
        Language::CSharp,
        "using Point = (int, int);\n",
        "using\u{2028}$A = $T;",
    )
    .unwrap();
    assert!(hits.is_empty(), "C8 walk empty: {hits:?}");
    assert!(
        native_pattern_answerable(Language::CSharp, "using\u{2028}static $N;"),
        "census C13"
    );
    let hits = match_pattern(
        Language::CSharp,
        "using static System.Math;\n",
        "using\u{2028}static $N;",
    )
    .unwrap();
    assert!(hits.is_empty(), "C13 walk empty: {hits:?}");
    let hits = match_pattern(Language::CSharp, "using System.Text;\n", "using\u{FEFF}$N;").unwrap();
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("System.Text"),
        "C9"
    );
    let hits = match_pattern(
        Language::CSharp,
        "global using System;\n",
        "global\u{FEFF}using $N;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "C12");
    let hits = match_pattern(Language::CSharp, "using System.Text;\n", "using\u{00A0}$N;").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "C10 NBSP control");
    let hits = match_pattern(Language::CSharp, "using System.Text;\n", "using\u{000B}$N;").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "C11 VT control");
}

/// f142d (142B-F2 law pin, grids D_*): `del ((a, b))` — sg BINDS the
/// parenthesized-tuple candidate with the inner tuple text under the slot
/// (grid5/6 receipts J-cells); the subject's identical posture is the sg
/// agreement, NOT a law hole. The 142B-F2 flag is refuted of record.
#[test]
fn f142d_del_double_paren_law_pin() {
    let hits = match_pattern(Language::Python, "del ((a, b))\n", "del ($$$X)").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "D1: {hits:?}");
    assert_eq!(
        hits[0].captures.get("$$$X").map(String::as_str),
        Some("(a, b)"),
        "D1 X = the tuple text ($$$ multi-key encoding of record)"
    );
    let hits = match_pattern(Language::Python, "del ((a))\n", "del ($$$X)").unwrap();
    assert_eq!(
        hits[0].captures.get("$$$X").map(String::as_str),
        Some("(a)"),
        "D2"
    );
    let hits = match_pattern(Language::Python, "del ((a, b))\n", "del ($X)").unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("(a, b)"),
        "D4"
    );
    let hits = match_pattern(Language::Python, "del (x)\n", "del ($$$X)").unwrap();
    assert_eq!(
        hits[0].captures.get("$$$X").map(String::as_str),
        Some("x"),
        "D6 control"
    );
    let hits = match_pattern(Language::Python, "del (a, b)\n", "del ($$$X)").unwrap();
    assert!(
        hits.is_empty(),
        "D7: the bare-tuple >=2 law holds: {hits:?}"
    );
}

/// f142e (142A-F3 rs roots, grids E1-E4/H21-H22): `mod $N { $B }` binds
/// the SINGLE-statement mod block (N=name, B=the member text); 2-statement
/// bodies and bodyless `mod inner;` candidates refuse (sg rc1 `[]`).
/// `extern crate $N;` binds the crate name.
#[test]
fn f142e_rs_mod_and_extern_roots() {
    let hits = match_pattern(
        Language::Rust,
        "mod inner {\n    pub fn f() {}\n}\n",
        "mod $N { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "E1: {hits:?}");
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("inner"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("pub fn f() {}")
    );
    let hits = match_pattern(
        Language::Rust,
        "mod inner {\n    pub fn f() {}\n    pub fn g() {}\n}\n",
        "mod $N { $B }",
    )
    .unwrap();
    assert!(hits.is_empty(), "E2: single-exact body law: {hits:?}");
    let hits = match_pattern(Language::Rust, "mod inner;\n", "mod $N { $B }").unwrap();
    assert!(hits.is_empty(), "E3: bodyless candidate refuses: {hits:?}");
    let hits = match_pattern(
        Language::Rust,
        "mod inner {\n    pub fn f() {}\n}\n",
        "mod inner { $B }",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("pub fn f() {}"),
        "H21 literal name"
    );
    let hits = match_pattern(Language::Rust, "fn f() {}\n", "mod $N { $B }").unwrap();
    assert!(hits.is_empty(), "H22: kind gate: {hits:?}");
    for pattern in ["mod $N { $B }", "mod inner { $B }"] {
        assert!(
            native_pattern_answerable(Language::Rust, pattern),
            "census: {pattern}"
        );
    }
    let hits = match_pattern(Language::Rust, "extern crate alloc;\n", "extern crate $N;").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "E4: {hits:?}");
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("alloc"));
    assert!(
        native_pattern_answerable(Language::Rust, "extern crate $N;"),
        "census E4"
    );
}

/// f142f (142A-F3 go/rb roots, grids E5-E7/E14/H19-H20/H25-H26): go
/// `type $N $T` binds the type tail WHOLE (struct blocks included); rb
/// `module $N\n  $B\nend` binds the trimmed body; kind gates hold.
#[test]
fn f142f_go_type_and_rb_module_roots() {
    let hits = match_pattern(
        Language::Go,
        "package main\n\ntype MyInt int\n",
        "type $N $T",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3], "E5: {hits:?}");
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("MyInt"));
    assert_eq!(hits[0].captures.get("T").map(String::as_str), Some("int"));
    let hits = match_pattern(
        Language::Go,
        "package main\n\ntype P struct {\n\tX int\n}\n",
        "type $N $T",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("T").map(String::as_str),
        Some("struct {\n\tX int\n}"),
        "E6"
    );
    let hits = match_pattern(Language::Go, "package main\n\nvar x int\n", "type $N $T").unwrap();
    assert!(hits.is_empty(), "H26: kind gate: {hits:?}");
    let hits = match_pattern(
        Language::Go,
        "package main\n\ntype MyInt int\n",
        "type MyInt $T",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("T").map(String::as_str),
        Some("int"),
        "H25 literal name"
    );
    assert!(
        native_pattern_answerable(Language::Go, "type $N $T"),
        "census E5"
    );
    // the `;`-ful spelling is sg RC8 — both loud (E13): the census must stay loud.
    assert!(
        !native_pattern_answerable(Language::Go, "type $N $T;"),
        "census E13: the ; spelling stays fail-closed"
    );
    let hits = match_pattern(
        Language::Ruby,
        "module M\n  def f\n    1\n  end\nend\n",
        "module $N\n  $B\nend",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "E7: {hits:?}");
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("M"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("def f\n    1\n  end")
    );
    let hits = match_pattern(
        Language::Ruby,
        "module M\n  def f\n    1\n  end\n  def g\n    2\n  end\nend\n",
        "module $N\n  $B\nend",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("def f\n    1\n  end\n  def g\n    2\n  end"),
        "E14 whole-body law"
    );
    let hits = match_pattern(
        Language::Ruby,
        "module M\n  def f\n    1\n  end\nend\n",
        "module M\n  $B\nend",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "H19 literal name");
    let hits = match_pattern(
        Language::Ruby,
        "class C\n  def f\n    1\n  end\nend\n",
        "module $N\n  $B\nend",
    )
    .unwrap();
    assert!(hits.is_empty(), "H20: kind gate: {hits:?}");
    assert!(
        native_pattern_answerable(Language::Ruby, "module $N\n  $B\nend"),
        "census E7"
    );
}

/// f142g (142A-F3 ts roots, grids E10-E11/H23-H24 + E10b): `declare module
/// $N { $B }` binds the single-statement ambient module (N carries the
/// quotes); `declare const $X: $T;` binds the declarator; `let` refuses.
#[test]
fn f142g_ts_declare_roots() {
    let hits = match_pattern(
        Language::TypeScript,
        "declare module \"m\" {\n    export const x: number;\n}\n",
        "declare module $N { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "E10: {hits:?}");
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("\"m\""));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("export const x: number;")
    );
    let hits = match_pattern(
        Language::TypeScript,
        "declare module \"m\" {\n    export const x: number;\n    export const y: number;\n}\n",
        "declare module $N { $B }",
    )
    .unwrap();
    assert!(hits.is_empty(), "E10b: single-exact body law: {hits:?}");
    assert!(
        native_pattern_answerable(Language::TypeScript, "declare module $N { $B }"),
        "census E10"
    );
    let hits = match_pattern(
        Language::TypeScript,
        "declare const x: number;\n",
        "declare const $X: $T;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "E11: {hits:?}");
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("x"));
    assert_eq!(
        hits[0].captures.get("T").map(String::as_str),
        Some("number")
    );
    let hits = match_pattern(
        Language::TypeScript,
        "declare let x: number;\n",
        "declare const $X: $T;",
    )
    .unwrap();
    assert!(hits.is_empty(), "H23: const kind demanded: {hits:?}");
    let hits = match_pattern(
        Language::TypeScript,
        "declare const x: number;\n",
        "declare const x: $T;",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("T").map(String::as_str),
        Some("number"),
        "H24 literal name"
    );
    assert!(
        native_pattern_answerable(Language::TypeScript, "declare const $X: $T;"),
        "census E11"
    );
}

/// f142h (142A-F3 py async root, grids E8/E8b/E8c/H16-H18 + E9 control):
/// `async def $N($$P):\n    $$B` — P is PARAM-EXACT (0/2 params refuse),
/// B binds the whole body block text (multi-statement OK); the single-`$`
/// spelling keeps its existing answering route (E9 control).
#[test]
fn f142h_py_async_def_root() {
    let hits = match_pattern(
        Language::Python,
        "async def fetch(url):\n    return 1\n",
        "async def $N($$P):\n    $$B",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "E8: {hits:?}");
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("fetch"));
    assert_eq!(hits[0].captures.get("P").map(String::as_str), Some("url"));
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("return 1")
    );
    let hits = match_pattern(
        Language::Python,
        "async def fetch(url):\n    return 1\n    return 2\n",
        "async def $N($$P):\n    $$B",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("return 1\n    return 2"),
        "E8b whole-body law"
    );
    let hits = match_pattern(
        Language::Python,
        "async def fetch(url, opt):\n    return 1\n",
        "async def $N($$P):\n    $$B",
    )
    .unwrap();
    assert!(hits.is_empty(), "E8c: param-exact law: {hits:?}");
    let hits = match_pattern(
        Language::Python,
        "async def f():\n    return 1\n",
        "async def $N($$P):\n    $$B",
    )
    .unwrap();
    assert!(hits.is_empty(), "H16: 0-param refuses: {hits:?}");
    let hits = match_pattern(
        Language::Python,
        "async def fetch(url):\n    return 1\n",
        "async def fetch($$P):\n    $$B",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "H17 literal name");
    let hits = match_pattern(
        Language::Python,
        "async def fetch(url):\n    pass\n",
        "async def $N($$P):\n    $$B",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("pass"),
        "H18"
    );
    // the sync def candidate lacks the `async` token — the async face
    // refuses it (sg structural keyword alignment, E-face family).
    let hits = match_pattern(
        Language::Python,
        "def fetch(url):\n    return 1\n",
        "async def $N($$P):\n    $$B",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "H19: sync def refuses async face: {hits:?}"
    );
    // the single-$ spelling keeps its pre-existing answering route (E9).
    let hits = match_pattern(
        Language::Python,
        "async def fetch(url):\n    return 1\n",
        "async def $N($P):\n    $B",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "E9 control: single-$ face unchanged"
    );
    assert!(
        native_pattern_answerable(Language::Python, "async def $N($$P):\n    $$B"),
        "census E8"
    );
}

/// f142i (142A-F5 ja enum — grids G3/G10b/G11/G14-G17/H1-H3/J3/K3/K4/K6):
/// the enum-declaration lane: name meta-or-literal, body meta-or-literal
/// with the SINGLE-member law (constants + methods; the empty-body pattern
/// binds 0-member candidates); modifier-carrying and multi-member
/// candidates refuse; non-enum candidates refuse.
#[test]
fn f142i_ja_enum_lane() {
    let bare = "enum C {\n    RED\n}\n";
    let hits = match_pattern(Language::Java, bare, "enum $N { $B }").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "J3: {hits:?}");
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("C"));
    assert_eq!(hits[0].captures.get("B").map(String::as_str), Some("RED"));
    let hits = match_pattern(
        Language::Java,
        "enum Color {\n    RED\n}\n",
        "enum Color { $B }",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("RED"),
        "G10b literal name"
    );
    let hits = match_pattern(Language::Java, bare, "enum $N { RED }").unwrap();
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("C"),
        "G16 literal body"
    );
    let hits = match_pattern(
        Language::Java,
        "enum Color {\n    BLUE\n}\n",
        "enum $N { RED }",
    )
    .unwrap();
    assert!(hits.is_empty(), "H3 literal-body mismatch: {hits:?}");
    let hits = match_pattern(
        Language::Java,
        "enum C {\n    RED, GREEN\n}\n",
        "enum $N { $B }",
    )
    .unwrap();
    assert!(hits.is_empty(), "K3: single-member law: {hits:?}");
    let hits = match_pattern(
        Language::Java,
        "enum Color {\n    RED, GREEN;\n    void f() {}\n}\n",
        "enum $N { $B }",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "G3: modifier+multi candidate refuses: {hits:?}"
    );
    let hits = match_pattern(
        Language::Java,
        "enum Color {\n    void f() {}\n}\n",
        "enum Color { $B }",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("void f() {}"),
        "G17 method member"
    );
    let hits = match_pattern(Language::Java, "enum C { }\n", "enum $N { }").unwrap();
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("C"),
        "K4 empty-body pattern binds empty candidate"
    );
    let hits = match_pattern(Language::Java, "enum C {\n    RED\n}\n", "enum $N { }").unwrap();
    assert!(
        hits.is_empty(),
        "G11 empty-body pattern vs member candidate: {hits:?}"
    );
    let hits = match_pattern(
        Language::Java,
        "class C { void f() {} }\n",
        "enum $N { $B }",
    )
    .unwrap();
    assert!(hits.is_empty(), "K6: kind gate: {hits:?}");
    assert!(
        native_pattern_answerable(Language::Java, "enum $N { $B }"),
        "census J3"
    );
    assert!(
        native_pattern_answerable(Language::Java, "enum $N { }"),
        "census K4"
    );
}

/// f142j (142A-F5 cs record/struct — grids G4-G6/J1-J2/J6-J8/K1-K2/K5/K7-K8):
/// the record/struct declaration lane — bare candidates bind (name/param/
/// body slots, single-exact laws); modifier-carrying, multi-member, empty
/// and cross-kind candidates refuse.
#[test]
fn f142j_cs_record_struct_lane() {
    let hits = match_pattern(Language::CSharp, "record Q(int X);\n", "record $N($P);").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "J1: {hits:?}");
    assert_eq!(hits[0].captures.get("N").map(String::as_str), Some("Q"));
    assert_eq!(hits[0].captures.get("P").map(String::as_str), Some("int X"));
    let hits = match_pattern(
        Language::CSharp,
        "record Q(int X, int Y);\n",
        "record $N($P);",
    )
    .unwrap();
    assert!(hits.is_empty(), "K2: param-exact law: {hits:?}");
    let hits = match_pattern(Language::CSharp, "record Q();\n", "record $N($P);").unwrap();
    assert!(hits.is_empty(), "K7: 0-param refuses: {hits:?}");
    let hits = match_pattern(
        Language::CSharp,
        "record Q(int X)\n{\n    void F() {}\n}\n",
        "record $N($P) { $B }",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("void F() {}"),
        "J6"
    );
    let hits = match_pattern(
        Language::CSharp,
        "public record Point(int X, int Y);\n",
        "record $N($P);",
    )
    .unwrap();
    assert!(hits.is_empty(), "G4: modifier candidate refuses: {hits:?}");
    let hits = match_pattern(
        Language::CSharp,
        "record Point(int X);\n",
        "record Point($P);",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("P").map(String::as_str),
        Some("int X"),
        "J8 literal name"
    );
    let hits = match_pattern(
        Language::CSharp,
        "struct S {\n    int X;\n}\n",
        "struct $N { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "J2: {hits:?}");
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("int X;")
    );
    let hits = match_pattern(
        Language::CSharp,
        "struct S {\n    int X;\n    int Y;\n}\n",
        "struct $N { $B }",
    )
    .unwrap();
    assert!(hits.is_empty(), "K1: single-member law: {hits:?}");
    let hits = match_pattern(
        Language::CSharp,
        "public struct S\n{\n    public int X;\n}\n",
        "struct $N { $B }",
    )
    .unwrap();
    assert!(hits.is_empty(), "G6: modifier candidate refuses: {hits:?}");
    let hits = match_pattern(Language::CSharp, "record Q(int X);\n", "struct $N { $B }").unwrap();
    assert!(hits.is_empty(), "K5: kind gate: {hits:?}");
    let hits = match_pattern(
        Language::CSharp,
        "class C {\n    struct S { int X; }\n}\n",
        "struct $N { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "K8 nested bind");
    let hits = match_pattern(Language::CSharp, "struct S { }\n", "struct $N { $B }").unwrap();
    assert!(hits.is_empty(), "J9: empty body refuses: {hits:?}");
    assert!(
        native_pattern_answerable(Language::CSharp, "record $N($P);"),
        "census J1"
    );
    assert!(
        native_pattern_answerable(Language::CSharp, "struct $N { $B }"),
        "census J2"
    );
}

/// f142k (142A-F5 cs using dotted-meta patterns — grids G1/G2/G8/G9/I6):
/// a plain-face cs using pattern whose target is a dotted/qualified META
/// path is sg ACCEPTED-EMPTY (binds NOTHING on any candidate — census
/// answerable, walk empty, never loud).
#[test]
fn f142k_using_dotted_meta_accepted_empty() {
    for (pattern, cand, tag) in [
        ("using $A.B;", "using System.Console;\n", "G1"),
        ("using static $N.M;", "using static System.Math;\n", "G2"),
        ("using $A.B;", "using System;\n", "G8"),
        ("global using $A.B;", "global using System.Console;\n", "G9"),
        (
            "using static global::$N;",
            "using static global::System.Math;\n",
            "I6",
        ),
    ] {
        assert!(
            native_pattern_answerable(Language::CSharp, pattern),
            "census {tag}: {pattern} must be accepted-empty-answerable"
        );
        let hits = match_pattern(Language::CSharp, cand, pattern).unwrap();
        assert!(hits.is_empty(), "{tag} walk empty: {hits:?}");
    }
    // Negative control: the SINGLE-segment meta path is an ordinary
    // BINDING face, not accepted-empty — the >=2-segment minimum of the
    // dotted/qualified family is load-bearing (G1's base case).
    let hits = match_pattern(Language::CSharp, "using System;\n", "using $N;").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "G0: single-segment path binds: {hits:?}"
    );
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("System")
    );
}

/// f142l (142A-F4 cs alias rhs faces — grids F1/F4/F7/F8-F10/H4-H5/I7/B9):
/// the alias rhs admits the TUPLE-slot face (`($T, $U)`, count-exact) and
/// the ARRAY-suffix face (`$T[]`, element binding); the single-meta face
/// keeps its whole-rhs law.
#[test]
fn f142l_cs_alias_rhs_faces() {
    let hits = match_pattern(
        Language::CSharp,
        "using Point = (int, int);\n",
        "using $A = ($T, $U);",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "F1: {hits:?}");
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("Point"));
    assert_eq!(hits[0].captures.get("T").map(String::as_str), Some("int"));
    assert_eq!(hits[0].captures.get("U").map(String::as_str), Some("int"));
    let hits = match_pattern(
        Language::CSharp,
        "using Point = ( int, int );\n",
        "using $A = ($T, $U);",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "F4 spaced candidate");
    let hits = match_pattern(
        Language::CSharp,
        "using P = (int, int, int);\n",
        "using $A = ($T, $U);",
    )
    .unwrap();
    assert!(hits.is_empty(), "F7: count-exact law: {hits:?}");
    let hits = match_pattern(Language::CSharp, "using A = int[];\n", "using $A = $T[];").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "F10: {hits:?}");
    assert_eq!(hits[0].captures.get("T").map(String::as_str), Some("int"));
    let hits = match_pattern(Language::CSharp, "using A = int[][];\n", "using $A = $T[];").unwrap();
    assert_eq!(
        hits[0].captures.get("T").map(String::as_str),
        Some("int[]"),
        "H4 recursive suffix"
    );
    let hits = match_pattern(Language::CSharp, "using A = int [];\n", "using $A = $T[];").unwrap();
    assert_eq!(
        hits[0].captures.get("T").map(String::as_str),
        Some("int"),
        "H5 spaced suffix"
    );
    // single-meta whole-rhs law holds (F8 — sg T='(int, int)').
    let hits = match_pattern(
        Language::CSharp,
        "using P = (int, int);\n",
        "using $A = $T;",
    )
    .unwrap();
    assert_eq!(hits.len(), 1, "F8 control");
    // meta rhs binds the qualified text whole (B9).
    let hits = match_pattern(Language::CSharp, "using M = global::S;\n", "using $A = $T;").unwrap();
    assert_eq!(
        hits[0].captures.get("T").map(String::as_str),
        Some("global::S"),
        "B9 control"
    );
    assert!(
        native_pattern_answerable(Language::CSharp, "using $A = ($T, $U);"),
        "census F1"
    );
    assert!(
        native_pattern_answerable(Language::CSharp, "using $A = $T[];"),
        "census F10"
    );
}

/// f142m (142A-F4 py del slice — grids F3/F6/G7/H11-H15): the subscript
/// slot admits the `$A:$B` slice face — both slice bounds are slots (meta
/// or literal), the step is absorbed, missing bounds refuse, plain
/// subscript candidates refuse; the whole-text `$K` face holds (H15).
#[test]
fn f142m_py_del_slice_face() {
    let hits = match_pattern(Language::Python, "del d[1:2]\n", "del $O[$A:$B]").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "F3: {hits:?}");
    assert_eq!(hits[0].captures.get("O").map(String::as_str), Some("d"));
    assert_eq!(hits[0].captures.get("A").map(String::as_str), Some("1"));
    assert_eq!(hits[0].captures.get("B").map(String::as_str), Some("2"));
    let hits = match_pattern(Language::Python, "del d[1:2:3]\n", "del $O[$A:$B]").unwrap();
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("2"),
        "F6 step absorbed"
    );
    let hits = match_pattern(Language::Python, "del d[1:9]\n", "del $O[$A:9]").unwrap();
    assert_eq!(
        hits[0].captures.get("A").map(String::as_str),
        Some("1"),
        "H11 literal hi"
    );
    let hits = match_pattern(Language::Python, "del d[1:9]\n", "del $O[1:$B]").unwrap();
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("9"),
        "H12 literal lo"
    );
    let hits = match_pattern(Language::Python, "del d[:2]\n", "del $O[$A:$B]").unwrap();
    assert!(hits.is_empty(), "H13: missing lower refuses: {hits:?}");
    let hits = match_pattern(Language::Python, "del d[1:]\n", "del $O[$A:$B]").unwrap();
    assert!(hits.is_empty(), "H14: missing upper refuses: {hits:?}");
    let hits = match_pattern(Language::Python, "del d[k]\n", "del $O[$A:$B]").unwrap();
    assert!(hits.is_empty(), "G7: plain subscript refuses: {hits:?}");
    let hits = match_pattern(Language::Python, "del d[1:2]\n", "del $O[$K]").unwrap();
    assert_eq!(
        hits[0].captures.get("K").map(String::as_str),
        Some("1:2"),
        "H15 whole-text control"
    );
    assert!(
        native_pattern_answerable(Language::Python, "del $O[$A:$B]"),
        "census F3"
    );
}

/// f143a (142E-F1, phase143R grid cs_throw_*): sg's pinned cs grammar skips
/// the FULL Unicode White_Space set (plus U+FEFF and bare control junk) at
/// the throw-semi token gap — every spelling in the grid bind-set answers
/// n1 on the bare-rethrow candidate, where the subject's 0.23.5 parser
/// (ASCII-scoped extras) leaves an ERROR child that the pre-fix walk
/// treated as an operand (silent n0). An actual operand still refuses
/// (sg cs_throw_operand_2028 n0), and non-junk spellings that glue
/// (`throwfoo`) refuse.
#[test]
fn f143a_cs_throw_candidate_gap_bind() {
    let mk = |gap: &str| format!("class K {{\n    void M() {{\n        throw{gap};\n    }}\n}}\n");
    // sg bind-set of record (grid cs_throw_*): Unicode White_Space + FEFF
    // (142 class members: controls prove the junk arm) — ALL bind n1.
    for (ch, tag) in [
        ("\u{2028}", "T28"),
        ("\u{2029}", "T29"),
        ("\u{0085}", "T85"),
        ("\u{202F}", "T2F"),
        ("\u{1680}", "T80"),
        ("\u{2000}", "T00"),
        ("\u{205F}", "T5F"),
        ("\u{FEFF}", "TFE"),
        ("\u{001A}", "T1A"),
        ("\u{0000}", "T00NUL"),
    ] {
        let cand = mk(ch);
        let hits = match_pattern(Language::CSharp, &cand, "throw;").unwrap();
        assert_eq!(
            lines_of(&hits),
            vec![3],
            "{tag}: gap {ch:?} must bind: {hits:?}"
        );
    }
    // operand control: junk-gap + real operand refuses (sg n0).
    let cand = mk("\u{2028} foo");
    let hits = match_pattern(Language::CSharp, &cand, "throw;").unwrap();
    assert!(hits.is_empty(), "OP1: operand still refuses: {hits:?}");
    // glue control: no gap at all (`throwfoo` is one identifier) refuses.
    let cand = mk("\u{0000}foo");
    let hits = match_pattern(Language::CSharp, &cand, "throw;").unwrap();
    assert!(hits.is_empty(), "GL1: glued operand refuses: {hits:?}");
    // clean control binds (posture unchanged).
    let hits = match_pattern(Language::CSharp, &mk(""), "throw;").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "CTRL: {hits:?}");
}

/// f143b (142E-F1, phase143R grid cs_using_*/cs_global_*): the cs
/// candidate-side demarcation skips widen from the 142 pattern-side class to
/// the sg-pinned candidate gap set — plain, global, and static faces bind
/// the junk-gap candidates sg binds, while junk glued INSIDE a name token
/// refuses (sg cs_name_glue2028/cs_name_001a n0).
#[test]
fn f143b_cs_using_candidate_gap_bind() {
    for (ch, tag) in [
        ("\u{2028}", "U28"),
        ("\u{0085}", "U85"),
        ("\u{1680}", "U80"),
        ("\u{205F}", "U5F"),
        ("\u{001A}", "U1A"),
    ] {
        let cand = format!("using{ch}System;\nclass K {{}}\n");
        let hits = match_pattern(Language::CSharp, &cand, "using $N;").unwrap();
        assert_eq!(lines_of(&hits), vec![1], "{tag}: {hits:?}");
        assert_eq!(
            hits[0].captures.get("N").map(String::as_str),
            Some("System"),
            "{tag}: N binds trivia-free"
        );
        let cand = format!("global{ch}using System;\nclass K {{}}\n");
        let hits = match_pattern(Language::CSharp, &cand, "global using $N;").unwrap();
        assert_eq!(lines_of(&hits), vec![1], "G{tag}: {hits:?}");
    }
    // static face: junk gaps between BOTH demarcations (sg cs_static_2028 n1).
    let hits = match_pattern(
        Language::CSharp,
        "using\u{2028}static\u{2029}System;\nclass K {}\n",
        "using static $N;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "S1: {hits:?}");
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("System")
    );
    // glue controls: junk inside the name token refuses (sg n0).
    for (cand, tag) in [
        ("using Sys\u{2028}tem;\nclass K {}\n", "GL28"),
        ("using Sys\u{001A}tem;\nclass K {}\n", "GL1A"),
    ] {
        let hits = match_pattern(Language::CSharp, cand, "using $N;").unwrap();
        assert!(hits.is_empty(), "{tag}: glued name refuses: {hits:?}");
    }
}

/// f143c (142E-F1, phase143R grid php_use_* + boundary probes): the php
/// use-kind head lane lands sg-exact in BOTH directions — the gap after the
/// kind keyword must be an sg-php-gap-trivia run (ASCII ws, NBSP, U+FEFF,
/// U+001A): FEFF/NBSP/SUB gaps BIND (the pre-fix lane under-served FEFF and
/// over-served the Rust-ws outsiders that glue into the name token), the
/// glue-gap outsiders refuse, the name section binds the FIRST token (glue
/// chars are token chars), and a gap after `use` itself admits the same
/// trivia class (sg php_head_feff n1).
#[test]
fn f143c_php_use_kind_head_gap_law() {
    let mk = |gap: &str| format!("<?php\nuse function{gap}strlen;\n");
    // glue-gap outsiders: zero trivia gap -> the junk glues into one name
    // token with the kind keyword; sg refuses (grid php_use_* n0).
    for (ch, tag) in [
        ("\u{0085}", "P85"),
        ("\u{1680}", "P80"),
        ("\u{2000}", "P00"),
        ("\u{2028}", "P28"),
        ("\u{2029}", "P29"),
        ("\u{202F}", "P2F"),
        ("\u{205F}", "P5F"),
        ("\u{3000}", "P3000"),
    ] {
        let hits = match_pattern(Language::Php, &mk(ch), "use function $X;").unwrap();
        assert!(hits.is_empty(), "{tag}: glue gap must refuse: {hits:?}");
    }
    // sg-php-gap-trivia binds (FEFF was the pre-fix under-serve).
    for (ch, tag) in [
        ("\u{FEFF}", "PFE"),
        ("\u{00A0}", "PA0"),
        ("\u{001A}", "P1A"),
    ] {
        let hits = match_pattern(Language::Php, &mk(ch), "use function $X;").unwrap();
        assert_eq!(lines_of(&hits), vec![2], "{tag}: {hits:?}");
        assert_eq!(
            hits[0].captures.get("X").map(String::as_str),
            Some("strlen"),
            "{tag} X"
        );
    }
    // head gap after `use` itself: FEFF trivia admits (sg php_head_feff n1).
    let hits = match_pattern(
        Language::Php,
        "<?php\nuse\u{FEFF}function strlen;\n",
        "use function $X;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "HEAD-FE: {hits:?}");
    // no-gap control refuses (sg php_nogap n0).
    let hits = match_pattern(Language::Php, &mk(""), "use function $X;").unwrap();
    assert!(hits.is_empty(), "NOGAP: {hits:?}");
    // name section: the FIRST token binds; trivia splits it; glue chars are
    // token chars (sg space_glue/internal_feff/two_names captures).
    let hits = match_pattern(
        Language::Php,
        "<?php\nuse function str len;\n",
        "use function $X;",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("str"),
        "TWO: {hits:?}"
    );
    let hits = match_pattern(
        Language::Php,
        "<?php\nuse function str\u{FEFF}len;\n",
        "use function $X;",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("str"),
        "SPLIT-FE: {hits:?}"
    );
    let hits = match_pattern(
        Language::Php,
        "<?php\nuse function \u{2028}strlen;\n",
        "use function $X;",
    )
    .unwrap();
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("\u{2028}strlen"),
        "GLUE-X: {hits:?}"
    );
    // registered raw-head law holds: a comment before the kind refuses (A13).
    let hits = match_pattern(
        Language::Php,
        "<?php\nuse /* c */ function strlen;\n",
        "use function $X;",
    )
    .unwrap();
    assert!(hits.is_empty(), "A13-hold: {hits:?}");
}

/// f144a (143A-F1, phase144R grid R*): the cs bare-return semi family — sg
/// answers the operand-less return_statement across the full candidate gap
/// class (Unicode White_Space outsiders, FEFF, control junk, junk runs, and
/// comments are all sg-transparent at the `return`→`;` token gap) while the
/// structural general lane's child alignment refused every junk-carrying
/// candidate silently. Operand-bearing candidates still refuse (sg R9/R10
/// rc1) and glued `returnx` refuses.
#[test]
fn f144a_cs_return_semi_gap_bind() {
    let mk = |gap: &str| format!("class K {{\n    void M() {{\n        return{gap};\n    }}\n}}\n");
    for (ch, tag) in [
        ("\u{2028}", "R28"),
        ("\u{2029}", "R29"),
        ("\u{FEFF}", "RFE"),
        ("\u{00A0}", "RA0"),
        ("\u{0001}", "R01"),
        ("\u{0000}", "RNUL"),
    ] {
        let hits = match_pattern(Language::CSharp, &mk(ch), "return;").unwrap();
        assert_eq!(
            lines_of(&hits),
            vec![3],
            "{tag}: gap {ch:?} must bind: {hits:?}"
        );
    }
    // junk RUN and comment transparency (sg n1 on both).
    let hits = match_pattern(Language::CSharp, &mk("\u{2028}\u{2029}"), "return;").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "RUN: {hits:?}");
    let hits = match_pattern(Language::CSharp, &mk(" /* c */ "), "return;").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "COMMENT: {hits:?}");
    // operand-bearing candidates refuse (sg rc1: the `;`-ful spelling is
    // operand-less discipline in cs too).
    let hits = match_pattern(Language::CSharp, &mk(" 1"), "return;").unwrap();
    assert!(hits.is_empty(), "OP: {hits:?}");
    let hits = match_pattern(Language::CSharp, &mk(" /* c */ 1"), "return;").unwrap();
    assert!(hits.is_empty(), "OP-COMMENT: {hits:?}");
    // glue control refuses; plain control binds.
    let hits = match_pattern(
        Language::CSharp,
        "class K {\n    void M() {\n        returnx;\n    }\n}\n",
        "return;",
    )
    .unwrap();
    assert!(hits.is_empty(), "GLUE: {hits:?}");
    let hits = match_pattern(Language::CSharp, &mk(""), "return;").unwrap();
    assert_eq!(lines_of(&hits), vec![3], "CTRL: {hits:?}");
}

/// f144b (143A-F2, phase144R grid J*): the cs callee→`(` junction gate —
/// sg refuses a comment/U+2028/U+2029 run in the junction (J1/J2/J7/J8) and
/// binds the FEFF/NBSP junctions (J3/J4, sg's cs `\s` is Unicode there) and
/// comment-inside-args (J5). The pre-fix plain-call lane had NO junction
/// consult for csharp and over-served the refusals.
#[test]
fn f144b_cs_call_junction_gate() {
    for (cand, tag) in [
        ("void g() { f /* c */ (1); }\n", "J1-comment"),
        ("void g() { f\u{2028}(1); }\n", "J2-u2028"),
        ("void g() { f\u{2029}(1); }\n", "J7-u2029"),
    ] {
        let hits = match_pattern(Language::CSharp, cand, "f($A);").unwrap();
        assert!(hits.is_empty(), "{tag}: must refuse: {hits:?}");
    }
    // 2-arg junction comment refuses the same way (J8).
    let hits = match_pattern(
        Language::CSharp,
        "void g() { f /* c */ (a, b); }\n",
        "f($A, $B);",
    )
    .unwrap();
    assert!(hits.is_empty(), "J8: {hits:?}");
    // FEFF/NBSP junctions bind (sg n1) — the class of record is
    // is_sg_cs_trivia, NOT the wider candidate gap-junk class.
    for (cand, tag) in [
        ("void g() { f\u{FEFF}(1); }\n", "J3-feff"),
        ("void g() { f\u{00A0}(1); }\n", "J4-a0"),
        ("void g() { f(1); }\n", "J6-plain"),
        ("void g() { f(1 /* c */); }\n", "J5-arg-comment"),
    ] {
        let hits = match_pattern(Language::CSharp, cand, "f($A);").unwrap();
        assert_eq!(lines_of(&hits), vec![1], "{tag}: must bind: {hits:?}");
        assert_eq!(
            hits[0].captures.get("A").map(String::as_str),
            Some("1"),
            "{tag} A"
        );
    }
}

/// f144c (143A-F3, phase144R grid U*): the cs using-statement head junk gate
/// — sg REFUSES the U+2028/U+2029 outsider gaps (var→name and using→var,
/// U1/U6) while the A0/FEFF gap spellings bind (U3/U4/U5). The pre-fix
/// structural general lane skipped the unnamed ERROR child and over-served.
#[test]
fn f144c_cs_using_statement_head_gate() {
    for (cand, tag) in [
        ("void f() { using var\u{2028}s = g(); }\n", "U1-var-gap"),
        ("void f() { using\u{2028}var s = g(); }\n", "U6-using-gap"),
    ] {
        let hits = match_pattern(Language::CSharp, cand, "using var s = $E;").unwrap();
        assert!(hits.is_empty(), "{tag}: must refuse: {hits:?}");
    }
    for (cand, tag) in [
        ("void f() { using var s = g(); }\n", "U2-plain"),
        ("void f() { using var\u{00A0}s = g(); }\n", "U3-a0"),
        ("void f() { using\u{FEFF}var s = g(); }\n", "U4-feff-head"),
        ("void f() { using var\u{FEFF}s = g(); }\n", "U5-feff-var"),
    ] {
        let hits = match_pattern(Language::CSharp, cand, "using var s = $E;").unwrap();
        assert_eq!(lines_of(&hits), vec![1], "{tag}: must bind: {hits:?}");
        assert_eq!(
            hits[0].captures.get("E").map(String::as_str),
            Some("g()"),
            "{tag} E"
        );
    }
}

/// f144d (143A-F4, phase144R grid N*): the php braced-namespace head capture
/// law — n-level binding holds on the whole gap class, but the NAME capture
/// must be trivia-trimmed sg-exactly: `namespace<FEFF>X { f(); }` binds
/// N=`X` (sg metaVariables) where the pre-fix Rust `trim()` kept the BOM in
/// the capture. U+2028 glues into the name token and refuses at n-level
/// (sg N3 rc1) — the trivia-vs-glue boundary is is_sg_php_gap_trivia.
#[test]
fn f144d_php_namespace_head_gap_law() {
    for (gap, tag, bind) in [
        ("\u{FEFF}", "N1-feff", true),
        ("\u{00A0}", "N2-a0", true),
        ("\u{001A}", "N5-001a", true),
        (" ", "N4-plain", true),
        ("\u{2028}", "N3-u2028", false),
    ] {
        let cand = format!("<?php\nnamespace{gap}X {{ f(); }}\n");
        let hits = match_pattern(Language::Php, &cand, "namespace $N { $$B }").unwrap();
        if bind {
            assert_eq!(lines_of(&hits), vec![2], "{tag}: must bind: {hits:?}");
            assert_eq!(
                hits[0].captures.get("N").map(String::as_str),
                Some("X"),
                "{tag} N trimmed sg-exact"
            );
            assert_eq!(
                hits[0].captures.get("B").map(String::as_str),
                Some("f();"),
                "{tag} B"
            );
        } else {
            assert!(hits.is_empty(), "{tag}: glue gap must refuse: {hits:?}");
        }
    }
}

/// f144e (143A-F5, phase144R grid P*): the php KIND-LESS `use $X;` clause
/// head obeys the same sg php gap law as the 143-fixed kind lanes — the
/// FEFF/A0/U+001A head gaps bind (pre-fix the shared Semi arm demanded a
/// literal ASCII space and silently refused) while the U+2028 glue gap
/// refuses (sg n0: the junk eats into the name token).
#[test]
fn f144e_php_kindless_use_head_gap_law() {
    for (gap, tag) in [
        ("\u{FEFF}", "P1-feff"),
        ("\u{00A0}", "P2-a0"),
        ("\u{001A}", "P3-001a"),
    ] {
        let cand = format!("<?php\nuse{gap}MyClass;\n");
        let hits = match_pattern(Language::Php, &cand, "use $X;").unwrap();
        assert_eq!(lines_of(&hits), vec![2], "{tag}: must bind: {hits:?}");
        assert_eq!(
            hits[0].captures.get("X").map(String::as_str),
            Some("MyClass"),
            "{tag} X"
        );
    }
    // glue gap refuses (sg P5 n0); plain binds.
    let hits = match_pattern(Language::Php, "<?php\nuse\u{2028}MyClass;\n", "use $X;").unwrap();
    assert!(hits.is_empty(), "P5-glue: {hits:?}");
    let hits = match_pattern(Language::Php, "<?php\nuse MyClass;\n", "use $X;").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "P4-plain: {hits:?}");
}

/// f144f (143A-F6, phase144R grid T*): the go type root keyword→name gate —
/// sg REFUSES a comment (T1) or gap-junk run (T6 FEFF / T7 U+2028 / T8 A0)
/// between the `type` keyword and the name while every other comment
/// position binds (T2 name-internal, T3 before the tail, T5 lead+trail).
/// The pre-fix field walk was comment/junk-blind in the keyword→name gap.
#[test]
fn f144f_go_type_keyword_gap_gate() {
    for (cand, tag) in [
        ("package p\ntype /* c */ MyInt int\n", "T1-comment"),
        ("package p\ntype\u{FEFF}MyInt int\n", "T6-feff"),
        ("package p\ntype\u{2028}MyInt int\n", "T7-u2028"),
        ("package p\ntype\u{00A0}MyInt int\n", "T8-a0"),
    ] {
        let hits = match_pattern(Language::Go, cand, "type $N $T").unwrap();
        assert!(hits.is_empty(), "{tag}: must refuse: {hits:?}");
    }
    for (cand, tag, name) in [
        ("package p\ntype MyInt int\n", "T4-plain", "MyInt"),
        (
            "package p\ntype MyInt /* c */ int\n",
            "T3-pre-tail",
            "MyInt",
        ),
        (
            "package p\n// lead\ntype MyInt int // tail\n",
            "T5-lead-trail",
            "MyInt",
        ),
    ] {
        let hits = match_pattern(Language::Go, cand, "type $N $T").unwrap();
        assert_eq!(
            lines_of(&hits),
            vec![if tag == "T5-lead-trail" { 3 } else { 2 }],
            "{tag}: must bind: {hits:?}"
        );
        assert_eq!(
            hits[0].captures.get("N").map(String::as_str),
            Some(name),
            "{tag} N"
        );
    }
}

/// f144g (143A-F7, phase144R grid D*): the py del-slice colon-boundary
/// comment law — a comment BETWEEN the slice bounds/colons is sg trivia and
/// the slice binds O=d A=1 B=2 (D1 before `:`, D2 after `:`; pre-fix the
/// comment counted as a positional bound and the shapes refused), while the
/// comment in the START-bound position refuses (D3, sg rc1) and the
/// pre-`]` comment keeps binding (D4).
#[test]
fn f144g_py_del_slice_colon_comment_bind() {
    for (cand, tag) in [
        ("def f(d):\n    del d[1 # c\n:2]\n", "D1-before-colon"),
        ("def f(d):\n    del d[1: # c\n2]\n", "D2-after-colon"),
    ] {
        let hits = match_pattern(Language::Python, cand, "del $O[$A:$B]").unwrap();
        assert_eq!(lines_of(&hits), vec![2], "{tag}: must bind: {hits:?}");
        assert_eq!(
            hits[0].captures.get("O").map(String::as_str),
            Some("d"),
            "{tag} O"
        );
        assert_eq!(
            hits[0].captures.get("A").map(String::as_str),
            Some("1"),
            "{tag} A"
        );
        assert_eq!(
            hits[0].captures.get("B").map(String::as_str),
            Some("2"),
            "{tag} B"
        );
    }
    // start-bound position refuses (sg rc1; pre-fix this cell already
    // refused — the pin keeps the widened law from over-serving).
    let hits = match_pattern(
        Language::Python,
        "def f(d):\n    del d[ # c\n1:2]\n",
        "del $O[$A:$B]",
    )
    .unwrap();
    assert!(hits.is_empty(), "D3-start-comment: {hits:?}");
    // controls: pre-`]` comment and the plain slice keep binding.
    let hits = match_pattern(
        Language::Python,
        "def f(d):\n    del d[1:2 # c\n]\n",
        "del $O[$A:$B]",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "D4-pre-bracket: {hits:?}");
    let hits = match_pattern(
        Language::Python,
        "def f(d):\n    del d[1:2]\n",
        "del $O[$A:$B]",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "D5-plain: {hits:?}");
}

/// f144h (143A-F8, phase144R grid G*): the go semi-less `goto $L` spelling
/// binds per site sg-exactly (sg G1 n1 L=end; G5 n2) where the pre-fix
/// classifier admitted no lane and the shape failed closed at the query
/// level (rc2 structural fallback). The `;`-ful spelling keeps its
/// registered accepted-empty posture and the `break $L` sibling keeps
/// binding.
#[test]
fn f144h_go_goto_bare_binds() {
    let hits = match_pattern(
        Language::Go,
        "package p\nfunc f() {\n\tgoto end\nend:\n}\n",
        "goto $L",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3], "G1: {hits:?}");
    assert_eq!(
        hits[0].captures.get("L").map(String::as_str),
        Some("end"),
        "G1 L"
    );
    let hits = match_pattern(
        Language::Go,
        "package p\nfunc f() {\n\tgoto a\na:\n\tgoto b\nb:\n}\n",
        "goto $L",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3, 5], "G5-two-sites: {hits:?}");
    // registered sibling postures hold.
    let hits = match_pattern(
        Language::Go,
        "package p\nfunc f() {\n\tgoto end\nend:\n}\n",
        "goto $L;",
    )
    .unwrap();
    assert!(hits.is_empty(), "G3 semi-ful accepted-empty: {hits:?}");
    let hits = match_pattern(
        Language::Go,
        "package p\nfunc f() {\n\tfor {\n\t\tbreak outer\n\t}\nouter:\n}\n",
        "break $L",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![4], "G4 break control: {hits:?}");
}

/// f144i (143A-F9, phase144R grid C*/CJ*): the ts-only chain
/// receiver→`?.` comment gate — sg REFUSES `a /* c */?. b` under
/// tree-sitter-typescript (the named-wrapper grammar) and BINDS the same
/// spelling under tree-sitter-javascript (CJ1 js twin n1); comment AFTER
/// `?.` (C2) and plain (C3) keep binding. The pre-fix link decomposition
/// only vetoed a comment preceding the NAMED `optional_chain` wrapper —
/// the workspace's anonymous-token parse never matched that veto.
#[test]
fn f144i_ts_chain_receiver_comment_gate() {
    let hits = match_pattern(Language::TypeScript, "const x = a /* c */?. b;\n", "$A?.$B").unwrap();
    assert!(hits.is_empty(), "C1-ts: must refuse: {hits:?}");
    for (cand, tag) in [
        ("const x = a?. /* c */ b;\n", "C2-after"),
        ("const x = a?.b;\n", "C3-plain"),
    ] {
        let hits = match_pattern(Language::TypeScript, cand, "$A?.$B").unwrap();
        assert_eq!(lines_of(&hits), vec![1], "{tag}: {hits:?}");
    }
    // js twin binds (sg CJ1 n1) — the refusal is ts-only.
    let hits = match_pattern(Language::JavaScript, "const x = a /* c */?. b;\n", "$A?.$B").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "CJ1-js-twin: {hits:?}");
}

/// f144j (143A-F13 / CNR §51.c row 3 predicate FIRED, phase144R grid M*):
/// the php mixed-body namespace face — sg BINDS the exact-order spelling
/// `namespace X { const A = 1; $$B }` × `namespace X { const A = 1; f(); }`
/// with B=`f();` (M1, refuting the row's "sg binds nothing on every
/// mixed-body cell" premise) and binds NOTHING on the mixed-ORDER (M2),
/// zero-trailing (M3), two-trailing (M4), and prefix-mismatch (M5) faces.
/// The pre-fix template parsed only the bare-meta body: M1 was census-loud
/// rc2 and M2-M5 loud with it.
#[test]
fn f144j_php_mixed_namespace_body_exact_order() {
    let hits = match_pattern(
        Language::Php,
        "<?php\nnamespace X { const A = 1; f(); }\n",
        "namespace X { const A = 1; $$B }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "M1: {hits:?}");
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("f();"),
        "M1 B"
    );
    for (cand, tag) in [
        (
            "<?php\nnamespace X { f(); const A = 1; }\n",
            "M2-mixed-order",
        ),
        ("<?php\nnamespace X { const A = 1; }\n", "M3-zero-trailing"),
        (
            "<?php\nnamespace X { const A = 1; f(); g(); }\n",
            "M4-two-trailing",
        ),
        (
            "<?php\nnamespace X { const A = 2; f(); }\n",
            "M5-prefix-mismatch",
        ),
    ] {
        let hits = match_pattern(Language::Php, cand, "namespace X { const A = 1; $$B }").unwrap();
        assert!(hits.is_empty(), "{tag}: must refuse sg-silently: {hits:?}");
    }
    // plain bare-meta control keeps binding (M6).
    let hits = match_pattern(
        Language::Php,
        "<?php\nnamespace X { f(); }\n",
        "namespace X { $$B }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "M6-control: {hits:?}");
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("f();"),
        "M6 B"
    );
}

/// f144k (143B-F1, phase144R grid Q*): the cs qualified-name INTERNAL
/// token-gap law — sg skips gap junk at identifier↔`.` boundaries and binds
/// with the junk RETAINED in the capture (Q1-Q6: N=`Sys<U+2028>.IO` et al)
/// on the plain, static, and global lanes, while junk GLUED inside one
/// segment still refuses (Q7, sg cs_name_glue n0). The pre-fix plain-face
/// guard refused ANY interior junk wholesale (silent ok:true []).
#[test]
fn f144k_cs_using_internal_gap_bind() {
    for (_ch, tag, segs) in [
        ("\u{2028}", "Q1-u2028", "Sys\u{2028}.IO"),
        ("\u{0000}", "Q2-nul", "Sys\u{0000}.IO"),
        ("\u{FEFF}", "Q3-feff", "Sys\u{FEFF}.IO"),
        ("\u{001A}", "Q4-001a", "Sys\u{001A}.IO"),
        ("", "Q5-pre-name", "Sys.\u{2028}IO"),
    ] {
        let cand = format!("using {segs};\nclass K {{}}\n");
        let hits = match_pattern(Language::CSharp, &cand, "using $N;").unwrap();
        assert_eq!(lines_of(&hits), vec![1], "{tag}: must bind: {hits:?}");
        assert_eq!(
            hits[0].captures.get("N").map(String::as_str),
            Some(segs),
            "{tag}: junk RETAINED in the capture"
        );
    }
    // post-dot junk lead segment binds (Q6).
    let hits = match_pattern(
        Language::CSharp,
        "using Sys\u{2028}.tem;\nclass K {}\n",
        "using $N;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "Q6: {hits:?}");
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("Sys\u{2028}.tem"),
        "Q6 N"
    );
    // static + global lanes share the law (Q10/Q11).
    let hits = match_pattern(
        Language::CSharp,
        "using static Sys\u{2028}.IO;\nclass K {}\n",
        "using static $N;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "Q10-static: {hits:?}");
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("Sys\u{2028}.IO"),
        "Q10 N"
    );
    let hits = match_pattern(
        Language::CSharp,
        "global using Sys\u{2028}.IO;\nclass K {}\n",
        "global using $N;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "Q11-global: {hits:?}");
    // glue INSIDE one segment refuses (Q7); clean binds (Q8).
    let hits = match_pattern(
        Language::CSharp,
        "using Sys\u{2028}tem;\nclass K {}\n",
        "using $N;",
    )
    .unwrap();
    assert!(hits.is_empty(), "Q7-glue: {hits:?}");
    let hits = match_pattern(
        Language::CSharp,
        "using System.IO;\nclass K {}\n",
        "using $N;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "Q8-clean: {hits:?}");
    assert_eq!(
        hits[0].captures.get("N").map(String::as_str),
        Some("System.IO"),
        "Q8 N"
    );
}

/// f144l (143B-F2, phase144R grid Q12/Q13): the discriminating ALIAS-lane
/// pins — `using M<U+2028>= System.IO;` binds A=`M` on both engines and
/// `using M = System<U+2028>.IO;` binds T with the junk retained. These
/// cells kill the narrowed-guard mutant class: on `using Sys<U+001A>.IO;`
/// (f144k Q4) the 142-class guard refuses where sg binds.
#[test]
fn f144l_cs_using_alias_internal_gap() {
    let hits = match_pattern(
        Language::CSharp,
        "using M\u{2028}= System.IO;\nclass K {}\n",
        "using $A = $T;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "Q12: {hits:?}");
    assert_eq!(
        hits[0].captures.get("A").map(String::as_str),
        Some("M"),
        "Q12 A"
    );
    assert_eq!(
        hits[0].captures.get("T").map(String::as_str),
        Some("System.IO"),
        "Q12 T"
    );
    let hits = match_pattern(
        Language::CSharp,
        "using M = System\u{2028}.IO;\nclass K {}\n",
        "using $A = $T;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "Q13: {hits:?}");
    assert_eq!(
        hits[0].captures.get("T").map(String::as_str),
        Some("System\u{2028}.IO"),
        "Q13 T retains the internal junk"
    );
}

// ===========================================================================
// PASS 146 (r77 remediation, agent 146R): the 15 findings from 145A/145B —
// grids /tmp/phase146R/cases (104 cells, oracle ast-grep 0.45.2 pinned).
// ===========================================================================

/// f146a (145A-F1, grid a*): cs `goto $L;` binds PER SITE across the
/// candidate gap-junk class and comments (a1 clean, a2 comment, a3 U+2028;
/// a6 n2). The pre-fix subject answered rc2 fail-closed on every meta cell.
#[test]
fn f146a_cs_goto_meta_binds_per_site() {
    for (cand, tag) in [
        ("class C { void M() { goto end; } }", "a1-clean"),
        ("class C { void M() { goto /* c */ end; } }", "a2-comment"),
        ("class C { void M() { goto\u{2028}end; } }", "a3-u2028"),
    ] {
        let hits = match_pattern(Language::CSharp, cand, "goto $L;").unwrap();
        assert_eq!(lines_of(&hits), vec![1], "{tag}: sg binds n1: {hits:?}");
        assert_eq!(
            hits[0].captures.get("L").map(String::as_str),
            Some("end"),
            "{tag} L"
        );
    }
    let hits = match_pattern(
        Language::CSharp,
        "class C { void M() { goto end; if (x) { goto other; } } }",
        "goto $L;",
    )
    .unwrap();
    assert_eq!(hits.len(), 2, "a6: per-site n2");
    assert_eq!(
        hits[1].captures.get("L").map(String::as_str),
        Some("other"),
        "a6 L2"
    );
}

/// f146b (145A-F2, grid b*): cs `new $T($A);` binds the one-argument
/// object-creation STATEMENT (b1 clean, b2 comment junction, b3 semi-less,
/// b5 literal type) and refuses nested (b4), zero-arg (b6), two-arg (b7).
#[test]
fn f146b_cs_object_creation_binds_one_arg_statement() {
    let hits = match_pattern(
        Language::CSharp,
        "class C { void M() { new T(1); } }",
        "new $T($A);",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "b1: {hits:?}");
    assert_eq!(
        hits[0].captures.get("T").map(String::as_str),
        Some("T"),
        "b1 T"
    );
    assert_eq!(
        hits[0].captures.get("A").map(String::as_str),
        Some("1"),
        "b1 A"
    );
    for (cand, tag) in [
        (
            "class C { void M() { new /* c */ T(1); } }",
            "b2-comment-junction",
        ),
        ("class C { void M() { new T(1); } }", "b3-semi-less"),
    ] {
        let hits = match_pattern(
            Language::CSharp,
            cand,
            if tag == "b3-semi-less" {
                "new $T($A)"
            } else {
                "new $T($A);"
            },
        )
        .unwrap();
        assert_eq!(lines_of(&hits), vec![1], "{tag}: {hits:?}");
    }
    let hits = match_pattern(
        Language::CSharp,
        "class C { void M() { new T(1); } }",
        "new T($A);",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "b5 literal type: {hits:?}");
    assert_eq!(
        hits[0].captures.get("A").map(String::as_str),
        Some("1"),
        "b5 A"
    );
    for (cand, pattern, tag) in [
        (
            "class C { void M() { var x = new T(1); } }",
            "new $T($A);",
            "b4-nested",
        ),
        (
            "class C { void M() { new T(); } }",
            "new $T($A);",
            "b6-zero-arg",
        ),
        (
            "class C { void M() { new T(1, 2); } }",
            "new $T($A);",
            "b7-two-arg",
        ),
    ] {
        let hits = match_pattern(Language::CSharp, cand, pattern).unwrap();
        assert!(hits.is_empty(), "{tag}: sg refuses: {hits:?}");
    }
}

/// f146c (145A-F3, grid c*): php declaration-kind meta bodies bind ONE
/// member (function/class/trait/interface, `$B` and `$$B`), literal names
/// bind (c11/c12), the class Mixed exact-order face binds (c10), and
/// multi-member/empty candidates refuse (c2/c5/c6/c13). The served function
/// `$B` control (c3) keeps binding.
#[test]
fn f146c_php_decl_meta_bodies_bind_one_member() {
    for (pattern, cand, n_tag, name, body) in [
        (
            "function $N() { $$B }",
            "<?php\nfunction f() { g(); }",
            "c1-fn-multi",
            "f",
            "g();",
        ),
        (
            "class $N { $$B }",
            "<?php\nclass X { public $y; }",
            "c4-class-multi",
            "X",
            "public $y;",
        ),
        (
            "class $N { $B }",
            "<?php\nclass X { public $y; }",
            "c7-class-single",
            "X",
            "public $y;",
        ),
        (
            "trait $N { $$B }",
            "<?php\ntrait T { public $y; }",
            "c8-trait",
            "T",
            "public $y;",
        ),
        (
            "interface $N { $$B }",
            "<?php\ninterface I { public function f(); }",
            "c9-interface",
            "I",
            "public function f();",
        ),
        (
            "class X { $$B }",
            "<?php\nclass X { public $y; }",
            "c11-literal-name",
            "",
            "public $y;",
        ),
        (
            "function f() { $$B }",
            "<?php\nfunction f() { g(); }",
            "c12-literal-fn",
            "",
            "g();",
        ),
        (
            "class $N { const A = 1; $$B }",
            "<?php\nclass X { const A = 1; public $y; }",
            "c10-mixed",
            "X",
            "public $y;",
        ),
    ] {
        let hits = match_pattern(Language::Php, cand, pattern).unwrap();
        assert_eq!(lines_of(&hits), vec![2], "{n_tag}: sg binds n1: {hits:?}");
        if !name.is_empty() {
            assert_eq!(
                hits[0].captures.get("N").map(String::as_str),
                Some(name),
                "{n_tag} N"
            );
        }
        assert_eq!(
            hits[0].captures.get("B").map(String::as_str),
            Some(body),
            "{n_tag} B"
        );
    }
    for (pattern, cand, tag) in [
        (
            "function $N() { $$B }",
            "<?php\nfunction f() { g(); h(); }",
            "c2-fn-two-stmt",
        ),
        (
            "class $N { $$B }",
            "<?php\nclass X { public $y; public $z; }",
            "c5-class-two",
        ),
        ("class $N { $$B }", "<?php\nclass X { }", "c6-class-empty"),
        (
            "class $N { $$B }",
            "<?php\nclass X { const A = 1; public function f() { g(); } }",
            "c13-class-two",
        ),
    ] {
        let hits = match_pattern(Language::Php, cand, pattern).unwrap();
        assert!(hits.is_empty(), "{tag}: sg refuses: {hits:?}");
    }
    // served control: the function `$B` face keeps its pre-existing route (c3).
    let hits = match_pattern(
        Language::Php,
        "<?php\nfunction f() { g(); }",
        "function $N() { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "c3 control: {hits:?}");
}

/// f146d (145A-F4, grid d*): cs switch-STATEMENT meta body binds ONE section
/// at both spellings (d1 `$$B`, d2 `$B`, d6 literal subject) and refuses
/// empty (d4) and multi-section (d5) bodies plus the switch-EXPRESSION
/// candidate (d7 — the registered expression row must not cross-fire).
#[test]
fn f146d_cs_switch_statement_binds_one_section() {
    let sw1 = "class C { void M(int x) { switch (x) { case 1: break; } } }";
    for (pattern, tag) in [
        ("switch ($X) { $$B }", "d1-multi-spelling"),
        ("switch ($X) { $B }", "d2-single-spelling"),
    ] {
        let hits = match_pattern(Language::CSharp, sw1, pattern).unwrap();
        assert_eq!(lines_of(&hits), vec![1], "{tag}: {hits:?}");
        assert_eq!(
            hits[0].captures.get("X").map(String::as_str),
            Some("x"),
            "{tag} X"
        );
        assert_eq!(
            hits[0].captures.get("B").map(String::as_str),
            Some("case 1: break;"),
            "{tag} B"
        );
    }
    let hits = match_pattern(Language::CSharp, sw1, "switch (x) { $$B }").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "d6 literal subject: {hits:?}");
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("case 1: break;"),
        "d6 B"
    );
    for (cand, tag) in [
        ("class C { void M(int x) { switch (x) { } } }", "d4-empty"),
        ("class C { void M(int x) { switch (x) { case 1: break; case 2: break; default: break; } } }", "d5-three-sections"),
        ("class C { int M(int x) { return x switch { 1 => 2, _ => 3 }; } }", "d7-switch-expression"),
    ] {
        let hits = match_pattern(Language::CSharp, cand, "switch ($X) { $$B }").unwrap();
        assert!(hits.is_empty(), "{tag}: sg refuses: {hits:?}");
    }
}

/// f146e (145A-F5, grid e*): go root lanes — plain switch binds ONE case
/// clause at both spellings (e1/e2), refuses 2-clause (e3) and empty (e4)
/// bodies and the type-switch candidate (e14); `package $N` binds through a
/// comment gap (e5/e6); semi-less `import $X` binds single (e8) and grouped
/// (e9, X = the whole group text).
#[test]
fn f146e_go_switch_package_import_roots_bind() {
    let hits = match_pattern(
        Language::Go,
        "package main\nfunc f(x int) {\n\tswitch x {\n\tcase 1:\n\t\tg()\n\t}\n}\nfunc g() {}\n",
        "switch $X { $$B }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3], "e1: {hits:?}");
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("x"),
        "e1 X"
    );
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("case 1:\n\t\tg()\n"),
        "e1 B"
    );
    let hits = match_pattern(
        Language::Go,
        "package main\nfunc f(a bool) {\n\tswitch {\n\tcase a:\n\t\tg()\n\t}\n}\nfunc g() {}\n",
        "switch { $B }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3], "e2: {hits:?}");
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("case a:\n\t\tg()\n"),
        "e2 B"
    );
    for (cand, tag) in [
        (
            "package main\nfunc f(x int) {\n\tswitch x {\n\tcase 1:\n\t\tg()\n\tcase 2:\n\t\th()\n\t}\n}\nfunc g() {}\nfunc h() {}\n",
            "e3-two-clauses",
        ),
        ("package main\nfunc f(x int) {\n\tswitch x {\n\t}\n}\n", "e4-empty"),
        (
            "package main\nfunc f(i interface{}) {\n\tswitch v := i.(type) {\n\tcase int:\n\t\t_ = v\n\t}\n}\n",
            "e14-type-switch",
        ),
    ] {
        let hits = match_pattern(Language::Go, cand, "switch $X { $$B }").unwrap();
        assert!(hits.is_empty(), "{tag}: sg refuses: {hits:?}");
    }
    for (cand, tag) in [
        ("package main\nfunc f() {}\n", "e5-clean"),
        ("package /* c */ main\nfunc f() {}\n", "e6-comment-gap"),
    ] {
        let hits = match_pattern(Language::Go, cand, "package $N").unwrap();
        assert_eq!(lines_of(&hits), vec![1], "{tag}: {hits:?}");
        assert_eq!(
            hits[0].captures.get("N").map(String::as_str),
            Some("main"),
            "{tag} N"
        );
    }
    let hits = match_pattern(
        Language::Go,
        "package main\nimport \"fmt\"\nfunc f() { fmt.Print() }\n",
        "import $X",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "e8: {hits:?}");
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("\"fmt\""),
        "e8 X"
    );
    let hits = match_pattern(
        Language::Go,
        "package main\nimport (\n\t\"a\"\n\t\"b\"\n)\n",
        "import $X",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "e9: {hits:?}");
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("(\n\t\"a\"\n\t\"b\"\n)"),
        "e9 X = the whole group"
    );
}

/// f146f (145A-F6, grid e10-e13): go `if $C { $$B }` binds the WHOLE body
/// text (e10 single-stmt, e11 multi-stmt), refuses the empty body (e12), and
/// the literal-cond spelling binds (e13).
#[test]
fn f146f_go_if_multi_body_binds_whole_body() {
    let hits = match_pattern(
        Language::Go,
        "package main\nfunc f(x bool) {\n\tif x { g() }\n}\nfunc g() {}\n",
        "if $C { $$B }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3], "e10: {hits:?}");
    assert_eq!(
        hits[0].captures.get("C").map(String::as_str),
        Some("x"),
        "e10 C"
    );
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("g()"),
        "e10 B"
    );
    let hits = match_pattern(
        Language::Go,
        "package main\nfunc f(x bool) {\n\tif x { g(); h() }\n}\nfunc g() {}\nfunc h() {}\n",
        "if $C { $$B }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3], "e11: {hits:?}");
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("g(); h()"),
        "e11 B = the whole body"
    );
    let hits = match_pattern(
        Language::Go,
        "package main\nfunc f(x bool) {\n\tif x { }\n}\n",
        "if $C { $$B }",
    )
    .unwrap();
    assert!(hits.is_empty(), "e12 empty body refuses: {hits:?}");
    let hits = match_pattern(
        Language::Go,
        "package main\nfunc f(x bool) {\n\tif x { g() }\n}\nfunc g() {}\n",
        "if x { $$B }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![3], "e13 literal cond: {hits:?}");
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("g()"),
        "e13 B"
    );
}

/// f146g (145A-F7, grid f*): js `$$` arm bodies — the else-if face binds all
/// four captures (f1), refuses multi-statement (f2) and empty (f3) arms,
/// emits PER LEVEL on chains (f4 n2), binds plain-else (f5) and plain-if
/// (f6); the single-`$` control keeps its served route (f7).
#[test]
fn f146g_js_dollar_dollar_arm_bodies_bind() {
    let hits = match_pattern(
        Language::JavaScript,
        "if (a) { b(); } else if (c) { d(); }",
        "if ($A) { $$B } else if ($C) { $$D }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "f1: {hits:?}");
    assert_eq!(
        hits[0].captures.get("A").map(String::as_str),
        Some("a"),
        "f1 A"
    );
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("b();"),
        "f1 B"
    );
    assert_eq!(
        hits[0].captures.get("C").map(String::as_str),
        Some("c"),
        "f1 C"
    );
    assert_eq!(
        hits[0].captures.get("D").map(String::as_str),
        Some("d();"),
        "f1 D"
    );
    for (cand, tag) in [
        (
            "if (a) { b(); c(); } else if (cc) { d(); }",
            "f2-two-stmt-arm",
        ),
        ("if (a) { } else if (c) { }", "f3-empty-arms"),
    ] {
        let hits = match_pattern(
            Language::JavaScript,
            cand,
            "if ($A) { $$B } else if ($C) { $$D }",
        )
        .unwrap();
        assert!(hits.is_empty(), "{tag}: sg refuses: {hits:?}");
    }
    let hits = match_pattern(
        Language::JavaScript,
        "if (a) { b(); } else if (c) { d(); } else if (e) { g(); }",
        "if ($A) { $$B } else if ($C) { $$D }",
    )
    .unwrap();
    assert_eq!(hits.len(), 2, "f4: sg emits per level: {hits:?}");
    let hits = match_pattern(
        Language::JavaScript,
        "if (a) { b(); } else { d(); }",
        "if ($A) { $$B } else { $$D }",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "f5 plain else: {hits:?}");
    assert_eq!(
        hits[0].captures.get("B").map(String::as_str),
        Some("b();"),
        "f5 B"
    );
    let hits = match_pattern(Language::JavaScript, "if (a) { b(); }", "if ($C) { $$B }").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "f6 plain if: {hits:?}");
    let hits = match_pattern(
        Language::JavaScript,
        "if (a) { b(); } else if (c) { d(); }",
        "if ($C) { $A } else if ($D) { $B }",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "f7 single-$ control stays served: {hits:?}"
    );
}

/// f146h (145A-F8, grid g*): the go keyword→body junction gate — comments
/// AND the junk class (U+2028/FEFF/A0) between `for`/`go`/`defer` and their
/// first child refuse (g1-g4/g7/g8, sg rc1) while clean junctions (g5/g10),
/// argument comments (g6), and the `func` control (g9, sg binds both) keep
/// binding.
#[test]
fn f146h_go_keyword_body_junction_comments_refuse() {
    for (cand, tag) in [
        ("package main\nfunc main() { go /* c */ g(1) }\nfunc g(x int) {}\n", "g1-go-comment"),
        ("package main\nfunc main() { defer /* c */ g(1) }\nfunc g(x int) {}\n", "g2-defer-comment"),
        ("package main\nfunc main(xs []int) {\n\tfor /* c */ i := range xs { g() }\n}\nfunc g() {}\n", "g3-for-comment"),
        ("package main\nfunc main() { go\u{2028}g(1) }\nfunc g(x int) {}\n", "g4-go-u2028"),
        ("package main\nfunc main() { go\u{FEFF}g(1) }\nfunc g(x int) {}\n", "g7-go-feff"),
        ("package main\nfunc main() { go\u{00A0}g(1) }\nfunc g(x int) {}\n", "g8-go-a0"),
    ] {
        let pattern = if tag.starts_with("g3") { "for $I := range $X { $$B }" } else if tag.starts_with("g2") { "defer $F($A)" } else { "go $F($A)" };
        let hits = match_pattern(Language::Go, cand, pattern).unwrap();
        assert!(hits.is_empty(), "{tag}: sg refuses (rc1): {hits:?}");
    }
    for (cand, pattern, tag) in [
        (
            "package main\nfunc main() { go g(1) }\nfunc g(x int) {}\n",
            "go $F($A)",
            "g5-clean",
        ),
        (
            "package main\nfunc main() { go g(/* c */ 1) }\nfunc g(x int) {}\n",
            "go $F($A)",
            "g6-arg-comment",
        ),
        (
            "package main\nfunc /* c */ f() { g() }\nfunc g() {}\n",
            "func $N() { $$B }",
            "g9-func-control",
        ),
        (
            "package main\nfunc main() { defer g(1) }\nfunc g(x int) {}\n",
            "defer $F($A)",
            "g10-defer-clean",
        ),
    ] {
        let hits = match_pattern(Language::Go, cand, pattern).unwrap();
        assert_eq!(lines_of(&hits), vec![2], "{tag}: must bind: {hits:?}");
    }
}

/// f146i (145A-F9 + 145B-F2, grid h*): the cs using-var head gate — comments
/// in the keyword→name head refuse (h1/h2) while comment after the name
/// (h3), comment at the initializer (h4), clean (h5), and junk INSIDE the
/// initializer (uc: sg binds with the junk retained) keep binding.
#[test]
fn f146i_cs_using_var_head_gate_scoped_to_the_head() {
    for (cand, tag) in [
        (
            "class C { void M() { using var /* c */ s = g(); } }",
            "h1-comment-var-name",
        ),
        (
            "class C { void M() { using /* c */ var s = g(); } }",
            "h2-comment-using-var",
        ),
    ] {
        let hits = match_pattern(Language::CSharp, cand, "using var $N = $E;").unwrap();
        assert!(hits.is_empty(), "{tag}: sg refuses (rc1): {hits:?}");
    }
    for (cand, tag) in [
        (
            "class C { void M() { using var s /* c */ = g(); } }",
            "h3-comment-name-eq",
        ),
        (
            "class C { void M() { using var s = /* c */ g(); } }",
            "h4-comment-eq-init",
        ),
        ("class C { void M() { using var s = g(); } }", "h5-clean"),
    ] {
        let hits = match_pattern(Language::CSharp, cand, "using var $N = $E;").unwrap();
        assert_eq!(lines_of(&hits), vec![1], "{tag}: must bind: {hits:?}");
    }
    // 145B-F2: junk INSIDE the initializer binds sg-exactly (junk retained).
    let hits = match_pattern(
        Language::CSharp,
        "class C { void M() { using var s = g\u{2028}(); } }",
        "using var $N = $E;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "uc-initializer-junk: {hits:?}");
    assert_eq!(
        hits[0].captures.get("E").map(String::as_str),
        Some("g\u{2028}()"),
        "uc E retains the junk"
    );
}

/// f146j (145A-F10, grid i*): the cs nested-head seam gate is symmetrical —
/// a PATTERN-side U+2028 seam (i1) refuses where sg answers valid-empty,
/// while the candidate-side control (i2, seam-of-record) and the clean
/// pattern (i4) keep their record postures.
#[test]
fn f146j_cs_seam_gate_covers_the_pattern_side() {
    let fixd = "class C { void M() { fixed (char* p = s) { checked { *p = 'x'; } } } }";
    let hits = match_pattern(
        Language::CSharp,
        fixd,
        "fixed ($D) {\u{2028} checked { $B } }",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "i1 pattern-side U+2028 seam: sg valid-empty: {hits:?}"
    );
    let hits = match_pattern(
        Language::CSharp,
        "class C { void M() { fixed (char* p = s) {\u{2028} checked { *p = 'x'; } } } }",
        "fixed ($D) { checked { $B } }",
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "i2 candidate-side control (seam of record): {hits:?}"
    );
    let hits = match_pattern(Language::CSharp, fixd, "fixed ($D) { checked { $B } }").unwrap();
    assert_eq!(lines_of(&hits), vec![1], "i4 clean control: {hits:?}");
}

/// f146k (145A-F11, grid j*): php `use $X;` refuses the group-use candidate
/// (j1, sg rc1) while the plain candidate (j4) and the literal group-use
/// pattern (j2) keep binding.
#[test]
fn f146k_php_use_meta_refuses_group_use_candidate() {
    let hits = match_pattern(Language::Php, "<?php\nuse My\\{A, B};", "use $X;").unwrap();
    assert!(
        hits.is_empty(),
        "j1 group-use candidate: sg refuses: {hits:?}"
    );
    let hits = match_pattern(Language::Php, "<?php\nuse My\\Ns;", "use $X;").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "j4 control: {hits:?}");
    let hits = match_pattern(Language::Php, "<?php\nuse My\\{A, B};", "use My\\{A, B};").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![2],
        "j2 literal pattern control: {hits:?}"
    );
}

/// f146l (145B-F1, the TRUE root): the byte-prefilter literal must never
/// carry LAYOUT — `required_pattern_literal("namespace A { f(); $B }")`
/// returned `namespace A { f` and the memmem prefilter silently dropped
/// every pretty-printed namespace body the walk answers (sg n1, k1/k2/k6/k7).
#[test]
fn f146l_required_literal_never_carries_layout() {
    for pattern in ["namespace A { f(); $B }", "namespace A { f(); $$B }"] {
        let lit =
            ast_sgrep_lang::required_pattern_literal(pattern).expect("prefilter literal present");
        assert!(
            !lit.chars().any(char::is_whitespace),
            "{pattern}: literal must be layout-free, got {lit:?}"
        );
    }
}

/// f146m (145B-F4, grid m*): cs literal qualified-name patterns obey the
/// per-segment law — boundary junk (m1) and comment gaps (m3) bind sg-exactly
/// while glue-inside-one-segment still refuses (m2, sg n0).
#[test]
fn f146m_cs_literal_qname_segment_law() {
    for (cand, tag) in [
        (
            "class C { }\nusing Sys\u{2028}.IO;".to_string(),
            "m1-u2028-boundary",
        ),
        (
            "class C { }\nusing Sys /* c */ .IO;".to_string(),
            "m3-comment-gap",
        ),
    ] {
        let hits = match_pattern(Language::CSharp, &cand, "using Sys.IO;").unwrap();
        assert_eq!(lines_of(&hits), vec![2], "{tag}: sg binds n1: {hits:?}");
    }
    let hits = match_pattern(
        Language::CSharp,
        "class C { }\nusing Sy\u{2028}s.IO;",
        "using Sys.IO;",
    )
    .unwrap();
    assert!(hits.is_empty(), "m2 glue refuses: {hits:?}");
    let hits = match_pattern(
        Language::CSharp,
        "using static Sys\u{2028}.IO;",
        "using static Sys.IO;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![1], "m6 static lane: {hits:?}");
    let hits = match_pattern(
        Language::CSharp,
        "using static Sys/* c */.IO;",
        "using static Sys/* c */.IO;",
    )
    .unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![1],
        "z8 pattern-side comment face: sg binds n1: {hits:?}"
    );
}

/// f146n (145B-F3, grid l*): sg's php use-head gap law admits U+0000 at the
/// META faces — kind-less (l1) and the kind-clause name gap (l3) bind with
/// the NUL skipped and the capture stripped — while the LITERAL pattern
/// (`use Foo;` × NUL candidate, l4) keeps refusing.
#[test]
fn f146n_php_meta_use_head_admits_nul() {
    let hits = match_pattern(Language::Php, "<?php\nuse\u{0}Foo;", "use $X;").unwrap();
    assert_eq!(lines_of(&hits), vec![2], "l1: {hits:?}");
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("Foo"),
        "l1 X stripped"
    );
    let hits = match_pattern(
        Language::Php,
        "<?php\nuse function\u{0}Foo;",
        "use function $X;",
    )
    .unwrap();
    assert_eq!(lines_of(&hits), vec![2], "l3 kind-clause: {hits:?}");
    assert_eq!(
        hits[0].captures.get("X").map(String::as_str),
        Some("Foo"),
        "l3 X stripped"
    );
    let hits = match_pattern(Language::Php, "<?php\nuse\u{0}Foo;", "use Foo;").unwrap();
    assert!(hits.is_empty(), "l4 literal pattern refuses: {hits:?}");
}

/// f147c (146E-F1 law of record, lang level): a LINE BREAK between the
/// statement-head keyword and its operand roots the pattern as TWO AST
/// nodes, and sg 0.45.2 refuses the parse (rc8 "Multiple AST nodes are
/// detected"; oracle grid /tmp/phase147R, py/js/go nl+crlf cells). The
/// census keeps the class loud and the walk never serves rows. Same-line
/// layout keeps the sg-ACCEPTED binding faces: multi-space, tab, a lone CR
/// (not a line terminator in the py grammar), leading whitespace.
#[test]
fn f147c_statement_head_newline_seam_law() {
    let py = "def greet():\n    return msg\n";
    let js = "function greet() {\n  return msg;\n}\n";
    let go = "package main\n\nfunc greet() string {\n\treturn msg\n}\n";
    for (lang, src, pat, tag) in [
        (Language::Python, py, "return \n$A", "py-nl"),
        (Language::Python, py, "return \r\n$A", "py-crlf"),
        (Language::Python, py, "return \n\n$A", "py-nl2"),
        (Language::Python, py, "return \n $A", "py-nl-sp"),
        (Language::JavaScript, js, "return \n$A", "js-nl"),
        (Language::JavaScript, js, "return \r\n$A", "js-crlf"),
        (Language::Go, go, "return \n$A", "go-nl"),
        (Language::Go, go, "return \r\n$A", "go-crlf"),
    ] {
        assert!(
            needs_ast_grep_fallback(pat),
            "{tag}: sg rc8 multi-root must keep the loud fail-closed class"
        );
        assert!(
            match_pattern(lang, src, pat).unwrap().is_empty(),
            "{tag}: sg refuses to parse; the walk must not serve rows"
        );
    }
    for (lang, src, pat, tag) in [
        (Language::Python, py, "return $A", "py-sp1"),
        (Language::Python, py, "return  $A", "py-sp2"),
        (Language::Python, py, "return\t$A", "py-tab"),
        (Language::Python, py, "return \r$A", "py-cr"),
        (Language::Python, py, " return $A", "py-lead"),
    ] {
        assert!(
            !needs_ast_grep_fallback(pat),
            "{tag}: sg accepts the same-line face; must stay answerable"
        );
        assert_eq!(
            lines_of(&match_pattern(lang, src, pat).unwrap()),
            vec![2],
            "{tag}: sg binds the return row"
        );
    }
    // The 146E prefilter hypothesis is recorded as disproven at the API:
    // the repro pattern yields NO prefilter literal on any path.
    assert_eq!(
        ast_sgrep_lang::required_pattern_literal("return \n$A"),
        None
    );
}
