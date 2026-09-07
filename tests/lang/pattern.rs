use ast_sgrep_lang::{
    match_pattern, needs_ast_grep_fallback, native_pattern_answerable, Language,
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
    assert!(hits[0].excerpt.contains("function keep"), "{:?}", hits[0].excerpt);

    let swift = "func greet(name: String) -> String {\n    return name\n}\n\nfunc run() {\n    let _ = greet(name: \"x\")\n}\n";
    let hits = match_pattern(Language::Swift, swift, "func $A() { $$$B }").unwrap();
    assert_eq!(
        hits.len(),
        1,
        "swift return-typed func must not match: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    assert!(hits[0].excerpt.contains("func run"), "{:?}", hits[0].excerpt);
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
    assert!(
        match_pattern(Language::Rust, source, "zzz_no_such_content").unwrap().is_empty()
    );
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
    let source = "fn dup(dup: u32) -> u32 {\n    dup\n}\n\nfn other(name: u32) -> u32 {\n    name\n}\n";
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
    let source = "fn main() {\n    let v = Some(7).unwrap_or(7);\n    let w = Some(7).unwrap_or(9);\n}\n";
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
    let hits =
        match_pattern(Language::TypeScript, source, "function $A($B) { return $A; }").unwrap();
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
    assert_eq!(hits[1].captures.get("A").map(String::as_str), Some("helper"));
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
#[test]
fn registered_fail_closed_spellings_stay_fail_closed() {
    for pattern in [
        "let $A = $B",
        "fn $A($$$B) -> $C { $$$D }",
        "RETURN $A",
        "return $B// noteA",
        "def $A($B):\n    return $C",
        "int $A($B) { $$$C }",
        "fun $A() { $$$B }",
        "if ($COND) { $A; $B }",
        "$)(",
    ] {
        assert!(needs_ast_grep_fallback(pattern), "{pattern} must stay fail-closed");
        assert!(
            match_pattern(Language::Rust, "fn foo() {}", pattern)
                .unwrap()
                .is_empty(),
            "{pattern} must match nothing natively"
        );
    }
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
    let source = "def greet(name):\n    return name\nr1 = greet(\"world\")\nr2 = greet(  \"world\"  )\n";
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
    assert!(
        match_pattern(Language::Python, source, "$a = 1")
            .unwrap()
            .is_empty()
    );
}

/// v2 row 5: lowercase in a declaration-name position with a canonical `$$$`
/// body. sg: accepted-empty; the held-out literal control `def a(x): $$$B`
/// MATCHES the same def, so the `$` prefix alone kills the match — error-node
/// semantics, refuting the pass-34 literal-ident reading.
#[test]
fn metavar_v2_lowercase_decl_ingress_native_zero_hits() {
    assert!(!needs_ast_grep_fallback("def $a(x): $$$B"));
    let source = "def a(x):\n    a = 1\n    return a\n";
    assert!(
        match_pattern(Language::Python, source, "def $a(x): $$$B")
            .unwrap()
            .is_empty()
    );
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
    assert!(
        match_pattern(Language::Python, source, "greet($Ü)")
            .unwrap()
            .is_empty()
    );
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
    let docstring_src = "\"\"\"module docstring\nline two\n\"\"\"\n# a comment\nz = greet(\"world\")\n";
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
        assert!(
            match_pattern(Language::Rust, "fn x() {}\n", pattern)
                .unwrap()
                .is_empty()
        );
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
    assert!(
        match_pattern(Language::Python, source, "def $a(x): $$$B")
            .unwrap()
            .is_empty()
    );
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
    assert_eq!(lines_of(&uni), vec![1u32], "single-single unification survives");
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
    assert_eq!(hits[0].captures.get("O").map(String::as_str), Some("System"));
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
    let src = "fn area(r: i32) -> i32 {\n    r * r\n}\nfn main() {\n    let d = area( /* mid */ 9);\n}\n";
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
    let src = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\nfn main() {\n    let r = add(1, 2);\n}\n";
    let hits = match_pattern(Language::Rust, src, "\u{feff}add(1, 2)").unwrap();
    assert_eq!(
        lines_of(&hits),
        vec![5u32],
        "BOM-led pattern must match like its bare twin: {:?}",
        hits.iter().map(|h| &h.excerpt).collect::<Vec<_>>()
    );
    let bare = match_pattern(Language::Rust, src, "add(1, 2)").unwrap();
    assert_eq!(lines_of(&hits), lines_of(&bare), "BOM twin must equal the bare set");
}

/// PASS 60 (fuzz F26-0601 family): a PATTERN-side comment inside an argument
/// container is a REQUIRED SLOT (sg): it must not widen the match to
/// comment-less sites, while text-free comment variants still match.
#[test]
fn pass60_pattern_comment_is_a_required_slot() {
    // Slot absent in source: no match (pre-pass-60 the whole-text prefilter
    // masked this over-match; the prefilter must stay sound without it).
    let src = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\nfn main() {\n    let s = add(1, 2);\n}\n";
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
#[test]
fn pass60_registered_comment_and_statement_rows_stay_fail_closed() {
    for pattern in [
        "return $B// noteA",
        "foo($A) # note",
        "$A + $A# note",
        "let $A = $B",
        "int $A($B) { $$$C }",
        "RETURN $A",
    ] {
        assert!(needs_ast_grep_fallback(pattern), "{pattern} must stay fail-closed");
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
    assert!(!native_pattern_answerable(Language::Python, "$A + $A# note"));
    // misrepresentative java template: `$O.IF ($A) { $B }` parses as a
    // `block`, not the call the pattern text advertises — unanswerable.
    assert!(!native_pattern_answerable(Language::Java, "$O.IF ($A) { $B }"));
    // dup-meta family stays answerable (pass-51 faces keep serving)
    assert!(native_pattern_answerable(Language::Python, "$A == $A == $A"));
    assert!(native_pattern_answerable(Language::Python, "foo($A, bar($A))"));
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
    assert_eq!(derive[0].captures.get("A").map(String::as_str), Some("Debug"));
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
    assert!(hits.is_empty(), "raw-string metavar must bind nothing: {hits:?}");
}

#[test]
fn f62_1_c_preprocessor_templates_match() {
    let src = "#include <a.h>\n#include \"b.h\"\n#define MAX 3\n#include <c.h>\n";
    let includes = match_pattern(Language::C, src, "#include $X").unwrap();
    assert_eq!(lines_of(&includes), vec![1u32, 2, 4], "{includes:?}");
    let defines = match_pattern(Language::C, src, "#define $X $Y").unwrap();
    assert_eq!(lines_of(&defines), vec![3u32], "{defines:?}");
    assert_eq!(defines[0].captures.get("X").map(String::as_str), Some("MAX"));
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
    let py = "def f(cond):\n    if cond:\n        raise ValueError('bad')\n    yield 1\n    yield 2\n";
    let raises = match_pattern(Language::Python, py, "raise $A").unwrap();
    assert_eq!(lines_of(&raises), vec![3u32], "{raises:?}");
    assert_eq!(raises[0].captures.get("A").map(String::as_str), Some("ValueError('bad')"));
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
    assert_eq!(wild[0].captures.get("O").map(String::as_str), Some("arr[0]"));
    assert_eq!(wild[0].captures.get("M").map(String::as_str), Some("len"));
    assert_eq!(wild[2].captures.get("O").map(String::as_str), Some("vec![1, 2]"));

    let named = match_pattern(Language::Rust, src, "$A.$B($$$C)").unwrap();
    assert_eq!(lines_of(&named), vec![2u32, 3, 4], "{named:?}");

    let dup = match_pattern(Language::Rust, src, "$O.$O($$$A)").unwrap();
    assert!(dup.is_empty(), "same-name chains must stay rejected: {dup:?}");
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
    assert_eq!(lines_of(&n_slot), vec![5u32], "post-comma slot must answer only the /* n */ line: {n_slot:?}");
    let mid_slot = match_pattern(Language::Rust, src, "calc(1 /* mid */, 2)").unwrap();
    assert_eq!(lines_of(&mid_slot), vec![4u32], "glued slot keeps its own face: {mid_slot:?}");
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
    let py_raise =
        match_pattern(Language::Python, "def g():\n    raise ValueError\n", "raise $A").unwrap();
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
    assert_eq!(klass_lines(&three), vec![19u32, 20, 22, 23, 24], "{three:?}");

    let dup_src = "fn main() {\n    let y = Y;\n    let a = y.y().y();\n    let b = y.y().z();\n}\n";
    let dup = match_pattern(Language::Rust, dup_src, "$O.$O($$$A).$O($$$B)").unwrap();
    assert_eq!(lines_of(&dup), vec![3u32], "same-name veto: only y.y().y(): {dup:?}");
    let two = match_pattern(Language::Rust, dup_src, "$O.$M($$$A)").unwrap();
    // sg answers BOTH chain nodes per line (JSON probe 0.45.2: 4 rows —
    // outer cols 12-21 with O=y.y(), inner cols 12-17 with O=y); the subject
    // keeps that exact multiplicity on the two-segment contract.
    assert_eq!(lines_of(&two), vec![3u32, 3, 4, 4], "two-segment contract unchanged: {two:?}");

    let py = "class Alpha:\n    def first(self):\n        return Beta()\n\nclass Beta:\n    def second(self):\n        return Gamma()\n\n    def second_more(self, extra):\n        return Gamma()\n\nclass Gamma:\n    def third(self):\n        return 3\n\ndef main():\n    alpha = Alpha()\n    x = alpha.first().second()\n    y = alpha.first().third()\n    b = Beta()\n    z = b.second().third()\n    dup = b.second().second()\n    deep = alpha.first().second().third()\n    argd = alpha.first().second_more(9)\n";
    let py_three = match_pattern(Language::Python, py, "$O.$M1($$$A).$M2($$$B)").unwrap();
    assert_eq!(klass_lines(&py_three), vec![18u32, 19, 21, 22, 23, 24], "{py_three:?}");
    let py_dup = match_pattern(Language::Python, py, "$O.$O($$$A).$O($$$B)").unwrap();
    assert!(py_dup.is_empty(), "same-name three-seg must veto: {py_dup:?}");

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
    assert_eq!(ifdef[0].captures.get("A").map(String::as_str), Some("FEATURE"));
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
    assert!(endif.is_empty(), "#endif (sg-empty) must not answer: {endif:?}");

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
    assert_eq!(user[0].captures.get("N").map(String::as_str), Some("session.user"));

    let hello = match_pattern(Language::Ruby, rb, "\"hello-#{$A}-bye\"").unwrap();
    assert_eq!(lines_of(&hello), vec![3u32], "{hello:?}");
    assert_eq!(hello[0].captures.get("A").map(String::as_str), Some("user.name"));

    let serial = match_pattern(Language::Ruby, rb, "\"##{$D}\"").unwrap();
    assert_eq!(lines_of(&serial), vec![4u32], "{serial:?}");
    assert_eq!(serial[0].captures.get("D").map(String::as_str), Some("serial"));

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

    let ts = "const maybe = { load: () => 1 };\nconst v1 = maybe.load();\nconst v2 = maybe?.load();\n";
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
    assert_eq!(lines_of(&match_pattern(Language::Java, java, "break").unwrap()), vec![7u32]);
    assert_eq!(lines_of(&match_pattern(Language::Java, java, "break;").unwrap()), vec![7u32]);
    assert_eq!(lines_of(&match_pattern(Language::Java, java, "continue").unwrap()), vec![4u32]);
    assert_eq!(lines_of(&match_pattern(Language::Java, java, "continue;").unwrap()), vec![4u32]);
    assert_eq!(lines_of(&match_pattern(Language::Java, java, "throw").unwrap()), vec![9u32]);

    // csharp bare heads (sg: break {8}/{7}, continue {4}, throw kind-level
    // over every throw_statement incl. `throw new ...`).
    let cs = "class Store {\n    void Load() {\n        throw new SystemException();\n        throw new Exception(\"bad\");\n    }\n    int Pick() {\n        if (true) { return 1; }\n        break\n    }\n}\n";
    let cs2 = "class C {\n    void M() {\n        for (int i = 0; i < 3; i++) {\n            continue;\n        }\n        while (true) {\n            break;\n        }\n        throw new Exception();\n    }\n}\n";
    assert_eq!(lines_of(&match_pattern(Language::CSharp, cs, "break").unwrap()), vec![8u32]);
    assert_eq!(lines_of(&match_pattern(Language::CSharp, cs2, "break").unwrap()), vec![7u32]);
    assert_eq!(lines_of(&match_pattern(Language::CSharp, cs2, "continue").unwrap()), vec![4u32]);
    assert_eq!(lines_of(&match_pattern(Language::CSharp, cs, "throw").unwrap()), vec![3u32, 4]);
    assert_eq!(lines_of(&match_pattern(Language::CSharp, cs2, "throw").unwrap()), vec![9u32]);

    // rust break is kind-level over `break 9;` and `break;` (sg {3,6}).
    let rs = "fn f() {\n    loop {\n        break 9;\n    }\n    loop {\n        break;\n    }\n}\n";
    assert_eq!(lines_of(&match_pattern(Language::Rust, rs, "break").unwrap()), vec![3u32, 6]);

    // python bare yield / raise (sg kind-level {2,3,4} / {7,8}).
    let py = "def gen():\n    yield\n    yield 1\n    yield 2\n\ndef risky():\n    raise\n    raise ValueError\n\ndef loopy():\n    for i in range(3):\n        break\n    return i\n";
    assert_eq!(lines_of(&match_pattern(Language::Python, py, "yield").unwrap()), vec![2u32, 3, 4]);
    assert_eq!(lines_of(&match_pattern(Language::Python, py, "raise").unwrap()), vec![7u32, 8]);
    assert_eq!(lines_of(&match_pattern(Language::Python, py, "raise $A").unwrap()), vec![8u32]);

    // Registered pass-63 js faces stay green.
    let js = "function loopy() {\n  for (let i = 0; i < 3; i++) {\n    if (i === 1) { break; }\n    if (i === 2) { continue; }\n  }\n}\n";
    assert_eq!(lines_of(&match_pattern(Language::JavaScript, js, "break").unwrap()), vec![3u32]);
    assert_eq!(lines_of(&match_pattern(Language::JavaScript, js, "continue").unwrap()), vec![4u32]);
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
        match_pattern(Language::TypeScript, opt_src, "$O.$M($$$A)").unwrap().is_empty(),
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
    assert_eq!(by_line(3).captures.get("M").map(String::as_str), Some("load"));
    assert_eq!(by_line(6).captures.get("O").map(String::as_str), Some("cfg"));
    assert_eq!(by_line(6).captures.get("M").map(String::as_str), Some("get"));
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
    let rows: Vec<(u32, &str)> =
        hits.iter().map(|h| (h.line_start, h.excerpt.as_str())).collect();
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
        by(2, "maybe()?.load()").captures.get("O").map(String::as_str),
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
    let plain_hits =
        match_pattern(Language::TypeScript, F64_7_TS_CORPUS, "$O.$M($$$A)").unwrap();
    assert_eq!(lines_of(&plain_hits), vec![1u32, 4, 5, 7, 8, 9]);
    let first = &plain_hits[0];
    let o = first.captures.get("O").map(String::as_str).unwrap();
    let m = first.captures.get("M").map(String::as_str).unwrap();
    let rewrite = format!("log({o}, {m})");
    assert_eq!(rewrite, "log(a, b)", "plain face must rewrite to sg's bytes");
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
    let opt_hits =
        match_pattern(Language::TypeScript, F64_7_TS_CORPUS, "$O?.$M($$$A)").unwrap();
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
    assert!(match_pattern(Language::Python, "y = plain.value()\n", "$O?.$M($$$A)")
        .unwrap()
        .is_empty());
    assert!(match_pattern(Language::Rust, "fn f() {\n    let y = a?.b();\n}\n", "$O?.$M($$$A)")
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
        let ranges: Vec<(usize, usize)> =
            hits.iter().map(|h| (h.byte_start, h.byte_end)).collect();
        // The sound pre-order invariant: starts never decrease; on a tie the
        // OUTER (longer) range comes first (parents precede children); and no
        // byte range repeats. The duplicate clause is the dedup contract's
        // mutation kill cell: same-span statement/identifier pairs (the
        // bare `alpha` expression_statement spans exactly its identifier)
        // answer once, so the drop-dedup mutant doubles a range and dies.
        assert!(
            ranges.windows(2).all(|w| {
                w[0].0 < w[1].0 || (w[0].0 == w[1].0 && w[0].1 > w[1].1)
            }),
            "{pattern} emission must stay in pre-order and duplicate-free: {ranges:?}"
        );
        let unique: std::collections::HashSet<(usize, usize)> = ranges.iter().copied().collect();
        assert_eq!(unique.len(), ranges.len(), "{pattern} duplicated a byte range");
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

    let ts_three = match_pattern(Language::TypeScript, F66A_TS_CORPUS, "$O.$M1($$$A).$M2($$$B)").unwrap();
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
    let lit = match_pattern(Language::TypeScript, F66A_TS_CORPUS, "alpha.$M1($$$A).$M2($$$B)").unwrap();
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
    let dup = "fn main() {\n    let y = Y;\n    let a = y.y().y();\n    let b = y.f().y().y();\n}\n";
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
    assert!(native_pattern_answerable(Language::TypeScript, "$O?.$M1($$$A).$M2($$$B)"));
    assert!(native_pattern_answerable(Language::JavaScript, "$O?.$M1($$$A).$M2($$$B)"));

    let hits = match_pattern(Language::TypeScript, F66A_TS_CORPUS, "$O?.$M1($$$A).$M2($$$B)").unwrap();
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
    let plain = match_pattern(Language::TypeScript, F66A_TS_CORPUS, "$O.$M1($$$A).$M2($$$B)").unwrap();
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
        span_hits.first().unwrap().captures.get("O").map(String::as_str),
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
    let deep_hits = match_pattern(Language::TypeScript, deep, "$O?.$M1($$$A).$M2($$$B).$M3($$$C)")
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
    assert_eq!(lines_of(&ifndef), vec![21u32], "c #ifndef must answer like sg: {ifndef:?}");
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
    assert!(wrong.is_empty(), "#ifdef must not answer the ifndef region: {wrong:?}");
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
    assert_eq!(both[0].captures.get("A").map(String::as_str), Some("FEATURE_A"));
    assert_eq!(both[0].captures.get("B").map(String::as_str), Some("FEATURE_B"));

    // THE structural cell: `$A` = the whole left operand (sg 0.45.2).
    let mixed = match_pattern(Language::C, c, "#if $A && defined($B)").unwrap();
    assert_eq!(lines_of(&mixed), vec![5u32], "{mixed:?}");
    assert_eq!(
        mixed[0].captures.get("A").map(String::as_str),
        Some("defined(FEATURE_A)"),
        "sg binds the leading metavariable to the whole operand"
    );
    assert_eq!(mixed[0].captures.get("B").map(String::as_str), Some("FEATURE_B"));

    // Parseable-but-unmatched compound conditions are answerable-and-empty.
    assert!(
        !needs_ast_grep_fallback("#if defined($A) || defined($B)"),
        "the || compound parses in sg (exit 0, empty) — must be native"
    );
    assert!(native_pattern_answerable(Language::C, "#if defined($A) || defined($B)"));
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
    let src = "#define PLAIN\n#define VAL 1\n#define FUNC(x) ((x)+1)\n#define OTHER 2\nint v = VAL;\n";
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
    assert_eq!(valued[1].captures.get("A").map(String::as_str), Some("OTHER"));
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
    let maxes = match_pattern(Language::C, "#include <a.h>\n#define MAX 3\n", "#define $X $Y").unwrap();
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
    let php = "<?php\n$a1 = a->b();\n$a2 = a?->b();\n$a3 = $cfg?->get(\"k\", 1);\n$a4 = g?->h?->i();\n";
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
    assert_eq!(by_line(4).captures.get("O").map(String::as_str), Some("$cfg"));
    assert_eq!(by_line(4).captures.get("M").map(String::as_str), Some("get"));
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
    let opt_on_plain = match_pattern(Language::Php, "<?php\n$p = a->b();\n", "$O?->$M($$$A)").unwrap();
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
    assert_eq!(klass_lines(&lit), vec![2u32], "literal lane control: {lit:?}");
    // Empty-args `::` face (the idxgate_php line-3 shape, sg {3}): the
    // literal lane keeps answering it after the fix.
    let empty_php = "<?php\nFoo::bar(1);\nFoo::bar();\n";
    let empty = match_pattern(Language::Php, empty_php, "Foo::bar()").unwrap();
    assert_eq!(klass_lines(&empty), vec![3u32], "empty-args control: {empty:?}");
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
    assert_eq!(eleven.captures.get("$$$A").map(String::as_str), Some("a, b"));
    assert_eq!(eleven.captures.get("B").map(String::as_str), Some("send(c)"));
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
    assert_eq!(klass_lines(&twin), vec![2u32, 6, 12], "twin control: {twin:?}");
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
        ("$A::$B($C)", vec![
            2u32, 3, 4, 5, 6, 7, 8, 10, 12, 13, 14, 17,
        ]),
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
    let two = two_dollar.iter().find(|h| h.line_start == 2).expect("line 2");
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
    for (pattern, want) in [
        ("g(1, $$A)", vec![1u32, 3]),
        ("add(1, $$A)", vec![2u32, 4]),
    ] {
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


