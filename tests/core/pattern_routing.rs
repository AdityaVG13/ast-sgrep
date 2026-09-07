//! Pattern routing tests (e9qc) — native union / prefix routing without external ast-grep.
use ast_sgrep_core::{IndexOptions, SearchOptions, Searcher};
use ast_sgrep_testkit::{isolated_index_session, IsolatedIndexSession};

fn indexed_rs(body: &str) -> IsolatedIndexSession {
    let session = isolated_index_session();
    session.write("mod.rs", body);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    session
}

#[test]
fn pattern_prefix_routes_to_native_or_index_hits() {
    let session = indexed_rs("fn greet_user() {}\nfn other() { greet_user(); }\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher.search("pattern: greet_user").unwrap();
    assert!(
        !response.hits.is_empty(),
        "pattern: greet_user should hit via index signatures and/or native matcher"
    );
}

#[test]
fn rust_function_body_template_matches_without_external_ast_grep() {
    let session = indexed_rs("fn alpha() { beta(); }\nfn beta() {}\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:fn $NAME() { $$$BODY }")
        .expect("native pattern search");
    assert!(
        response
            .hits
            .iter()
            .any(|hit| hit.excerpt.contains("fn alpha")),
        "native function template must find alpha: {:?}",
        response.hits
    );
}

#[test]
fn malformed_function_tail_does_not_use_broad_cached_signature() {
    let session = indexed_rs("fn first() {}\nfn second() {}\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let result = searcher.search("pattern:fn $NAME($$$) trailing garbage");
    assert!(
        result.is_err() || result.is_ok_and(|response| response.hits.is_empty()),
        "malformed pattern must not return broad cached matches"
    );
}

#[test]
fn exotic_pattern_fails_closed_loudly_not_panic() {
    let session = indexed_rs("fn alpha() {}\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    // H-CONF-023 (pass 30): a `$`-pattern the native classifier rejects must
    // not panic AND must not answer a silent `ok:true` empty set — the same
    // loud fail-closed contract codemod has always had (codemod.rs:82). This
    // supersedes the pass-19 "structured empty" contract for the
    // no-structure-char face (`$$$UNLIKELY_EXOTIC_RULE<<<`).
    let err = searcher
        .search("pattern: $$$UNLIKELY_EXOTIC_RULE<<<")
        .expect_err("classification-rejected pattern must fail closed");
    assert!(
        format!("{err:#}").contains("fail-closed"),
        "rejection message must state fail-closed: {err:#}"
    );
}

#[test]
fn pattern_ingress_rejection_is_loud_not_silent_empty() {
    // H-CONF-023 minimal live repro (pass-26 fuzz face): `RETURN $A` carries a
    // metavariable but no structural syntax; sg fails pattern parse (exit 8)
    // while the pre-fix subject answered `ok:true` with zero hits.
    let session = indexed_rs("fn alpha() {}\nfn beta() {}\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    let err = searcher
        .search("pattern:RETURN $A")
        .expect_err("classification-rejected pattern must fail closed, not answer empty");
    assert!(
        format!("{err:#}").contains("RETURN $A"),
        "rejection message must name the pattern: {err:#}"
    );
}

#[test]
fn valid_native_pattern_with_zero_hits_stays_ok_true_empty() {
    // The other side of the H-CONF-023 line: a pattern the native classifier
    // ACCEPTS (the `if` template) that genuinely matches nothing must stay a
    // clean success-with-zero-hits — never a loud rejection.
    let session = indexed_rs("fn alpha() {}\nfn beta() { alpha(); }\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:if $COND { $BODY }")
        .expect("a classifier-accepted pattern must never reject");
    assert!(
        response.hits.is_empty(),
        "corpus has no if statements: {:?}",
        response.hits
    );
}

#[test]
fn hybrid_quoted_literal_intent_hits_phrase_line() {
    let session = indexed_rs("fn main() {\n    let msg = \"foo bar unique_phrase\";\n}\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 16,
        ..session.search_options()
    });
    let hybrid = searcher.search("\"foo bar unique_phrase\"").unwrap();
    let literal = searcher.search("literal:foo bar unique_phrase").unwrap();
    assert!(
        !literal.hits.is_empty(),
        "literal phrase must hit: {:?}",
        literal.hits
    );
    let lit_line = literal.hits[0].line_start;
    assert!(
        hybrid.hits.iter().any(|h| h.line_start == lit_line),
        "quoted hybrid Literal intent must hit same line as literal: (50hx); hybrid={:?} literal={:?}",
        hybrid.hits,
        literal.hits
    );
}

#[test]
fn ident_pattern_is_served_from_index() {
    let session = indexed_rs(
        "pub struct SearchHit { pub file: String }\nfn other() { let _ = SearchHit { file: String::new() }; }\n",
    );
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher.search("pattern:SearchHit").unwrap();
    assert!(
        response
            .hits
            .iter()
            .any(|hit| hit.file.ends_with("mod.rs") && hit.line_start >= 1),
        "ident pattern must hit indexed identifier nodes: {:?}",
        response.hits
    );
}

// EXP-005 (H-CONF-010, pass 14): kind-signature index rows are inexact (every
// `function_definition`), so merging them into search results let a zero-param
// `def main():` match a one-param template. The native matcher decides every
// hit; inexact rows must not be unioned into the result set.
#[test]
fn kind_signature_index_rows_do_not_bypass_native_arity() {
    let session = isolated_index_session();
    session.write(
        "mod.py",
        "def greet(name):\n    msg = name\n    return msg\n\n\ndef shout(text):\n    return text.upper()\n\n\ndef main():\n    print(\"x\")\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:def $A($B): $$$C")
        .expect("indexed pattern search");
    let mut lines: Vec<u32> = response.hits.iter().map(|hit| hit.line_start).collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        vec![1, 6],
        "zero-param def must not match a one-param template: {:?}",
        response.hits
    );
}

// EXP-010 (H-CONF-012 / H-CONF-002, pass 14): the decl-keyword candidate
// table must not mask kinds a language actually emits. Ruby `def` produces
// `method` nodes and php `function` produces `function_definition`; a pattern
// whose classify claim is native must reach the matcher for those kinds and
// return the pure matcher's hits instead of a silent empty set.
#[test]
fn ruby_and_php_decl_templates_are_not_masked_by_kind_prefilter() {
    let session = isolated_index_session();
    session.write(
        "helper.rb",
        "def greet(name)\n  \"hello \" + name\nend\n\ndef shout(text)\n  text.upcase\nend\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:def $A($B)")
        .expect("native ruby def search");
    let mut lines: Vec<u32> = response.hits.iter().map(|hit| hit.line_start).collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        vec![1, 5],
        "ruby def template must match both methods: {:?}",
        response.hits
    );

    let session = isolated_index_session();
    session.write(
        "helper.php",
        "<?php\nfunction greet($name) {\n  return \"hi \" . $name;\n}\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:function $A($B) { $$$C }")
        .expect("native php function search");
    assert_eq!(
        response.hits.len(),
        1,
        "php function template must match greet exactly once: {:?}",
        response.hits
    );
    assert!(
        response.hits[0].excerpt.contains("function greet"),
        "{:?}",
        response.hits[0].excerpt
    );
}

/// H-CONF-025 (pass 43) end-to-end: the duplicate-metavar unification must
/// hold through the full search pipeline (the cached kind lane cannot serve
/// this shape, so the native walk decides — and must reject independent
/// bindings). Registered face FUZZ38-R1-DUPMETA-PY.
#[test]
fn dupmeta_pattern_unifies_through_search_pipeline() {
    let session = isolated_index_session();
    session.write("greet.py", "def greet(name):\n    msg = \"hello \" + name\n    return msg\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:def $A($A): $$$C")
        .expect("native dupmeta search");
    assert!(
        response.hits.is_empty(),
        "duplicate $A must unify through the pipeline: {:?}",
        response.hits
    );
}

/// H-CONF-021 (pass 43) end-to-end: a `$`-less literal-content pattern must
/// return its matches through the search pipeline instead of a silent
/// ok:true empty envelope. Registered face `pattern:1` rust.
#[test]
fn literal_content_pattern_returns_hits_through_pipeline() {
    let session = indexed_rs("fn calc() -> i32 {\n    1\n}\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher.search("pattern:1").expect("literal search");
    assert!(
        !response.hits.is_empty(),
        "pattern:1 must match the number literal, not silently answer empty: {:?}",
        response.hits
    );
}

// ---------------------------------------------------------------------------
// PASS 51 (r9-remediation): the dup-meta family routes through the SEARCH
// PIPELINE natively (pass 48 found these failing closed at the core ingress),
// while the registered fail-closed spellings keep their loud rejection.
// Expected sets are the pinned sg 0.45.2 oracle's (probes in
// gauntlet workspace artifacts/conformance/pass48 + pass-51 re-probes).
// ---------------------------------------------------------------------------

fn indexed_py(body: &str) -> IsolatedIndexSession {
    let session = isolated_index_session();
    session.write("probe.py", body);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    session
}

/// P48-V1/V2/V3 faces through the pipeline: operator chains, nested-call
/// arguments, and `$O.$O($$$A)` receiver==method equality (exactly one hit).
#[test]
fn dupmeta_general_family_answers_through_search_pipeline() {
    let session = indexed_py(
        "x = 1\nif x == x == x:\n    chained = True\n\ndef f(arg):\n    return foo(arg, bar(arg))\n\nobj = Obj()\nobj.method()\nbuilder = Wrap()\nbuilder.builder()\nresult = a.b.c(1)\nsame = Trip()\nsame.same.same(1)\n",
    );
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let chain = searcher
        .search("pattern:$A == $A == $A")
        .expect("operator-chain dupmeta must search natively");
    let chain_lines: Vec<u32> = chain.hits.iter().map(|h| h.line_start).collect();
    assert_eq!(chain_lines, vec![2u32], "{:?}", chain.hits);

    let nested = searcher
        .search("pattern:foo($A, bar($A))")
        .expect("nested-call dupmeta must search natively");
    let nested_lines: Vec<u32> = nested.hits.iter().map(|h| h.line_start).collect();
    assert_eq!(nested_lines, vec![6u32], "{:?}", nested.hits);

    let member = searcher
        .search("pattern:$O.$O($$$A)")
        .expect("member-call unification must search natively");
    let member_lines: Vec<u32> = member.hits.iter().map(|h| h.line_start).collect();
    assert_eq!(
        member_lines,
        vec![11u32],
        "receiver==method equality must leave exactly builder.builder(): {:?}",
        member.hits
    );
}

/// The registered `let $A = $B` fail-closed contract (cases.jsonl
/// P4-LET-GATEOFF-FAILCLOSED) must survive the general expression lane.
#[test]
fn registered_let_pattern_still_fails_closed() {
    let session = indexed_rs("fn alpha() {\n    let x = y;\n}\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    let err = searcher
        .search("pattern:let $A = $B")
        .expect_err("let-binding shape must keep the loud fail-closed contract");
    assert!(
        format!("{err:#}").contains("fail-closed"),
        "rejection must state fail-closed: {err:#}"
    );
}

// ---------------------------------------------------------------------------
// PASS 54 (r9-design-implementation): H-CONF-021 rule R3 full-face pipeline
// parity + H-CONF-022 v2 ingress classes. The corpus below is the VERBATIM
// fixtures/pass53_probes/corpus/pass53_ws.py probe file; the expected hit set
// is the live ast-grep 0.45.2 probe (gauntlet workspace
// artifacts/conformance/pass53/whitespace_literal_matrix_round2.json, cell
// B2-P4, re-probed 2026-09-04).
// ---------------------------------------------------------------------------

const PASS53_PY_CORPUS: &str = r#""""pass-53 probe corpus (python). NEW this pass; never used by prior lanes.

Layout notes for the two probe families:
- whitespace/literal family: greet(...) call written 5 ways (exact, spaced,
  multiline, trailing comma, inline comment) plus negative and AST-variant
  controls;
- metavar-grammar family: lowercase `a` in def-name / assignment / call
  positions so the literal-ident reading can be discriminated from the
  error-node reading, plus float/negative/unicode spellings in free positions.
"""


def greet(name):
    return name


def greet_callers():
    r1 = greet("world")
    r2 = greet(  "world"  )
    r3 = greet(
        "world",
    )
    r4 = greet(  # inline comment
        "world",
    )
    r5 = greet("world", "extra")
    r6 = greet(
        "wor" "ld",
    )
    r7 = greet(

        "world"
    )
    return r1 + r2 + r3 + r4 + r5 + r6 + r7


def a(x):
    a = 1
    return a


def call_sites():
    t = (1, 2)
    t.t(1)
    q = a(1)
    return t


def spaced_kw(name = "world"):
    return name


class alpha:
    def helper(self):
        return alpha.helper(self)


def greet_trailing_comment():
    r8 = greet(
        "world",
        # trailing comment after arg
    )
    return r8


def greet_space_before_paren():
    r9 = greet ("world")
    return r9


def d_probe_py():
    r10 = greet ("world" )
    r11 = greet("world" ,)
    return r10 + r11
"#;

/// Registered H-CONF-021 face through the full search pipeline: the literal
/// pattern `greet("world")` must hit the sg set {18,19,20,30,59,67,72,73}
/// PLUS line 23 — the one documented grammar-attachment variance (sg's
/// python fork attaches the pre-first-arg inline comment at the call node;
/// our vendored tree-sitter-python attaches it inside the arguments
/// container, where the R3 guard skips it). Spaced, multiline, blank-line,
/// trailing-comment-inside-args, and paren-gap variants included;
/// AST-delta sites (extra arg 26, concat 27) excluded.
#[test]
fn literal_r3_full_oracle_set_through_pipeline() {
    let session = isolated_index_session();
    session.write("pass53_ws.py", PASS53_PY_CORPUS);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 64,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:greet(\"world\")")
        .expect("literal R3 search");
    let mut lines: Vec<u32> = response.hits.iter().map(|h| h.line_start).collect();
    lines.sort_unstable();
    lines.dedup();
    assert_eq!(
        lines,
        vec![18u32, 19, 20, 23, 30, 59, 67, 72, 73],
        "literal pattern must match the sg R3 set (+ the documented line-23 attachment variance): {:?}",
        response.hits.iter().map(|h| (h.line_start, &h.excerpt)).collect::<Vec<_>>()
    );
}

/// H-CONF-022 v2 ingress classes through the pipeline: lowercase metavars are
/// valid-native-but-empty (sg accepted-empty), the `fn $3.14()` hole fails
/// closed loudly instead of wildcard-matching, and `$$A` is a universal match
/// (hits exist, including the file's line 1).
#[test]
fn metavar_v2_ingress_classes_through_pipeline() {
    let session = isolated_index_session();
    session.write("pass53_ws.py", PASS53_PY_CORPUS);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 64,
        ..session.search_options()
    });
    let lowercase = searcher
        .search("pattern:greet($a)")
        .expect("lowercase metavar is valid ingress, never fail-closed");
    assert!(
        lowercase.hits.is_empty(),
        "lowercase-led $a must match nothing: {:?}",
        lowercase.hits
    );

    let universal = searcher
        .search("pattern:$$A")
        .expect("$$A must be a native universal match, never fail-closed");
    assert!(
        !universal.hits.is_empty(),
        "$$A must match nodes: {:?}",
        universal.hits
    );
    assert!(
        universal.hits.iter().any(|h| h.line_start == 1),
        "universal must reach the module docstring on line 1: {:?}",
        universal.hits.first()
    );

    let session = isolated_index_session();
    session.write("hole.rs", "fn x() {}\nfn y() { 1 }\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    let err = searcher
        .search("pattern:fn $3.14() { }")
        .expect_err("the wildcard-empty-body hole must fail closed loudly");
    assert!(
        format!("{err:#}").contains("fail-closed"),
        "garbage fn head rejection must state fail-closed: {err:#}"
    );
}

// ---------------------------------------------------------------------------
// PASS 60 (r11-remediation): H-CONF-029 search-level loudness. A `$`-pattern
// the classifier rejects and the general lane cannot template for a corpus
// language must fail CLOSED at search when the query answers empty — never
// the silent ok:true-0 the pass-59 fuzz campaign recorded (23 cases, REGRESSION
// of the pass-30 loud guard). RED against the pass-59 tree: the searcher
// answered Ok with zero hits.
// ---------------------------------------------------------------------------

#[test]
fn hconf029_unanswerable_general_lane_pattern_fails_closed_not_silent_empty() {
    let session = isolated_index_session();
    session.write(
        "mod.ts",
        "function alpha() {\n  beta();\n}\nfunction beta() {}\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let result = searcher.search("pattern:$O.if ($B) { $BODY }");
    assert!(
        result.is_err(),
        "member-prefixed if-template must fail closed loudly, not answer silent empty: {:?}",
        result.map(|r| r.hits.len())
    );
}

#[test]
fn hconf029_answerable_dupmeta_family_still_serves_hits() {
    // The pass-51 faces keep serving through the same ingress (the loud
    // backstop must not over-fire on general-lane patterns the corpus
    // language answers).
    let session = isolated_index_session();
    session.write("mod.py", "x = 1\nif x == x == x:\n    chained = True\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher.search("pattern:$A == $A == $A").expect("dup-meta serves");
    assert!(
        !response.hits.is_empty(),
        "the comparison-chain face must keep its hit: {:?}",
        response.hits
    );
}

// ---------------------------------------------------------------------------
// PASS 63 (r13-remediation): H-CONF-031 search-level loudness, H-CONF-032
// BOM lane, H-CONF-033 read-side root binding. Contracts are sg 0.45.2-probed
// (fixtures/pass63_r13_remediation probe matrix).
// ---------------------------------------------------------------------------

/// H-CONF-031: a classifier-accepted decl template whose grammar cannot
/// parse it under the queried language must fail CLOSED at search (sg exit
/// 8 on the same inputs) — never the silent ok:true-0 the pass-62b fuzz
/// faces recorded, and never the phantom hits the walk over-matched on
/// `function $A($B) { $$$C }` (subject {2} vs sg exit 8).
#[test]
fn hconf031_unanswerable_decl_templates_fail_closed_at_search() {
    let session = isolated_index_session();
    session.write(
        "mod.py",
        "def alpha(name):\n    return name\n\ndef beta():\n    pass\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    for pattern in [
        "pattern:fn $A($B) { $$C }",
        "pattern:function $A($B) { $$$C }",
        "pattern:struct $A { $$$B }",
    ] {
        let result = searcher.search(pattern);
        assert!(
            result.is_err(),
            "{pattern} must fail closed loudly on python, not answer {:?}",
            result.map(|r| r.hits.len())
        );
    }
    // Mirror control: the python-spelled family keeps serving hits.
    let mirror = searcher
        .search("pattern:def $A($B): $$$C")
        .expect("python decl mirror stays answerable");
    assert!(!mirror.hits.is_empty(), "{:?}", mirror.hits);
}

/// H-CONF-032: a BOM-led pattern is stripped at lane entry, so the byte
/// prefilter cannot require the invisible U+FEFF bytes and silently drop
/// every file (sg answers through the BOM).
#[test]
fn hconf032_bom_led_pattern_answers_on_the_index_lane() {
    let session = isolated_index_session();
    session.write(
        "mod.py",
        "def greet(name):\n    return name\n\nprint(greet('world'))\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:\u{feff}greet($A)")
        .expect("BOM-led pattern must answer");
    assert_eq!(response.hits.len(), 1, "{:?}", response.hits);
    assert_eq!(response.hits[0].line_start, 4, "{:?}", response.hits);
}

/// H-CONF-033: an index db is bound to the root that built it
/// (`meta.root`). Answering a query rooted elsewhere must never mix
/// wrong-tree rows into results. The searcher treats a foreign-root db as
/// INERT — the native walk alone answers from the query root — so a
/// same-named file with different content in the two trees yields exactly
/// the query tree's hits, and the foreign tree's symbol never appears. A
/// same-root search keeps full index serving.
#[test]
fn hconf033_searcher_treats_foreign_root_index_as_inert() {
    let session_a = isolated_index_session();
    // Tree A: `shared.py` collides by rel path with tree B but holds
    // DIFFERENT decls (alpha vs gamma); `a_only.py` exists only in A.
    session_a.write(
        "shared.py",
        "def alpha():\n    pass\n\ndef shared_extra():\n    pass\n",
    );
    session_a.write("a_only.py", "def beta():\n    pass\n");
    session_a.index_all(IndexOptions {
        embed_semantic: false,
        ..session_a.index_options()
    });
    // Tree B: same rel path `shared.py`, different decls; plus its own file.
    let session_b = isolated_index_session();
    session_b.write("shared.py", "def gamma():\n    pass\n");
    session_b.write("b_only.py", "def delta():\n    pass\n");

    let options = SearchOptions {
        root: session_b.corpus_root.clone(),
        index_path: Some(session_a.index_path.clone()),
        use_embed: false,
        limit: 32,
        ..session_b.search_options()
    };
    let searcher = Searcher::new(options).expect("foreign-root db is inert, not fatal");
    let response = searcher
        .search("pattern:def $A(): $$$C")
        .expect("foreign-root db must degrade to the walk lane, not error");
    let mut answered: Vec<(String, u32)> = response
        .hits
        .iter()
        .map(|hit| (hit.file.clone(), hit.line_start))
        .collect();
    answered.sort();
    // Exactly the query tree's own zero-arg decls: `gamma` (shared.py:1) and
    // `delta` (b_only.py:1). An unguarded db would leak tree A's rows —
    // `a_only.py:1` (exists only in A) and `shared.py:4` (A's line corpus) —
    // and would MISS `b_only.py` entirely (absent from A's index).
    assert_eq!(
        answered,
        vec![
            ("b_only.py".to_string(), 1),
            ("shared.py".to_string(), 1)
        ],
        "foreign rows leaked or walk lane broken: {:?}",
        response.hits
    );

    // Mirror control: the same db still answers its OWN root at full
    // strength (all three tree-A decls).
    let own = session_a.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session_a.search_options()
    });
    let own_hits = own
        .search("pattern:def $A(): $$$C")
        .expect("same-root serving stays green");
    let mut own_answered: Vec<(String, u32)> = own_hits
        .hits
        .iter()
        .map(|hit| (hit.file.clone(), hit.line_start))
        .collect();
    own_answered.sort();
    assert_eq!(
        own_answered,
        vec![
            ("a_only.py".to_string(), 1),
            ("shared.py".to_string(), 1),
            ("shared.py".to_string(), 4),
        ],
        "same-root answering must keep working: {:?}",
        own_hits.hits
    );
}

/// H-CONF-033 codemod arm: planning rewrites against a foreign-root db is
/// refused by the same binding check.
#[test]
fn hconf033_codemod_refuses_index_bound_to_a_different_root() {
    use ast_sgrep_core::codemod::plan_codemod;

    let session_a = isolated_index_session();
    session_a.write("mod.rs", "fn old_name() {}\n");
    session_a.index_all(IndexOptions {
        embed_semantic: false,
        ..session_a.index_options()
    });
    let session_b = isolated_index_session();

    let result = plan_codemod(
        &session_b.corpus_root,
        Some(&session_a.index_path),
        None,
        "old_name",
        "new_name",
    );
    let err = result.expect_err("cross-root codemod must be refused");
    assert!(
        format!("{err:#}").contains("different project root"),
        "refusal must name the root binding: {err:#}"
    );
}

/// F62-2: a bare statement keyword is not a `pattern_nodes` row — the index
/// early-serve can only answer a silent empty for it; the native
/// statement-template lane must serve sg's hit instead.
#[test]
fn f62_2_bare_break_and_continue_answer_at_search() {
    let session = isolated_index_session();
    session.write(
        "mod.js",
        "async function f(p) {\n    if (p) { break; }\n    continue;\n}\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let brk = searcher.search("pattern:break").expect("break answers");
    assert_eq!(brk.hits.len(), 1, "{:?}", brk.hits);
    assert_eq!(brk.hits[0].line_start, 2, "{:?}", brk.hits);
    let cont = searcher.search("pattern:continue").expect("continue answers");
    assert_eq!(cont.hits.len(), 1, "{:?}", cont.hits);
    assert_eq!(cont.hits[0].line_start, 3, "{:?}", cont.hits);
}

// ---------------------------------------------------------------------------
// PASS 65 (r15-remediation) — failure-first RED tests. Contracts are pinned
// against sg 0.45.2 probes (fixtures/pass65_r15 corpora).
// ---------------------------------------------------------------------------

/// P1 (pass 65, r15 finding 1): an inert `:memory:` store's user_version
/// migration must never touch filesystem paths. Pre-fix,
/// `invalidate_semantic_ivf(":memory:")` resolves a CWD-relative
/// `semantic.ivf` and deletes whatever file sits there — recorded data
/// loss. The CWD sentinel must survive a foreign-root search that swaps to
/// the in-memory stand-in (H-CONF-033 path).
#[test]
fn f64_in_memory_store_migration_spares_cwd_semantic_ivf() {
    // CWD is process-global: serialize against any other cwd-switching test.
    static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let session_a = isolated_index_session();
    session_a.write("mod.py", "def alpha():\n    pass\n");
    session_a.index_all(IndexOptions {
        embed_semantic: false,
        ..session_a.index_options()
    });

    let session_b = isolated_index_session();
    session_b.write("mod.rs", "fn beta() {}\n");
    session_b.index_all(IndexOptions {
        embed_semantic: false,
        ..session_b.index_options()
    });

    let scratch = tempfile::tempdir().expect("scratch cwd");
    let sentinel = scratch.path().join("semantic.ivf");
    std::fs::write(&sentinel, b"cwd-sentinel").expect("write sentinel");
    let original = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(scratch.path()).expect("switch cwd");
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // db bound to root A, queried against root B: read-side root
        // binding swaps to the inert in-memory store, whose fresh
        // user_version=0 runs the legacy migration branch.
        Searcher::new(SearchOptions {
            root: session_b.corpus_root.clone(),
            index_path: Some(session_a.index_path.clone()),
            use_embed: false,
            limit: 8,
            ..session_b.search_options()
        })
    }));
    std::env::set_current_dir(original).expect("restore cwd");
    let searcher = outcome
        .expect("foreign-root open must not panic")
        .expect("foreign-root search must open the inert store");
    let response = searcher
        .search("pattern:fn $A() { $$$B }")
        .expect("search against the swapped store must succeed");
    let _ = response;
    assert!(
        sentinel.exists(),
        "in-memory store migration deleted the CWD-relative semantic.ivf (data loss)"
    );
}

/// P9a (pass 65, LOW a): a BOM-only pattern must be refused loudly like the
/// empty pattern — pre-fix the BOM strip runs AFTER the empty check, so
/// `"\u{feff}"` slips the guard and plans a nonsense codemod.
#[test]
fn f64_codemod_bom_only_pattern_is_loud() {
    use ast_sgrep_core::codemod::plan_codemod;

    let session = isolated_index_session();
    session.write("mod.rs", "fn alpha() {}\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });

    for pattern in ["", " ", "\u{feff}", " \u{feff} "] {
        let result = plan_codemod(
            &session.corpus_root,
            Some(&session.index_path),
            None,
            pattern,
            "x",
        );
        let err = result.expect_err(&format!("BOM-only pattern {pattern:?} must refuse loudly"));
        let msg = format!("{err:#}");
        assert!(
            msg.contains("must not be empty"),
            "refusal must name the empty-pattern guard for {pattern:?}: {msg}"
        );
    }
}

/// P9b (pass 65, LOW b): the H-CONF-023 honesty gate must reconcile a stale
/// index against the CURRENT file bytes before refusing — the tree no
/// longer contains the served identifier, so the zero-edit plan is honest
/// (search on the same stale db answers stale hits, but the codemod reads
/// the tree). A fresh index that still contains the identifier keeps the
/// loud refusal.
#[test]
fn f64_codemod_honesty_gate_reconciles_stale_index() {
    use ast_sgrep_core::codemod::plan_codemod;

    let session = isolated_index_session();
    session.write("mod.rs", "fn old_name() {}\nfn keeper() {}\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    // Concurrent edit after indexing: old_name is gone from the tree while
    // the index still serves it.
    session.write("mod.rs", "fn keeper() {}\n");
    let plan = plan_codemod(
        &session.corpus_root,
        Some(&session.index_path),
        None,
        "old_name",
        "new_name",
    )
    .expect("stale index must reconcile against current file bytes");
    assert_eq!(plan.edit_count, 0, "stale hit must not plan an edit");
    assert_eq!(plan.files_changed, 0);

    // Control: a fresh index serving the DECL face (search serves it, the
    // in-process matcher produces no edit span) keeps the H-CONF-023 loud
    // refusal — the registered true-positive contract
    // (`codemod_loudly_refuses_index_served_decl_pattern_that_plans_zero_edits`,
    // tests/cli/codemod_crash_windows.rs). The bare identifier is the wrong
    // control shape: the matcher spans it fine, so a 1-edit plan is the
    // honest answer there and no refusal may fire.
    let fresh = isolated_index_session();
    fresh.write("mod.rs", "fn old_name() {}\n");
    fresh.index_all(IndexOptions {
        embed_semantic: false,
        ..fresh.index_options()
    });
    let err = plan_codemod(
        &fresh.corpus_root,
        Some(&fresh.index_path),
        None,
        "fn old_name",
        "fn new_name",
    )
    .expect_err("fresh index serving the decl face must still refuse loudly");
    assert!(
        format!("{err:#}").contains("search serves it from the index"),
        "{err:#}"
    );
}

/// P9d (pass 65, LOW d): the unanswerable-language census must classify
/// extension-less files by CONTENT (shebang), not by extension alone —
/// pre-fix a `pybox` file is invisible to the census, so a rust-spelled
/// template over a python-only corpus answers a silent ok:true empty
/// instead of failing closed (H-CONF-029/031 class).
#[test]
fn f64_census_counts_content_detected_languages_for_unanswerable_gate() {
    let session = isolated_index_session();
    session.write("pybox", "#!/usr/bin/env python3\ndef alpha():\n    pass\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    let result = searcher.search("pattern:fn $A($B) { $$C }");
    assert!(
        result.is_err(),
        "rust-spelled template over a content-detected python corpus must fail \
         closed loudly, not answer silent empty: {:?}",
        result.map(|r| r.hits.len())
    );
    // Control: an answerable pattern still answers over the same corpus.
    let literal = searcher.search("pattern:alpha").expect("literal answers");
    assert!(!literal.hits.is_empty(), "{:?}", literal.hits);
}

/// P9c (pass 65, LOW c): the capture->commit race refusal must KEEP the
/// pre-race capture sidecar on disk (it holds the only copy of the
/// pre-swap content) and NAME it in the error. RED note: the test was
/// written against the extracted `concurrent_capture_error` helper, which
/// did not exist pre-fix — the compile failure is the recorded RED; the
/// sidecar-existence assert kills the re-added-deletion mutant.
#[test]
fn f64_race_capture_sidecar_is_preserved_and_named() {
    use ast_sgrep_core::codemod::concurrent_capture_error;

    let root = tempfile::tempdir().expect("temp root");
    let sidecar = root.path().join("mod.rs.asgrep-codemod-backup-0");
    std::fs::write(&sidecar, b"pre-race content").expect("write sidecar");

    let err = concurrent_capture_error(root.path(), "mod.rs", &sidecar, None);
    let msg = format!("{err:#}");
    assert!(msg.contains("concurrent write"), "{msg}");
    assert!(
        msg.contains(sidecar.to_str().expect("utf8 sidecar path")),
        "error must name the capture sidecar: {msg}"
    );
    assert!(
        sidecar.exists(),
        "the pre-race capture sidecar must stay on disk (data loss)"
    );

    // The rollback-failed variant keeps the same sidecar naming.
    let err = concurrent_capture_error(
        root.path(),
        "mod.rs",
        &sidecar,
        Some(std::io::Error::other("rollback boom")),
    );
    let msg = format!("{err:#}");
    assert!(msg.contains("rollback also failed"), "{msg}");
    assert!(
        msg.contains(sidecar.to_str().expect("utf8 sidecar path")),
        "{msg}"
    );
    assert!(sidecar.exists(), "{msg}");
}

/// PASS 65 (LOW a, search half): a BOM-only pattern degrades to `""` after
/// the H-CONF-032 strip; pre-fix it slipped the ingress and answered a
/// silent ok:true-empty envelope (release-binary probe, exit 0, no hits).
/// Contract: empty-after-strip refuses loudly in the same operational class
/// as the structural-ingress rejection, while a BOM-LED real pattern keeps
/// answering through the strip (kills an always-err guard mutant).
#[test]
fn bom_only_pattern_refuses_loud_not_silent_empty() {
    let session = indexed_rs("fn greet_user() {}\nfn other() { greet_user(); }\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 16,
        ..session.search_options()
    });
    for query in ["pattern:", "pattern:\u{feff}", "pattern: \u{feff} "] {
        let error = searcher
            .search(query)
            .expect_err("empty/BOM-only pattern must refuse loudly, not answer silent empty");
        let text = format!("{error:#}");
        assert!(
            text.contains("must not be empty"),
            "refusal must name the emptiness, got: {text}"
        );
    }
    let response = searcher
        .search("pattern:\u{feff}greet_user")
        .expect("a BOM-led real pattern still answers through the strip");
    assert!(
        !response.hits.is_empty(),
        "the stripped BOM-led pattern must keep its hits"
    );
}

// ---------------------------------------------------------------------------
// PASS 67c (r17-remediation) — H-CONF-036 / F26-0561. sg 0.45.2 keeps `$$$Rest`
// (multi-capture) and `$Rest` (single capture) in DISTINCT slots even when they
// share a base name; the native chain matcher binds both through one map key,
// so a classifier-accepted pattern like `$O.out.$$$A($A)` cannot bind the
// property-name rest and answers a SILENT ok:true-empty where sg answers hits
// (`System.out.println(total)`). The chain lane otherwise implements sg's
// rest-binds-property semantics exactly — every non-colliding shape below is
// pinned to the live sg probe on this exact corpus (pass-67c probe matrix,
// oracle binary ast-grep 0.45.2, sha'd corpus in fixtures/rest67c_pin notes).
// ---------------------------------------------------------------------------

fn indexed_java(body: &str) -> IsolatedIndexSession {
    let session = isolated_index_session();
    session.write("Main.java", body);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    session
}

const REST67C_JAVA: &str = "public class Main {\n    public static void main(String[] args) {\n        int total = add(1, 2);\n        System.out.println(total);\n    }\n}\n";

/// The F26-0561 face itself: a rest-meta in the property-NAME slot whose base
/// name collides with a single metavariable must answer sg's hit — never the
/// silent ok:true-empty the index-served lane recorded (pass-66b fuzz, native
/// lane loud, controls exact). RED against the pre-67c tree: searcher answered
/// Ok with zero hits.
#[test]
fn hconf036_rest_name_collision_with_single_meta_answers_sg_hits() {
    let session = indexed_java(REST67C_JAVA);
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 16,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:$O.out.$$$A($A)")
        .expect("classifier-accepted collision pattern must answer, not reject");
    let lines: Vec<u32> = response.hits.iter().map(|h| h.line_start).collect();
    assert_eq!(
        lines,
        vec![4u32],
        "colliding rest-name slot must answer sg's System.out.println hit: {:?}",
        response.hits
    );
    assert!(
        response.hits[0].excerpt.contains("System.out.println"),
        "hit must be the member call sg binds: {:?}",
        response.hits[0]
    );
}

/// Controls (mission-pinned): the exact-answer faces are unchanged by the 67c
/// lane, and the non-colliding rest-name shapes keep proving the lane already
/// implements sg's rest-binds-property semantics (each set is the live sg
/// probe on this corpus). `$O.$$$M($A)` must NOT match the bare `add(1, 2)`
/// call — a name-slot rest binds the property node of a member chain only,
/// exactly sg's semantics.
#[test]
fn hconf036_controls_and_non_colliding_rest_name_sets_stay_exact() {
    let session = indexed_java(REST67C_JAVA);
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 16,
        ..session.search_options()
    });
    for pattern in [
        // mission controls (exact answers before 67c)
        "pattern:$O.out.println($A)",
        "pattern:$O.$M($A)",
        // non-colliding rest-name shapes (sg-probed: all hit line 4 only)
        "pattern:$O.$$$M($A)",
        "pattern:$O.out.$$$C($A)",
        "pattern:$O.$$$M.$$$C($A)",
    ] {
        let response = searcher
            .search(pattern)
            .unwrap_or_else(|err| panic!("{pattern} must answer, not reject: {err:#}"));
        let lines: Vec<u32> = response.hits.iter().map(|h| h.line_start).collect();
        assert_eq!(
            lines,
            vec![4u32],
            "{pattern} must keep sg's exact hit set: {:?}",
            response.hits
        );
    }
    // sg agreement on the negative: a whole-callee rest does not match the
    // bare (non-member) call `add(1, 2)`. sg answers accepted-empty there;
    // the subject keeps its REGISTERED fail-closed posture for the
    // classifier-rejected whole-callee rest shape — loud, never a hit on the
    // bare call, never a silent serve of a shape its classifier rejects.
    let bare = searcher.search("pattern:$O.$$$A(1, 2)");
    let err = bare.expect_err("whole-callee rest must keep the registered loud class");
    assert!(
        format!("{err:#}").contains("fail-closed"),
        "rejection must state fail-closed: {err:#}"
    );
}

/// Presentation guard: the classifier-rejected colliding-arg face keeps its
/// loud fail-closed class through the 67c ingress (the rename must run AFTER
/// the H-CONF-023 gate, never before it).
#[test]
fn hconf036_rejected_rest_arg_collision_keeps_loud_class() {
    let session = indexed_java(REST67C_JAVA);
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 16,
        ..session.search_options()
    });
    let err = searcher
        .search("pattern:$O.log($$$A, $A)")
        .expect_err("classifier-rejected rest-arg collision must stay loud");
    assert!(
        format!("{err:#}").contains("fail-closed"),
        "rejection must state fail-closed: {err:#}"
    );
}
