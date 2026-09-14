//! Pattern routing tests (e9qc) — native union / prefix routing without external ast-grep.
use ast_sgrep_core::{IndexOptions, ParsedQuery, QueryMode, SearchOptions, Searcher};
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

// ---------------------------------------------------------------------------
// PASS 87a (r37-remediation) — 86a-H1: the r35 comment-pin faces are answered
// by the native matcher (FB-84a-03 php hook, suite cell
// `f85a_php_comment_between_binary_operands_answered`, 136/136 on binary
// 98e9a0aba2215901) but the SEARCH PIPELINE refused them with the loud
// structural-fallback error. Ingress path map (probed + code-read): the
// H-CONF-023 ingress gate itself ADMITS these faces — every php-hook face
// carries a lowercase-led `$var` token, so classify_native lands
// Some(NeverMatches) and needs_ast_grep_fallback is false. The refusal
// composes later: the per-file H-CONF-031 skip and the H-CONF-029 census are
// keyed on native_pattern_answerable, which cannot see the php hook (for a
// NeverMatches classify its only false arm is lane_comment_refused —
// pattern-side `/* */` / `//` comment syntax), so every file was skipped,
// the census counted every corpus language unanswerable, and the iva9.7
// backstop turned the composed skips into `pattern requires structural
// fallback (...; fail-closed)` (rc2) — exactly the red-team 86a-H1 probe.
// The fix (core/pattern.rs): NeverMatches-classified patterns cannot
// phantom-hit (their structural arm answers empty; only the registered
// sg-exact php-hook / dollar-literal lanes can answer), so the skip and the
// census defer to the matcher's own per-file answer for exactly that class:
// hits = hits, an empty is an honest empty. Every other class keeps its
// registered contract byte-for-byte (classifier-accepted decl/call shapes
// keep the skip + census incl. the deliberate 64c-F6 pin; ingress-rejected
// garbage keeps the loud H-CONF-023 refusal; §27.5 bare connectors have no
// `$` and never enter this path).
// sg 0.45.2 oracle probes, 2026-09-08, ATTACHED --pattern=<value> on these
// exact fixture bytes (= the f85a `com` fixture = pass86a z_com):
//   $a = ($V /* q */ + 1); -> {10}   $a = $V + /* c */ 1; -> {4}
//   $a = $W . /* m */ $X;  -> {8}    $a = $V + 1;         -> {2,3,4,7}
//   $a = ($V + 1);         -> {6,9,10}
// ---------------------------------------------------------------------------

const F87A_PHP_COM: &str = "<?php\n$a = $v + 1;\n$a = $v /* = */ + 1;\n$a = $v + /* c */ 1;\n$a = /* h */ $v + 1;\n$a = ($v /* p */ + 1);\n$a = $v + 1; // tail\n$a = $w . /* m */ $x;\n$a = ($v + 1);\n$a = ($v /* q */ + 1);\n";

/// RED (86a-H1): the pattern-side-comment faces must answer sg's hit sets
/// through the full pipeline. Pre-fix the three comment faces returned the
/// structural-fallback error (rc2 class) while the two comment-FREE controls
/// already answered — the RED failure is exactly the comment family.
#[test]
fn f87a_php_pattern_side_comment_faces_answer_sg_sets_through_pipeline() {
    let session = isolated_index_session();
    session.write("probe.php", F87A_PHP_COM);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    for (pattern, want) in [
        ("pattern:$a = ($V /* q */ + 1);", vec![10u32]),
        ("pattern:$a = $V + /* c */ 1;", vec![4u32]),
        ("pattern:$a = $W . /* m */ $X;", vec![8u32]),
        // Comment-FREE controls (already answered pre-fix): the fix must not
        // move the transparency faces, and line 5 (`/* h */` before the first
        // operand) must stay excluded from `$a = $V + 1;` (the top-node
        // comment guard — sg refuses that candidate too).
        ("pattern:$a = $V + 1;", vec![2u32, 3, 4, 7]),
        ("pattern:$a = ($V + 1);", vec![6u32, 9, 10]),
    ] {
        let response = searcher
            .search(pattern)
            .unwrap_or_else(|err| panic!("{pattern} must answer, not reject: {err:#}"));
        let mut lines: Vec<u32> = response.hits.iter().map(|h| h.line_start).collect();
        lines.sort_unstable();
        lines.dedup();
        assert_eq!(
            lines, want,
            "{pattern}: sg 0.45.2 line set (86a-H1): {:?}",
            response.hits
        );
    }
}

/// RED (86a-H1, empty half): for a NeverMatches-classified comment face with
/// no candidates the walk's empty is an HONEST empty (sg answers [] rc0 on
/// the same input) — the census must not re-classify it unanswerable and the
/// backstop must not compose the skips into the loud fallback error. Pre-fix
/// this returned the structural-fallback error. Mutant tooth: revert the
/// census exemption (keep the skip exemption) and this flips to Err; drop
/// the skip exemption instead and the hit faces above stay rejected.
#[test]
fn f87a_comment_face_without_candidates_answers_honest_empty() {
    let session = isolated_index_session();
    session.write("probe.php", F87A_PHP_COM);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    // sg 0.45.2 (ATTACHED, same fixture): [] rc0 — the pinned comment text
    // `/* nope */` appears on no line.
    let response = searcher
        .search("pattern:$a = ($V /* nope */ + 1);")
        .expect("a NeverMatches-classified comment face with no candidates must answer, not reject");
    assert!(
        response.hits.is_empty(),
        "no line carries /* nope */: {:?}",
        response.hits
    );
}

/// The exactly-consistent side of the 86a-H1/M2 contract: the ingress refuses
/// EXACTLY the faces match_pattern cannot answer. These five faces are
/// sg-ANSWERING (probed 2026-09-08, ATTACHED) — `$` -> every `$` line
/// ({2,3,4,5,6,7,8} on the fixture below; {2..10} on the pass86a z_com copy),
/// `$日本 = $V;` -> {2,4}, `$α = $V + 1;` -> {5}, `$ü = $V . $W;` -> {8},
/// `$α = $V;` -> {5} on this fixture ([] on the §27.2 fixture — RHS meta
/// binds the binary expression) — and `Foo::bar($A);` -> {4} on the
/// pass86a f_fp corpus — but the NATIVE MATCHER answers none of them (the
/// php hook's is_php_literal_target refuses non-ASCII/2+-word targets, the
/// dollar-literal lane has no bare-`$` cell, the Call arm refuses a trailing
/// `;`, and the general substitution refuses non-ASCII and bare-`$` names),
/// so admission would only convert the loud refusal into a silent miss (and
/// for `$α = $V;` flip the §27.2-registered loud class). They stay LOUD rc2
/// fail-closed; the sg-hit divergence is the registered matcher-gap residual
/// (retry predicate: when lang widens the literal-target / literal-lane /
/// call-arm surface, these spellings must be revisited at the ingress gate
/// in the SAME pass — the gate's substitution and Call-arm refusals key on
/// the same spellings the matcher would answer). Mutant tooth: a fix that
/// opens the ingress gate (or drops the empty-backstop) for these faces
/// turns at least one cell here into Ok — killed.
#[test]
fn f87a_ingress_keeps_matcher_unanswerable_faces_loud() {
    const F87A_PHP_NON_ASCII: &str = "<?php\n$日本 = $v;\n$alpha = $v;\n$日本 = $b = 1;\n$α = $v + 1;\n$e = $v /* 🎉 */ + 1;\n$p = ($v /* 🎉 */ + 1);\n$ü = $v . $w;\n";
    let session = isolated_index_session();
    session.write("probe.php", F87A_PHP_NON_ASCII);
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
        "pattern:$",
        "pattern:$日本 = $V;",
        "pattern:$α = $V + 1;",
        "pattern:$α = $V;",
        "pattern:$ü = $V . $W;",
    ] {
        match searcher.search(pattern) {
            Ok(response) => panic!(
                "{pattern} must stay loud (the matcher cannot answer it), answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{pattern} must keep the fail-closed class: {err:#}"
            ),
        }
    }
    // Positive control: the hook-answerable face keeps answering through the
    // same walk (sg {3}) — loudness is per-face, not per-corpus.
    let alpha = searcher
        .search("pattern:$alpha = $V;")
        .expect("hook-answerable face must keep answering");
    let mut lines: Vec<u32> = alpha.hits.iter().map(|h| h.line_start).collect();
    lines.sort_unstable();
    lines.dedup();
    assert_eq!(lines, vec![3u32], "{:?}", alpha.hits);

    // `Foo::bar($A);` (sg {4} on pass86a f_fp): the trailing `;` defeats the
    // Call arm and every general template, so the face stays loud.
    let session = isolated_index_session();
    session.write("probe.php", "<?php\n$o->c1(2);\nFoo::bar(3);\n");
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
        .search("pattern:Foo::bar($A);")
        .expect_err("Foo::bar($A); must keep the loud class (matcher cannot answer it)");
    assert!(
        format!("{err:#}").contains("fail-closed"),
        "rejection must state fail-closed: {err:#}"
    );
}

// ---------------------------------------------------------------------------
// PASS 89a (r39-remediation) — 88c-H1: the r37 matcher_decides exemption
// (classify NeverMatches ⇒ exempt from the H-CONF-031 per-file skip AND the
// H-CONF-029 census) is sg-exact only for the comment placements sg accepts.
// sg 0.45.2 matrix, 2026-09-08, ATTACHED `--pattern=<p> -l <lang>` on the
// /tmp/sgmatrix_r39 fixtures (99 cells: {block /* */, line //, hash #} ×
// {php, py, rb, rust, js, ts, go, c, cpp} × {none, paren, mid, lead, trail}):
//   - inline `/* */` parses in php (rc1/rc0 — the r37 hook family, f87a-pinned)
//     and in rb/rust/js/ts/c/cpp, but rc8s in py/go;
//   - a TRAILING `/* */` after a `;`-statement is rc8 in EVERY probed language
//     (php/rust/js/ts/c/cpp; py lenient rc0+ERROR-node);
//   - `$x # note` is rc8 in py/rb/php AND js/ts (hash = real syntax there, the
//     pass-54 exit-8-vs-empty genus); `$x // note` is rc8 in php/rust/c but
//     rc1 in rb/js/ts;
//   - `$a = $V + 1; // note` / `# note` (statement-terminated) are rc8 in
//     php/rust/c/go.
// Pre-r37 every rc8 spelling above was sg-agreed LOUD through the census
// (lane_comment_refused ⇒ native_pattern_answerable=false ⇒
// unanswerable_corpus_languages>0 ⇒ iva9.7 backstop). Post-r37 the blanket
// exemption silenced ALL of them into ok:true [] — probed on release
// 01d104a035ea0758: rc0 hits=0 on every loud cell below (the RED baseline).
// The fix gates matcher_decides per language: a NeverMatches pattern is
// exempt only when it is comment-free or its only comments are INLINE block
// comments in php (code follows the last `*/`); every other comment face
// falls back to the pre-r37 census path byte-for-byte.
// ---------------------------------------------------------------------------

/// RED (88c-H1, loud half): the glued line-comment NeverMatches faces sg rc8s
/// must return to the census-loud fail-closed class. Pre-fix each answered
/// silent ok:true [] (r37 exemption); pre-r37 each was sg-agreed loud.
#[test]
fn f89a_glued_line_comment_faces_stay_loud_where_sg_rejects() {
    for (file, body, lang, pattern) in [
        (
            "probe.py",
            "alpha = 1 + 1\nbeta = 2 + 1\n",
            "py",
            "$x # note",
        ),
        (
            "probe.rb",
            "alpha = 1 + 1\nbeta = 2 + 1\n",
            "rb",
            "$x # note",
        ),
        (
            "probe.php",
            "<?php\n$alpha = 1 + 1;\n$beta = 2 + 1;\n",
            "php",
            "$x # note",
        ),
        (
            "probe.php",
            "<?php\n$alpha = 1 + 1;\n$beta = 2 + 1;\n",
            "php",
            "$x // note",
        ),
        (
            "probe.rs",
            "fn main() {\n    let alpha = 1 + 1;\n}\n",
            "rust",
            "$x // note",
        ),
        (
            "probe.rs",
            "fn main() {\n    let alpha = 1 + 1;\n}\n",
            "rust",
            "$a = $V + 1; // note",
        ),
        (
            "probe.c",
            "int main(void) {\n    int alpha = 1 + 1;\n    return 0;\n}\n",
            "c",
            "$x // note",
        ),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "{lang} {pattern:?}: sg 0.45.2 rc8 — must stay census-loud, answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{lang} {pattern:?} must keep the fail-closed class: {err:#}"
            ),
        }
    }
}

/// RED (88c-H1, trailing-block half): a trailing `/* */` after the statement
/// is sg rc8 in every probed language (88a fu04 rider included), so the block
/// exemption must cover only the INLINE placement. Pre-fix these answered
/// silent ok:true [] through the walk's structural empty.
#[test]
fn f89a_trailing_block_comment_faces_stay_loud_where_sg_rejects() {
    for (file, body, lang, pattern) in [
        (
            "probe.php",
            "<?php\n$alpha = 1 + 1;\n$beta = 2 + 1;\n",
            "php",
            "$a = $V + 1; /* c */",
        ),
        (
            "probe.php",
            "<?php\n$alpha = 1 + 1;\n$beta = 2 + 1;\n",
            "php",
            "$a = $V + 1; /*++*/",
        ),
        (
            "probe.js",
            "const alpha = 1 + 1;\nconst beta = 2 + 1;\n",
            "js",
            "$a = $V + 1; /* c */",
        ),
        (
            "probe.rs",
            "fn main() {\n    let alpha = 1 + 1;\n}\n",
            "rust",
            "$a = $V + 1; /* c */",
        ),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "{lang} {pattern:?}: sg 0.45.2 rc8 — trailing block comment must stay \
                 census-loud, answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{lang} {pattern:?} must keep the fail-closed class: {err:#}"
            ),
        }
    }
}

/// Class preservation (88c-H1 blast-radius guard): the fix must move ONLY the
/// sg-rc8 comment placements. Inline php block faces keep answering sg sets
/// (the f87a contract), the census fallback keeps the registered pre-r37
/// postures on the sg-accepted line-comment faces (loud genus), and the
/// walk-decided silent cells (comment-free NeverMatches, `#`-as-syntax in
/// rust/js) keep their registered silence byte-for-byte.
#[test]
fn f89a_exemption_gate_preserves_registered_classes() {
    // Positive control: the r37 php inline-block hook family keeps answering
    // sg's set through the gated exemption (sg 0.45.2 {10} on this fixture).
    let session = isolated_index_session();
    session.write("probe.php", F87A_PHP_COM);
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
        .search("pattern:$a = ($V /* q */ + 1);")
        .expect("inline php block face must keep answering through the gated exemption");
    let mut lines: Vec<u32> = response.hits.iter().map(|h| h.line_start).collect();
    lines.sort_unstable();
    lines.dedup();
    assert_eq!(lines, vec![10u32], "{:?}", response.hits);
    // Census fallback keeps the registered pre-r37 loud postures where sg
    // accepts the spelling (php `#`/`//` after `;` — sg rc8 WITH the `;` on
    // the 0.45.2 matrix, rc1 without; js `//` — sg rc1 accepted-empty): the
    // loud genus is the pre-r37 contract, never a silent answer.
    for pattern in ["$a = $V + 1; # note", "$a = $V + 1; // note"] {
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "{pattern} must stay census-loud (pre-r37 registered posture), answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{pattern} must keep the fail-closed class: {err:#}"
            ),
        }
    }
    // Non-php inline block faces keep the census path (sg rc8 in py/rb —
    // agreed loud; pre-r37 contract for the rc1 languages).
    for (file, body, lang, pattern) in [
        (
            "probe.py",
            "alpha = 1 + 1\nbeta = 2 + 1\n",
            "py",
            "$a = ($V /* q */ + 1)",
        ),
        (
            "probe.rb",
            "alpha = 1 + 1\nbeta = 2 + 1\n",
            "rb",
            "$a = $V + /* c */ 1",
        ),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "{lang} {pattern:?}: non-php inline block must keep the census path \
                 (sg rc8), answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{lang} {pattern:?} must keep the fail-closed class: {err:#}"
            ),
        }
    }
    // Registered silent cells must NOT move: the walk decides them pre-r37
    // and post-fix alike.
    //   py `$a = 1` — comment-free NeverMatches, pass-54 sg-exit-8-vs-empty
    //   residual; guards the comment-free exemption arm.
    //   rust `$x # note` — `#` is rust attribute syntax; census fallback finds
    //   the pattern answerable and the walk answers; sg 0.45.2 is
    //   lenient-empty (rc0 + ERROR node) — agreed silent.
    //   js `$x # note` — `#` is js private-field syntax; sg 0.45.2 rc8s the
    //   spelling (pass-54 exit-8-vs-empty genus) and the subject keeps its
    //   registered walk-decided silence (pre-r37 byte-equal).
    for (file, body, lang, pattern) in [
        ("probe.py", "alpha = 1 + 1\n", "py", "$a = 1"),
        (
            "probe.rs",
            "fn main() {\n    let alpha = 1 + 1;\n}\n",
            "rust",
            "$x # note",
        ),
        ("probe.js", "const alpha = 1 + 1;\n", "js", "$x # note"),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        let response = searcher
            .search(&format!("pattern:{pattern}"))
            .unwrap_or_else(|err| panic!("{lang} {pattern:?} must stay silent: {err:#}"));
        assert!(
            response.hits.is_empty(),
            "{lang} {pattern:?}: registered silent class must keep its honest empty: {:?}",
            response.hits
        );
    }
}

// ---------------------------------------------------------------------------
// PASS 91a (F-r40-1 / 90A-F1 + 90B-89CONF-2): the 89a php block-comment
// exemption was placement-blind — a LEADING `/* */` (only whitespace before
// it) spawns a second top-level node, so sg 0.45.2 rc8s the pattern
// ("Cannot parse query as a valid pattern"), yet the exemption let the walk
// answer silent ok:true []. Probed matrix (gauntlet workspace
// artifacts/conformance/pass91a/probes_run1.jsonl, matrix A): the leading
// block rc8s the NeverMatches assignment face in ALL eleven languages and
// the bare-meta face everywhere but python (rc1 lenient-empty — the
// registered census-loud posture there). The other ten languages already
// answered loud through the lang-side comment refusal; php was the silenced
// cell. Fix: `block_leading` is excluded from the exemption for ALL
// languages.
// ---------------------------------------------------------------------------

/// RED (F-r40-1): php leading-block faces must return to the census-loud
/// fail-closed class. Pre-fix each answered silent ok:true [] through the
/// walk (the 90A-F1 probe face).
#[test]
fn f91a_leading_block_comment_faces_stay_loud_where_sg_rejects() {
    let php = (
        "probe.php",
        "<?php\n$alpha = 1 + 1;\n$beta = 2 + 1;\n",
        "php",
    );
    for (file, body, lang, pattern) in [
        (php.0, php.1, php.2, "/* c */ $a = $V + 1;"),
        (php.0, php.1, php.2, "/* c */ $x"),
        // whitespace-led twin: still the leading placement (probes_run1
        // matrix A: sg rc8, subject silent pre-fix).
        (php.0, php.1, php.2, "  /* c */ $x"),
        // Non-php leading-block controls — sg rc8 on the assignment face in
        // every probed language (matrix A); py bare-meta is sg rc1
        // lenient-empty and keeps the registered census-loud class. These
        // were loud pre-fix via the lang-side comment refusal; the fix must
        // not silence them either.
        (
            "probe.py",
            "alpha = 1 + 1\nbeta = 2 + 1\n",
            "py",
            "/* c */ $a = $V + 1;",
        ),
        (
            "probe.rs",
            "fn main() {\n    let alpha = 1 + 1;\n}\n",
            "rust",
            "/* c */ $a = $V + 1;",
        ),
        (
            "probe.js",
            "const alpha = 1 + 1;\nconst beta = 2 + 1;\n",
            "js",
            "/* c */ $x",
        ),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "{lang} {pattern:?}: sg 0.45.2 rc8 — leading block comment must stay \
                 census-loud, answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{lang} {pattern:?} must keep the fail-closed class: {err:#}"
            ),
        }
    }
    // Positive control: the php INLINE (mid-pattern) block placement keeps
    // its walk exemption — sg 0.45.2 rc1 [] on this fixture and the subject
    // answers ok:true (probes_run1 matrix A control cell). The exemption
    // narrows; it does not close.
    let session = isolated_index_session();
    session.write(php.0, php.1);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:$a = ($V /* q */ + 1);")
        .expect("php inline block placement must keep the walk exemption");
    assert!(
        response.hits.is_empty(),
        "inline control must keep its (empty) walk answer: {:?}",
        response.hits
    );
}

// ---------------------------------------------------------------------------
// PASS 91a (F-r40-4 / 90A-F3): `$`-less comment faces dodged both comment
// gates. classify_native returns None (no `$`), so the face never takes the
// NeverMatches route, and `native_pattern_answerable` returns true for any
// `$`-less pattern BEFORE its comment refusal is consulted — the
// template-exists path built a doc with the comment trivia, walked, and
// answered a silent ok:true [] where sg 0.45.2 rc8s the placement
// (probes_run1/run2 matrix B: rc8 on every cell below).
// ---------------------------------------------------------------------------

/// RED (F-r40-4): `$`-less comment faces must be loud, never silent.
#[test]
fn f91a_dollar_less_comment_faces_route_through_comment_gate() {
    for (file, body, lang, pattern) in [
        (
            "probe.py",
            "alpha = 1 + 1\nbeta = 2 + 1\nx = 1\ny = a + b\n",
            "py",
            "(a + b) # c",
        ),
        (
            "probe.py",
            "alpha = 1 + 1\nbeta = 2 + 1\nx = 1\ny = a + b\n",
            "py",
            "x = 1 # c",
        ),
        (
            "probe.py",
            "alpha = 1 + 1\nbeta = 2 + 1\nx = 1\ny = a + b\n",
            "py",
            "# c\nx = 1",
        ),
        (
            "probe.rb",
            "alpha = 1 + 1\nbeta = 2 + 1\nx = 1\ny = a + b\n",
            "rb",
            "x = 1 # c",
        ),
        (
            "probe.rb",
            "alpha = 1 + 1\nbeta = 2 + 1\nx = 1\ny = a + b\n",
            "rb",
            "# c\nx = 1",
        ),
        (
            "probe.js",
            "const alpha = 1 + 1;\nlet x = 1;\nlet y = a + b;\n",
            "js",
            "/* c */ a + b",
        ),
        (
            "probe.rs",
            "fn main() {\n    let alpha = 1 + 1;\n    let y = a + b;\n}\n",
            "rust",
            "/* c */ a + b",
        ),
        (
            "probe.ts",
            "const alpha: number = 1 + 1;\nlet y = a + b;\n",
            "ts",
            "/* c */ a + b",
        ),
        (
            "probe.php",
            "<?php\n$alpha = 1 + 1;\n$beta = 2 + 1;\n",
            "php",
            "/* c */ echo 1;",
        ),
        (
            "probe.py",
            "alpha = 1 + 1\nbeta = 2 + 1\nx = 1\ny = a + b\n",
            "py",
            "/* c */ x = 1",
        ),
        (
            "probe.rs",
            "fn main() {\n    let x = 1;\n    let y = a + b;\n}\n",
            "rust",
            "/* c */ let x = 1;",
        ),
        (
            "probe.js",
            "const alpha = 1 + 1;\nlet x = 1;\nlet y = a + b;\n",
            "js",
            "/* c */ let x = 1;",
        ),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "{lang} {pattern:?}: sg 0.45.2 rc8 — $-less comment face must not answer \
                 a silent empty through the template-exists path, answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{lang} {pattern:?} must keep the fail-closed class: {err:#}"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// PASS 91a blast-radius guards: the template-route comment gate must be
// language-aware for `#` (real syntax in c/rust/js — sg ANSWERS those,
// probes_run1/run2 matrix C, one row on BOTH engines today) and must keep
// the accepted block placements (php inline non-leading non-trailing) and
// the comment-free template route byte-identical.
// ---------------------------------------------------------------------------

#[test]
fn f91a_comment_gate_preserves_hash_syntax_and_php_inline_template_faces() {
    // `#`-as-syntax faces: sg 0.45.2 answers one row, subject answers the
    // same line today — the gate must not refuse them (a language-free hash
    // arm at the template route is the regression this pins against).
    for (file, body, lang, pattern, line) in [
        (
            "probe.c",
            "#include <stdio.h>\n\nint main(void) {\n    int alpha = 1 + 1;\n    return 0;\n}\n",
            "c",
            "#include <stdio.h>",
            1u32,
        ),
        (
            "probe.rs",
            "#[derive(Debug)]\nstruct Probe;\n\nfn main() {\n    let alpha = 1 + 1;\n}\n",
            "rust",
            "#[derive($A)]",
            1u32,
        ),
        (
            "probe.js",
            "class C {\n    #x = 0;\n    bump() {\n        this.#x = 1;\n    }\n}\n",
            "js",
            "this.#x = 1",
            4u32,
        ),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        let response = searcher
            .search(&format!("pattern:{pattern}"))
            .unwrap_or_else(|err| panic!("{lang} {pattern:?} must keep answering: {err:#}"));
        let lines: Vec<u32> = response.hits.iter().map(|h| h.line_start).collect();
        assert!(
            lines.contains(&line),
            "{lang} {pattern:?} must keep hitting line {line}: {lines:?} ({:?})",
            response.hits
        );
    }
    // php inline block on the TEMPLATE route (`$`-less): the accepted
    // placement keeps the walk path — sg 0.45.2 rc1 [], subject ok:true
    // (probes_run1 run-to-run cell). A mutant that refuses every block at
    // the template route flips this loud where sg accepts the placement.
    let session = isolated_index_session();
    session.write(
        "probe.php",
        "<?php\n$alpha = 1 + 1;\n$beta = 2 + 1;\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:alpha = 1 + /* c */ 2;")
        .expect("php inline template-route face must keep its accepted placement");
    assert!(
        response.hits.is_empty(),
        "php inline template-route face must keep its honest empty: {:?}",
        response.hits
    );
    // Comment-free `$`-less control on the template route: unchanged.
    // sg 0.45.2 answers line 3 (probes: `x = 1` one row) and so does the
    // subject today.
    let session = isolated_index_session();
    session.write("probe.py", "alpha = 1 + 1\nbeta = 2 + 1\nx = 1\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:x = 1")
        .expect("comment-free $-less template route must keep answering");
    let lines: Vec<u32> = response.hits.iter().map(|h| h.line_start).collect();
    assert_eq!(lines, vec![3u32], "{:?}", response.hits);
}

// ---------------------------------------------------------------------------
// PASS 92 (F-r41-1): 91a's template-route comment gate refused ANY `//`
// (all languages), ANY `#` (py/rb/php) and ANY non-php block placement.
// The probe matrix (gauntlet artifacts/conformance/pass92/probes_run1.jsonl
// + probes_run2.jsonl, subject bf1bd8801e95bd2a vs oracle 0.45.2) shows sg
// ANSWERS embedded placements: rust container/call slots
// (`area( /* note */ 9)` {lit_rs.rs:13}, `calc(1, /* n */ 2)` {slots.rs:5}
// — the two registered oracle faces F-r41-1 unflipped to census-loud rc2),
// rust binary `a + // c\n b` {15}, py floor-div `a // b` (NOT comment
// syntax), and sg lenient-accepts (rc1-empty) embedded line/hash placements
// in every other probed language. Only LEADING and TRAILING line/hash
// placements (and leading/trailing blocks everywhere, inline blocks in
// ruby) are sg-rc8.
// ---------------------------------------------------------------------------

/// RED (F-r41-1): the two registered rust container-slot comment faces must
/// answer their sg-exact sets again (oracle cases P60-HCONF-030-II-COMMENT-SLOT
/// {lit_rs.rs:13} and P63-F62-4-RS-SLOT {slots.rs:5}, sg-exact since passes
/// 60/63 until 91a's blanket block refusal flipped them census-loud rc2).
#[test]
fn f92_rust_container_slot_comment_faces_return_sg_exact_sets() {
    // Verbatim gauntlet fixture corpora (fixtures/pass59_litlane/lit_rs.rs,
    // fixtures/pass63_r13_remediation/corpus_slots/slots.rs) so the
    // registered line numbers hold.
    let lit_rs = "fn area(r: i32) -> i32 {\n    r * r\n}\n\nfn vol(w: i32, h: i32) -> i32 {\n    w * h\n}\n\nfn main() {\n    let a = area(9); /* trail comment */\n    let v = vol(2, 3);\n    let c = vol(2, 3,); // trailing comma in call\n    let d = area( /* mid */ 9);\n    println!(\"{} {}\", a, v);\n}\n";
    let slots = "fn calc(a: u32, b: u32) -> u32 { a + b }\n\nfn demo() -> u32 {\n    let x = calc(1 /* mid */, 2);\n    let y = calc(1, /* n */ 2);\n    let z = calc(3, 7);\n    x + y + z\n}\n";
    for (body, pattern, line) in [
        (lit_rs, "area( /* note */ 9)", 13u32),
        (slots, "calc(1, /* n */ 2)", 5u32),
    ] {
        let session = isolated_index_session();
        session.write("probe.rs", body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        let response = searcher
            .search(&format!("pattern:{pattern}"))
            .unwrap_or_else(|err| {
                panic!("{pattern:?}: sg 0.45.2 answers the registered set — the \
                        container-slot placement must not be refused: {err:#}")
            });
        let lines: Vec<u32> = response.hits.iter().map(|h| h.line_start).collect();
        assert_eq!(
            lines,
            vec![line],
            "{pattern:?} must answer the sg-exact set {{find line {line}}}: {lines:?} ({:?})",
            response.hits
        );
    }
}

/// RED (F-r41-1): embedded (mid-pattern) comment placements must reach the
/// walk again where ingress can deliver them — sg ANSWERS hits on the rust
/// block-slot and py floor-div faces and lenient-accepts (rc1-empty) the
/// rest; 91a's gate refused every cell census-loud. Expected sets are the
/// live 0.45.2 answers on the same corpora (probes_run1.jsonl; block-twin
/// and line-twin faces live on SEPARATE corpora: on a mixed corpus the
/// subject's slot alignment pre-91a also answers the line-comment twin for
/// a block-comment pattern where sg does not — a pre-existing over-match
/// genus the registered corpus never exercises, left byte-untouched).
/// NOTE on the line-twin cells: the pattern contains a literal newline, and
/// the registered user-WIP ingress newline-collapse (91E F-r41-2 fold
/// class) rewrites `calc(1, // line\n 2)` to `calc(1, // line 2)` BEFORE
/// routing — the comment becomes TRAILING, so those faces keep the sg-rc8-
/// class loud posture under the line arm (the embedded-line placement sg
/// accepts is unreachable through the query layer; the collapse is
/// owner=user and byte-untouched here). A mutant that drops the line arm
/// flips these silent.
#[test]
fn f92_template_route_embedded_comment_faces_answer_sg_sets() {
    let rust_block_twin_body = "fn area(r: i32) -> i32 {\n    r * r\n}\n\nfn vol(w: i32, h: i32) -> i32 {\n    w * h\n}\n\nfn main() {\n    let y = a + b;\n    let d = area( /* mid */ 9);\n    let e = calc(1, /* n */ 2);\n}\n";
    for (file, body, lang, pattern, expected) in [
        (
            "probe.rs",
            rust_block_twin_body,
            "rust",
            "area( /* note */ 9)",
            vec![11u32],
        ),
        (
            "probe.rs",
            rust_block_twin_body,
            "rust",
            "calc(1, /* n */ 2)",
            vec![12u32],
        ),
        (
            "probe.py",
            "alpha = 1 + 1\nbeta = 2 + 1\ny = a + b\nz = f(1,\n    2)\nw = [1,\n     2]\nv = a // b\n",
            "py",
            "a // b",
            vec![8u32],
        ),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        let response = searcher
            .search(&format!("pattern:{pattern}"))
            .unwrap_or_else(|err| {
                panic!("{lang} {pattern:?}: sg 0.45.2 accepts the embedded comment \
                        placement — must not be refused census-loud: {err:#}")
            });
        let lines: Vec<u32> = response.hits.iter().map(|h| h.line_start).collect();
        assert_eq!(
            lines, expected,
            "{lang} {pattern:?} must answer the sg-exact set {expected:?} ({:?})",
            response.hits
        );
    }
    // Line-twin spellings (literal newline in the raw pattern): the ingress
    // newline-collapse makes the comment TRAILING before routing, so the
    // sg-rc8-class loud posture holds (sg side of the collapsed spelling is
    // n/a — the collapse is the registered user-WIP class). A mutant that
    // accepts every line placement flips these silent.
    let rust_line_twin_body = "fn main() {\n    let y = a + b;\n    let f = calc(1, // line\n        2);\n}\n";
    for pattern in ["calc(1, // line\n 2)", "a + // c\n b"] {
        let session = isolated_index_session();
        session.write("probe.rs", rust_line_twin_body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "rust {pattern:?}: newline-collapsed to a TRAILING line comment — \
                 the loud posture must hold, answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "rust {pattern:?} must keep the fail-closed class: {err:#}"
            ),
        }
    }
}

/// RED (F-r41-2): fuzz face F26-0614 — ts `1_000 ($A) { $_0x1F }` (no
/// newline) answered silent ok:true [] where sg 0.45.2 rc8s "Multiple AST
/// nodes are detected" (two-statement brace-compound spelling, NeverMatches
/// class via the MixedCase `$_0x1F` token). The probe matrix
/// (probes_run1/run2 matrix F) fixes the sg-rc8 scope: ts/js/py/rust/go/cpp/
/// csharp refuse; php/c/java/rb/swift/kt lenient-accept (rc0/rc1-empty) and
/// KEEP the silent walk answer.
#[test]
fn f92_two_statement_brace_compound_stays_loud_where_sg_rejects() {
    let corpora: &[(&str, &str, &str)] = &[
        (
            "ts",
            "probe.ts",
            "const t = config(1_000, 7);\nfunction gen(a) { return a; }\nif (c) { d(); }\n",
        ),
        (
            "js",
            "probe.js",
            "const t = config(1000, 7);\nfunction gen(a) { return a; }\nif (c) { d(); }\n",
        ),
        ("py", "probe.py", "t = config(1000, 7)\nif c:\n    d()\n"),
        (
            "rust",
            "probe.rs",
            "fn main() {\n    let t = config(1000, 7);\n    if c { d(); }\n}\n",
        ),
        (
            "go",
            "probe.go",
            "package main\n\nfunc main() {\n\tt := config(1000, 7)\n}\n",
        ),
        (
            "cpp",
            "probe.cpp",
            "int main() {\n    int t = config(1000, 7);\n    return 0;\n}\n",
        ),
        (
            "csharp",
            "probe.cs",
            "class Probe {\n    void Main() {\n        int t = config(1000, 7);\n    }\n}\n",
        ),
    ];
    for (lang, file, body) in corpora {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search("pattern:1_000 ($A) { $_0x1F }") {
            Ok(response) => panic!(
                "{lang} 1_000 ($A) {{ $_0x1F }}: sg 0.45.2 rc8 (Multiple AST nodes) — \
                 the two-statement brace-compound face must not answer silent {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{lang} must keep the fail-closed class: {err:#}"
            ),
        }
        // semicolon + multiline twins: same sg-rc8 spelling class (probes_run2).
        for twin in [
            "1_000 ($A) { $_0x1F };",
            "1_000 ($A) {\n  $_0x1F\n}",
        ] {
            match searcher.search(&format!("pattern:{twin}")) {
                Ok(response) => panic!(
                    "{lang} {twin:?}: sg 0.45.2 rc8 — brace-compound twin must not \
                     answer silent {:?}",
                    response.hits
                ),
                Err(err) => assert!(
                    format!("{err:#}").contains("fail-closed"),
                    "{lang} {twin:?} must keep the fail-closed class: {err:#}"
                ),
            }
        }
    }
    // sg lenient-accept witnesses (rc0/rc1-empty): the silent walk answer is
    // the agreement and must NOT move to loud.
    for (lang, file, body) in [
        ("php", "probe.php", "<?php\n$t = config(1000, 7);\nif ($c) { d(); }\n"),
        (
            "c",
            "probe.c",
            "int main(void) {\n    int t = config(1000, 7);\n    return 0;\n}\n",
        ),
        (
            "java",
            "Probe.java",
            "class Probe {\n    void main() {\n        int t = config(1000, 7);\n    }\n}\n",
        ),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        let response = searcher
            .search("pattern:1_000 ($A) { $_0x1F }")
            .unwrap_or_else(|err| {
                panic!("{lang}: sg 0.45.2 lenient-accepts the spelling — must keep \
                        the silent walk answer: {err:#}")
            });
        assert!(
            response.hits.is_empty(),
            "{lang}: sg answers nothing here — honest empty expected: {:?}",
            response.hits
        );
    }
}

/// Preservation (F-r41-1 blast radius): the placements sg 0.45.2 RC8S stay
/// census-loud after the placement narrowing — trailing line `//` in
/// rust/csharp, leading and embedded-binary `//` in ruby (rb `//` is not
/// comment syntax and sg rc8s these spellings), trailing `#` with a newline
/// EOF twin in py/php. These pass pre-fix; their job is to catch an
/// over-narrow repair (mutant: drop the leading/trailing arms entirely).
#[test]
fn f92_template_route_gate_stays_loud_on_sg_rc8_placements() {
    for (file, body, lang, pattern) in [
        (
            "probe.rs",
            "fn main() {\n    let y = a + b;\n}\n",
            "rust",
            "a + b // c",
        ),
        (
            "probe.cs",
            "class Probe {\n    void Main() {\n        int y = a + b;\n    }\n}\n",
            "csharp",
            "a + b // c",
        ),
        ("probe.rb", "y = a + b\nz = f(1,\n  2)\n", "rb", "// c\na + b"),
        ("probe.rb", "y = a + b\nz = f(1,\n  2)\n", "rb", "a + // c\n b"),
        (
            "probe.py",
            "alpha = 1 + 1\nbeta = 2 + 1\nx = 1\n",
            "py",
            "x = 1 # c\n",
        ),
        (
            "probe.php",
            "<?php\n$alpha = 1 + 1;\n$beta = 2 + 1;\n$x = 1;\n",
            "php",
            "x = 1 # c\n",
        ),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "{lang} {pattern:?}: sg 0.45.2 rc8s this placement — must stay \
                 census-loud, answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{lang} {pattern:?} must keep the fail-closed class: {err:#}"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// PASS 94a (FB-93A-3 / F-93B-3 / F-93B-4): the `$`-carrying comment lane and
// the content-blind `#` scan. Probe matrices
// artifacts/conformance/pass94a/probes_run1.jsonl (106 cells) +
// probes_run2.jsonl (46 cells), subject release binary sha16
// 8e9d8432eb7cbee5 vs oracle 0.45.2:
//   - FB-93A-3: sg ANSWERS `$`-carrying inline-block faces in every
//     non-ruby/non-py block-comment language (rust `calc($A, /* n */ $B)`
//     hits[10], ts/js/go/java/php/c/cpp/csharp/swift/kotlin call/operand/
//     assignment-RHS cells — run1 cells 1-77), while the subject rc2s them
//     census-loud: the lang-side lane comment refusal is placement-unaware
//     for every `$`-carrying pattern and the core lets that refusal stand.
//     The 94a core-side fix stops the PRE-refusal (placement-aware
//     acceptance in the template gate + the lane override + the
//     comment-carrying ingress exemption), but the native walker has NO
//     meta+pattern-side-comment slot (match_pattern is lang/** — sibling
//     94b's surface), so these faces still end LOUD via the post-walk
//     `needs_ast_grep_fallback` backstop: fail-closed, never silent
//     (`f94a_dollar_carrying_inline_block_faces_stay_loud_pending_walker_lane`,
//     `f94a_sg_lenient_inline_comment_faces_stay_loud_pending_walker_lane`).
//     Reconciler note: they flip to sg-exact when the walker gains
//     meta+comment slots; expected line sets are preserved in run1.
//     Ruby inline stays rc1-lenient-loud (registered genus) and python has
//     no block comments (rc0/rc1 registered genus). Leading/trailing/line
//     placements stay sg-rc8 or registered-loud (zero movement).
//   - F-93B-3: ruby `/.../ ` regex literals — `#` inside the literal or its
//     `#{...}` interpolation is NOT a comment (run2 cells 1-5: sg hits) but
//     the byte scan flags it; division `#` comments stay sg-rc8 (run2
//     cells 8-9, 12-14).
//   - F-93B-4: php 8 attributes `#[...]` — a LEADING attribute (optionally
//     followed by a `function` declaration) is real syntax sg answers
//     (run2 cells 18-23, 27, 31: hits / rc0-empty) while the byte scan
//     flags `#`; attribute after code, attribute+property tails, and
//     `# [space]` comments stay sg-rc8 (run2 cells 24-25, 29, 34). ts/js
//     `#[` is INVALID syntax sg rc8s (run2 cells 37-38) where the subject
//     answered a fail-open silent empty. ruby/python `#[` are comments
//     (sg rc8 — run2 cells 35-36, 39) and stay refused.
// ---------------------------------------------------------------------------

const F94A_RUST_CORPUS: &str = "fn calc(a: u32, b: u32) -> u32 {\n    a + b\n}\n\nfn area(r: u32) -> u32 {\n    r\n}\n\nfn demo() -> u32 {\n    let x = calc(1, /* n */ 2);\n    let y = area( /* note */ 9);\n    let z = 1 + /* c */ 2;\n    let w = /* w */ 3;\n    let v = calc(3, 7);\n    let t = a + b;\n    z + w + v + t\n}\n";
const F94A_TS_CORPUS: &str = "function f(a: number, b: number): number {\n  return a + b;\n}\nfunction demo(): void {\n  const x = f(1, /* n */ 2);\n  const y = g( /* note */ 9);\n  const z = 1 + /* c */ 2;\n  const w = /* w */ 3;\n  const v = h(3, 7);\n  const t = a + b;\n  const obj = { m: (k: number) => k };\n  const r = obj.m(/* m */ 5);\n  const t2 = config(1000, 7);\n}\n";
const F94A_JS_CORPUS: &str = "function f(a, b) {\n  return a + b;\n}\nfunction demo() {\n  var x = f(1, /* n */ 2);\n  var y = g( /* note */ 9);\n  var z = 1 + /* c */ 2;\n  var w = /* w */ 3;\n  var v = h(3, 7);\n  var t = a + b;\n  var obj = { m: function (k) { return k; } };\n  var r = obj.m(/* m */ 5);\n}\n";
const F94A_GO_CORPUS: &str = "package main\n\nfunc f(a int, b int) int {\n\treturn a + b\n}\n\nfunc demo() {\n\tx := f(1, /* n */ 2)\n\ty := g( /* note */ 9)\n\tz := 1 + /* c */ 2\n\tw := /* w */ 3\n\tv := h(3, 7)\n\tt := a + b\n\t_, _, _, _, _, _ = x, y, z, w, v, t\n}\n";
const F94A_JAVA_CORPUS: &str = "class Probe {\n    int f(int a, int b) { return a + b; }\n    void demo() {\n        int x = f(1, /* n */ 2);\n        int y = g( /* note */ 9);\n        int z = 1 + /* c */ 2;\n        int w = /* w */ 3;\n        int v = h(3, 7);\n        int t = a + b;\n    }\n}\n";
const F94A_PHP_CORPUS: &str = "<?php\n$x = f(1, /* n */ 2);\n$y = g( /* note */ 9);\n$z = 1 + /* c */ 2;\n$w = /* w */ 3;\n$v = h(3, 7);\n$t = $a + $b;\n$obj = new Obj();\n$r = $obj->m(/* m */ 5);\n$q = $obj?->n(/* q */ 7);\n$u = $v1;\n";
const F94A_C_CORPUS: &str = "int f(int a, int b) { return a + b; }\nvoid demo(void) {\n    int x = f(1, /* n */ 2);\n    int y = g( /* note */ 9);\n    int z = 1 + /* c */ 2;\n    int w = /* w */ 3;\n    int v = h(3, 7);\n    int t = a + b;\n}\n";
const F94A_CPP_CORPUS: &str = "int f(int a, int b) { return a + b; }\nvoid demo(void) {\n    int x = f(1, /* n */ 2);\n    int y = g( /* note */ 9);\n    int z = 1 + /* c */ 2;\n    int w = /* w */ 3;\n    int v = h(3, 7);\n    int t = a + b;\n}\n";
const F94A_CS_CORPUS: &str = "class Probe {\n    int F(int a, int b) { return a + b; }\n    void Demo() {\n        int x = F(1, /* n */ 2);\n        int y = G( /* note */ 9);\n        int z = 1 + /* c */ 2;\n        int w = /* w */ 3;\n        int v = H(3, 7);\n        int t = a + b;\n    }\n}\n";
const F94A_SWIFT_CORPUS: &str = "func f(_ a: Int, _ b: Int) -> Int { return a + b }\nfunc demo() {\n    let x = f(1, /* n */ 2)\n    let y = g( /* note */ 9)\n    let z = 1 + /* c */ 2\n    let w = /* w */ 3\n    let v = h(3, 7)\n    let t = a + b\n}\n";
const F94A_KT_CORPUS: &str = "fun f(a: Int, b: Int): Int { return a + b }\nfun demo() {\n    val x = f(1, /* n */ 2)\n    val y = g( /* note */ 9)\n    val z = 1 + /* c */ 2\n    val w = /* w */ 3\n    val v = h(3, 7)\n    val t = a + b\n}\n";
const F94A_RB_CORPUS: &str = "re = /ab#{x}cd/\ny2 = /foo#{bar}(.*)/i\nz2 = a / b\nq2 = x / y2\nr2 = a / b # floor division\nh2 = /a#b/\nw2 = 1 / 2 # trailing comment\nm2 = (p1 + p2) / 2 # tail\nn2 = p1 / p2\ncalc(1, /* n */ 2)\ng( /* note */ 9)\nv2 = 1 + 2\n";
const F94A_PHP_ATTR_CORPUS: &str = "<?php\n#[Route('/home')]\nfunction home() { return 1; }\n#[Route('/away')] function away() { return 2; }\nclass C {\n    #[Id]\n    private $id;\n}\n$alpha = 1;\n$beta = 2;\n";

/// RED (FB-93A-3): the 18 `$`-carrying inline-block faces below are sg
/// 0.45.2-ANSWERABLE (run1 cells 1-77: rust `calc($A, /* n */ $B)` hits[10]
/// and the ts/js/go/java/php/c/cpp/csharp/swift/kotlin call/operand/
/// assignment-RHS cells). Pre-fix the subject rc2'd them via the core-side
/// placement gate BEFORE any walk. The 94a fix removes that pre-refusal
/// (placement-aware acceptance now lets the inline block through the gate
/// for every non-ruby/non-py language), but the native walker has NO
/// meta+pattern-side-comment lane (match_pattern is `ast-sgrep-lang` —
/// sibling 94b's surface; probed: `calc($A, $B)` answers {10,14},
/// `calc(1, /* n */ 2)` answers {10}, `calc($A, /* n */ 2)` cannot bind),
/// so every face below still ends loud through the post-walk
/// `needs_ast_grep_fallback` backstop — fail-closed, NOT silent. This pin
/// freezes that contract: the routing layer must not pre-refuse (the
/// `fail-closed` error here comes from the backstop, whose disjunct is
/// keyed on the same `needs_ast_grep_fallback` the ingress exemption
/// preserves), and no future core-side loosening may turn these faces into
/// a silent `ok:true []` — that would be the forbidden loud→silent flip.
/// Reconciler note: these 18 cells become sg-EXACT ({10}/{11}/{12}/... per
/// run1) the moment the walker gains meta+comment slots; at that point the
/// backstop disjunct stops firing and THIS pin's assertion must be
/// upgraded to the sg-exact line sets (expected values are preserved in the
/// run1 adjudicated matrix) — flip the assertion, not the gate.
#[test]
fn f94a_dollar_carrying_inline_block_faces_stay_loud_pending_walker_lane() {
    for (file, body, lang, pattern) in [
        ("probe.rs", F94A_RUST_CORPUS, "rust", "calc($A, /* n */ $B)"),
        ("probe.rs", F94A_RUST_CORPUS, "rust", "area( /* note */ $A)"),
        ("probe.rs", F94A_RUST_CORPUS, "rust", "$A + /* c */ $B"),
        ("probe.rs", F94A_RUST_CORPUS, "rust", "let $A = /* w */ $B;"),
        ("probe.ts", F94A_TS_CORPUS, "typescript", "f($A, /* n */ $B)"),
        ("probe.ts", F94A_TS_CORPUS, "typescript", "const $A = /* w */ $B;"),
        ("probe.js", F94A_JS_CORPUS, "javascript", "f($A, /* n */ $B)"),
        ("probe.go", F94A_GO_CORPUS, "go", "f($A, /* n */ $B)"),
        ("probe.go", F94A_GO_CORPUS, "go", "$A := /* w */ $B"),
        ("Probe.java", F94A_JAVA_CORPUS, "java", "f($A, /* n */ $B)"),
        ("probe.php", F94A_PHP_CORPUS, "php", "f($A, /* n */ $B)"),
        ("probe.php", F94A_PHP_CORPUS, "php", "$A + /* c */ $B"),
        ("probe.c", F94A_C_CORPUS, "c", "f($A, /* n */ $B)"),
        ("probe.c", F94A_C_CORPUS, "c", "$A + /* c */ $B"),
        ("probe.cpp", F94A_CPP_CORPUS, "cpp", "f($A, /* n */ $B)"),
        ("probe.cs", F94A_CS_CORPUS, "csharp", "F($A, /* n */ $B)"),
        ("probe.swift", F94A_SWIFT_CORPUS, "swift", "f($A, /* n */ $B)"),
        ("probe.kt", F94A_KT_CORPUS, "kotlin", "f($A, /* n */ $B)"),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "{lang} {pattern:?}: sg 0.45.2 answers this face (run1 cells \
                 1-77) and the walker cannot yet — until the lang/** walker \
                 gains meta+comment slots the backstop must keep it LOUD \
                 fail-closed; a silent answer here is the forbidden \
                 loud-to-silent flip: answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{lang} {pattern:?} must keep the fail-closed class: {err:#}"
            ),
        }
    }
}

/// RED (FB-93A-3 lenient flank): the inline-block faces sg 0.45.2
/// LENIENT-ACCEPTS as empty (rc1/rc0 — run1 cells 92, 93, 101) share the
/// walker-boundary contract of the sibling pin above: the 94a routing fix
/// stopped pre-refusing them, the walker cannot bind `$` inside a
/// comment-slot member call (`$obj.m(/* m */ $A)`), and the backstop keeps
/// the face LOUD fail-closed instead of the fail-open silent empty the
/// subject used to answer. Silent here would be the forbidden loud-to-silent
/// flip; sg-exactness (honest empty) arrives with the walker's meta+comment
/// lane (lang/** reconciler note).
#[test]
fn f94a_sg_lenient_inline_comment_faces_stay_loud_pending_walker_lane() {
    for (file, body, lang, pattern) in [
        ("probe.ts", F94A_TS_CORPUS, "typescript", "$obj.m(/* m */ $A)"),
        ("probe.js", F94A_JS_CORPUS, "javascript", "$obj.m(/* m */ $A)"),
        ("probe.php", F94A_PHP_CORPUS, "php", "$A /* c */ = $B"),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "{lang} {pattern:?}: sg 0.45.2 lenient-accepts this face empty \
                 (run1 cells 92/93/101) and the walker cannot yet — the backstop \
                 must keep it LOUD fail-closed until the lang/** walker gains \
                 meta+comment slots; a silent answer is the forbidden \
                 loud-to-silent flip: answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{lang} {pattern:?} must keep the fail-closed class: {err:#}"
            ),
        }
    }
}

/// RED (F-93B-4): php 8 attributes the WALKER can answer today. A LEADING
/// literal `#[Route('/home')]` is real syntax sg ANSWERS (run2 cell 18:
/// hits[2]); `#[ ]` degenerate is sg rc0-empty (cell 27) — the honest empty
/// agrees. The language-aware scan clears the `#`/`#[` flags for the
/// leading-attribute face (optionally followed by a `function` declaration)
/// so the py/rb/php hash arm cannot refuse real syntax.
///
/// The `$`-carrying twins (`#[Route($X)]`, with and without the trailing
/// `function` face, single-line and newline spellings) are ALSO sg-answered
/// (run2 cells 19-23) but the walker cannot bind metas inside an attribute
/// argument list — they stay census-loud via the `php_attribute_carved`
/// carve-out and are pinned loud in
/// [`f94a_comment_and_attribute_gate_stays_loud_where_sg_is_loud`]. Moving
/// them here un-carved would answer a silent empty: the loud-to-silent flip
/// the constraints forbid.
#[test]
fn f94a_php_attribute_faces_answer_sg_sets() {
    for (pattern, expected) in [
        ("#[Route('/home')]", Some(vec![2u32])),
        ("#[ ]", None),
    ] {
        let session = isolated_index_session();
        session.write("probe.php", F94A_PHP_ATTR_CORPUS);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match (
            expected,
            searcher.search(&format!("pattern:{pattern}")),
        ) {
            (Some(expected), Ok(response)) => {
                let lines: Vec<u32> = response.hits.iter().map(|h| h.line_start).collect();
                assert_eq!(
                    lines, expected,
                    "php {pattern:?} must answer the sg-exact set {expected:?} ({:?})",
                    response.hits
                );
            }
            (None, Ok(response)) => assert!(
                response.hits.is_empty(),
                "php {pattern:?}: sg rc0-empty (run2 cell 27) — the walk must \
                 agree with an honest empty, got {:?}",
                response.hits
            ),
            (_, Err(err)) => panic!(
                "php {pattern:?}: sg 0.45.2 answers this attribute face (run2) — \
                 must not die census-loud: {err:#}"
            ),
        }
    }
}

/// RED (F-93B-3): ruby regex literals — `#` inside a `/.../ ` literal or its
/// `#{...}` interpolation is NOT a comment; sg answers the faces (run2 cells
/// 1-5) while the byte scan flags the hash and the gate refuses.
#[test]
fn f94a_ruby_regex_literal_hash_faces_answer_sg_sets() {
    for (pattern, expected) in [
        ("re = /ab#{x}cd/", vec![1u32]),
        ("y2 = /foo#{bar}(.*)/i", vec![2u32]),
        ("/foo#{bar}(.*)/i", vec![2u32]),
        ("h2 = /a#b/", vec![6u32]),
        ("/a#b/", vec![6u32]),
    ] {
        let session = isolated_index_session();
        session.write("probe.rb", F94A_RB_CORPUS);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        let response = searcher
            .search(&format!("pattern:{pattern}"))
            .unwrap_or_else(|err| {
                panic!("ruby {pattern:?}: sg 0.45.2 answers the regex-literal face \
                        (run2 cells 1-5) — the `#` inside the literal is not a \
                        comment and must not be refused: {err:#}")
            });
        let lines: Vec<u32> = response.hits.iter().map(|h| h.line_start).collect();
        assert_eq!(
            lines, expected,
            "ruby {pattern:?} must answer the sg-exact set {expected:?} ({:?})",
            response.hits
        );
    }
}

/// Preservation (FB-93A-3/F-93B-3/F-93B-4 blast radius): every cell sg
/// 0.45.2 itself refuses — or refuses-loud as the registered lenient genus —
/// STAYS census-loud after the lane override and the scan refinements.
/// run1 cells 5-7, 12-14, 19, 21, 87-91, 105; run2 cells 8-9, 12-14, 24-25,
/// 29, 34-39. The ts/js `#[` cells are ALSO sg-rc8 and are the silent
/// fail-opens this round closes (run2 cells 37-38: pre-fix ok:true []).
#[test]
fn f94a_comment_and_attribute_gate_stays_loud_where_sg_is_loud() {
    for (file, body, lang, pattern) in [
        // trailing / leading / line placements (run1 cells 5-7)
        ("probe.rs", F94A_RUST_CORPUS, "rust", "$A + $B /* c */"),
        ("probe.rs", F94A_RUST_CORPUS, "rust", "/* c */ $A + $B"),
        ("probe.rs", F94A_RUST_CORPUS, "rust", "$A + $B // c"),
        // ts/js rc1-lenient trailing/line cells (registered census-loud genus)
        ("probe.ts", F94A_TS_CORPUS, "typescript", "$A + $B /* c */"),
        ("probe.ts", F94A_TS_CORPUS, "typescript", "$A + $B // c"),
        ("probe.js", F94A_JS_CORPUS, "javascript", "$A + $B /* c */"),
        ("probe.js", F94A_JS_CORPUS, "javascript", "$A + $B // c"),
        // ruby inline blocks: rc1-lenient / rc8 (run1 cells 85-91)
        ("probe.rb", F94A_RB_CORPUS, "ruby", "calc($A, /* n */ $B)"),
        ("probe.rb", F94A_RB_CORPUS, "ruby", "$A + /* c */ $B"),
        ("probe.rb", F94A_RB_CORPUS, "ruby", "$A + $B /* c */"),
        // ruby division `#` comments are REAL comments (run2 cells 8-9, 12-14)
        ("probe.rb", F94A_RB_CORPUS, "ruby", "a / b # c"),
        ("probe.rb", F94A_RB_CORPUS, "ruby", "x = 1 / 2 # c"),
        // py/rb `#[` and py hash: comment syntax (run2 cells 35-36, 39, 16)
        ("probe.rb", F94A_RB_CORPUS, "ruby", "#[Attr]\nv2 = 1 + 2"),
        ("probe.py", "x = f(1, 2)\nz = 1 + 2\n", "python", "x = 1 # c"),
        ("probe.py", "x = f(1, 2)\nz = 1 + 2\n", "python", "#[Attr]\nx = f(1, 2)"),
        // php assignment + comment faces sg rc8s (run1 cell 39; run2 40, 42-44, 46)
        ("probe.php", F94A_PHP_CORPUS, "php", "$A = /* w */ $B;"),
        ("probe.php", F94A_PHP_CORPUS, "php", "$A = $B;"),
        ("probe.php", F94A_PHP_CORPUS, "php", "$A = f(/* w */ $B);"),
        ("probe.php", F94A_PHP_CORPUS, "php", "$A = $B /* w */;"),
        // php attribute faces sg rc8s (run2 cells 24-25, 29, 34)
        ("probe.php", F94A_PHP_ATTR_CORPUS, "php", "#[Id]\nprivate $id;"),
        (
            "probe.php",
            F94A_PHP_ATTR_CORPUS,
            "php",
            "function home() { return 1; } #[Route('/x')] function away() { return 2; }",
        ),
        ("probe.php", F94A_PHP_ATTR_CORPUS, "php", "$x # [y] + 1"),
        // php `$`-carrying attribute faces sg ANSWERS (run2 cells 19-23) but
        // the walker cannot bind metas inside an attribute argument list —
        // the `php_attribute_carved` carve-out keeps them census-loud (the
        // un-carved override would walk them to a silent empty: the
        // loud-to-silent flip the constraints forbid). sg-exactness
        // ([2] / [2,4] per run2) arrives with the walker's meta+comment lane.
        ("probe.php", F94A_PHP_ATTR_CORPUS, "php", "#[Route($X)]"),
        (
            "probe.php",
            F94A_PHP_ATTR_CORPUS,
            "php",
            "#[Route($X)] function home() { return 1; }",
        ),
        (
            "probe.php",
            F94A_PHP_ATTR_CORPUS,
            "php",
            "#[Route($X)]\nfunction home() { return 1; }",
        ),
        // ts/js `#[` is invalid syntax — sg rc8 (run2 cells 37-38); the
        // subject answered a silent fail-open ok:true [] pre-fix.
        ("probe.ts", F94A_TS_CORPUS, "typescript", "#[Attr]\nconst t2 = config(1000, 7);"),
        ("probe.js", F94A_JS_CORPUS, "javascript", "#[Attr]\nvar v = h(3, 7);"),
        // NeverMatches + inline block keeps the registered loud routing
        // (run1 cell 105: sg rc8).
        ("probe.ts", F94A_TS_CORPUS, "typescript", "$f($A) { /* c */ $_0x1F }"),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "{lang} {pattern:?}: sg 0.45.2 refuses this face (or holds the \
                 registered lenient-loud genus) — must stay census-loud, \
                 answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{lang} {pattern:?} must keep the fail-closed class: {err:#}"
            ),
        }
    }
}

/// PASS 96 probe corpora (byte-identical to the pass96 y_* corpora; the rust
/// aux file carries NO µ bytes — it exists to pin the byte-prefilter guard:
/// a µ-spelled meta pattern must not drop µ-free files from the walk).
const F96_RUST_MAIN: &str = "fn main() {\n    \u{b5}A + 1;\n    x + 1;\n    \u{b5} + 1;\n    f(9);\n    f(y);\n    \u{b5}A;\n    zz;\n    g(\"hw\");\n    g(\"other\");\n    w = v;\n}\n";
const F96_RUST_AUX: &str = "fn aux() {\n    q + 1;\n    r + 2;\n}\n";
const F96_PY_BODY: &str = "\u{b5}A + 1\nx + 1\n\u{b5} + 1\nf(9)\nf(y)\n\u{b5}A\nzz\ng(\"hw\")\ng(\"other\")\nw = v\n";
const F96_RUBY_BODY: &str = "\u{b5}A + 1\nx + 1\n\u{b5} + 1\nf(9)\nf(y)\n\u{b5}A\nzz\ng(\"hw\")\ng(\"other\")\nw = v\n";
const F96_JS_BODY: &str = "\u{b5}A + 1\nx + 1\n\u{b5} + 1\nf(9)\nf(y)\n\u{b5}A\nzz\ng(\"hw\")\ng(\"other\")\nw = v\n";
const F96_JAVA_BODY: &str = "class T {\n    void m() {\n    \u{b5}A + 1;\n    x + 1;\n    \u{b5} + 1;\n    f(9);\n    f(y);\n    \u{b5}A;\n    zz;\n    g(\"hw\");\n    g(\"other\");\n    w = v;\n    }\n}\n";

fn f96_hits(session: &IsolatedIndexSession, pattern: &str) -> Vec<(String, u32)> {
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 64,
        ..session.search_options()
    });
    searcher
        .search(&format!("pattern:{pattern}"))
        .expect("pattern search")
        .hits
        .iter()
        .map(|h| (h.file.clone(), h.line_start))
        .collect()
}

/// RED (F-95A-1, search ingress): the expando spellings must route exactly
/// like their `$` twins at the CLI search surface.
/// - root `µµµ` / `µµµ + 1`: sg 0.45.2 rc8 (RootMultiMetaVar / census) — the
///   search must fail closed LOUD (pre-fix: silent ok:true [] = fail-open).
/// - `µµA` (≡ `$$A`): sg answers every node line — must answer, not silent.
/// - `µA + 1` (≡ `$A + 1`): must answer the meta hit set INCLUDING the
///   µ-free aux file (a byte prefilter keyed on the µ spelling would drop
///   exactly the files sg's meta reading answers).
/// - java control: no expando — literal faces unchanged.
#[test]
fn f96_expando_spelling_search_ingress_folds_to_dollar_behavior() {
    // root multi meta: LOUD (sg rc8 fold) for both spellings
    let session = isolated_index_session();
    session.write("lib.rs", F96_RUST_MAIN);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    for pattern in ["µµµ", "µµµ + 1", "$$$ + 1"] {
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "rust {pattern:?}: sg 0.45.2 rc8 / registered loud genus — the \
                 expando spelling must fold to the $$$ spelling's loud class, \
                 answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "rust {pattern:?} must stay fail-closed loud: {err:#}"
            ),
        }
    }
    // µµA answers every node line (the $$A universal twin)
    let session = isolated_index_session();
    session.write("lib.rs", F96_RUST_MAIN);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let hits = f96_hits(&session, "µµA");
    let mut lines: Vec<u32> = hits.iter().map(|(_, l)| *l).collect();
    lines.sort_unstable();
    lines.dedup();
    assert_eq!(
        lines,
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        "rust µµA ≡ $$A: sg answers every node line (matrix m1 cell 3/4)"
    );
    // µA + 1 answers the meta set including the µ-FREE aux file
    let session = isolated_index_session();
    session.write("lib.rs", F96_RUST_MAIN);
    session.write("aux.rs", F96_RUST_AUX);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let hits = f96_hits(&session, "µA + 1");
    assert!(
        hits.iter().any(|(f, l)| f == "aux.rs" && *l == 2),
        "µA + 1 must meta-match `q + 1` in the µ-free aux file (sg meta \
         reading; a µ-byte prefilter would drop it): {hits:?}"
    );
    let main_lines: Vec<u32> = hits
        .iter()
        .filter(|(f, _)| f == "lib.rs")
        .map(|(_, l)| *l)
        .collect();
    assert_eq!(
        main_lines,
        vec![2, 3, 4],
        "lib.rs µA + 1 ≡ $A + 1 meta set (matrix m1 cell 1/2 oracle)"
    );
    // java control: no expando — the literal faces keep their sets
    let session = isolated_index_session();
    session.write("T.java", F96_JAVA_BODY);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let hits = f96_hits(&session, "µA + 1");
    assert_eq!(
        hits.iter().map(|(_, l)| *l).collect::<Vec<u32>>(),
        vec![3],
        "java µA + 1: literal face unchanged (matrix m1 java control)"
    );
}

/// RED (FB-95B-2, search ingress): the sg accepted-empty bracket fragments
/// must answer an honest empty `ok` (sg rc0) instead of the ingress rc2;
/// the sg-rc8 spellings (ruby all tails, js/ts `}`) stay LOUD.
#[test]
fn f96_bracket_fragment_search_empty_where_sg_accepts_empty() {
    let session = isolated_index_session();
    session.write("t.py", F96_PY_BODY);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    match searcher.search("pattern:$A ]") {
        Ok(response) => assert!(
            response.hits.is_empty(),
            "py $A ]: sg accepted-empty — the honest empty is the agreement, \
             answered {:?}",
            response.hits
        ),
        Err(err) => panic!(
            "py $A ]: sg 0.45.2 accepts this ERROR-repair fragment (rc0 \
             empty) — the search must not fail closed: {err:#}"
        ),
    }
    // ruby corpus: sg rc8 — stays loud
    let session = isolated_index_session();
    session.write("t.rb", F96_RUBY_BODY);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    match searcher.search("pattern:$A ]") {
        Ok(response) => panic!(
            "ruby $A ]: sg 0.45.2 rc8 (multi-root) — must stay census-loud, \
             answered {:?}",
            response.hits
        ),
        Err(err) => assert!(
            format!("{err:#}").contains("fail-closed"),
            "ruby $A ] must keep the fail-closed class: {err:#}"
        ),
    }
}

/// RED (FB-95B-1, search ingress): the whole-content string meta must answer
/// at the search surface (sg {8,9} on the js body), not ingress-rc2.
#[test]
fn f96_string_meta_search_answers_sg_set() {
    let session = isolated_index_session();
    session.write("t.js", F96_JS_BODY);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let hits = f96_hits(&session, "g(\"$A\")");
    let mut lines: Vec<u32> = hits.iter().map(|(_, l)| *l).collect();
    lines.sort_unstable();
    lines.dedup();
    assert_eq!(
        lines,
        vec![8, 9],
        "js g(\"$A\"): sg binds the whole-content string meta and answers \
         both string call sites (matrix ms_instring_meta)"
    );
}

// ---------------------------------------------------------------------------
// PASS 98 (r48 remediation): search-ingress pins for the F-97A-1 whole-node
// expando validation (incl. the $-spelled layer-2 literals), the F-97A-2
// $-less unbalanced-}-tail census governance, and the F97X-0072 µ-before-$
// composition. Probe matrices: artifacts/conformance/pass98/matrix/
// (oracle 0.45.2 ATTACHED 2026-09-09; subject binary d3b5a048052c4c88).
// ---------------------------------------------------------------------------

/// Byte-identical to the pass98 c98_wholenode rust corpus (mixed-tail rows
/// appended to the pass-96 shape) + the µ-free aux file from pass 96.
const F98_RUST_MAIN: &str = "fn main() {\n    \u{b5}A + 1\n    x + 1\n    \u{b5} + 1\n    f(9)\n    f(y)\n    \u{b5}A\n    zz\n    g(\"hw\")\n    g(\"other\")\n    w = v\n    \u{b5}Bx + 1\n    k(\u{b5}Bx)\n    f(\"\u{b5}Ax\")\n    \u{b5}Able\n    \u{b5}Ab\n    \u{b5}\u{b5}Able\n    \u{b5}\u{b5}\u{b5}ABle\n}\n";
const F98_COMPOSE_PY: &str = "msg = \"hello \" + name\nw = v\ny = z + 1\n";

/// RED (F-97A-1, search ingress): the whole-node literal faces must answer at
/// the CLI search surface under BOTH spellings — sg answers `µAble` {15} and
/// (layer 2) `k($Bx)` {13} on this corpus (m3a/m3a2 oracle sets). The r46
/// prefix-greedy normalizer rewrote `µAble` → `$Able` (silent []), and the
/// pass-54 literal lane's $-name scoping left the $ spelling silent too.
#[test]
fn f98_wholenode_literal_search_answers_sg_set() {
    let session = isolated_index_session();
    session.write("lib.rs", F98_RUST_MAIN);
    session.write("aux.rs", F96_RUST_AUX);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let hits = f96_hits(&session, "µAble");
    assert_eq!(
        hits.iter().map(|(_, l)| *l).collect::<Vec<u32>>(),
        vec![15],
        "rust µAble: sg answers the literal identifier row (m3a) — the whole \
         node text fails the meta grammar, verbatim bytes must reach the walk"
    );
    let hits = f96_hits(&session, "k($Bx)");
    assert_eq!(
        hits.iter().map(|(_, l)| *l).collect::<Vec<u32>>(),
        vec![13],
        "rust k($Bx): sg preprocesses to k(µBx) and answers the literal row \
         under the $ spelling too (m3a2) — never a silent empty"
    );
    let hits = f96_hits(&session, "k(µBx)");
    assert_eq!(
        hits.iter().map(|(_, l)| *l).collect::<Vec<u32>>(),
        vec![13],
        "rust k(µBx): the µ spelling answers the same literal row (m3a)"
    );
}

/// RED (F-97A-2, search ingress): sg 0.45.2 rc8-refuses `q }` in js/ts (m3b);
/// the subject walked it silent ok-empty because the census-loud backstop only
/// consulted the language-aware fragment gate for `$`-carrying patterns. The
/// census must govern $-less faces too (pass-90/91 template-route precedent).
/// sg accepted-empty tails (py `q }`, js `q ]`) must stay honest empties.
#[test]
fn f98_dollar_less_brace_tail_search_stays_loud_like_sg() {
    // js corpus: sg rc8 — the search must fail closed LOUD.
    let session = isolated_index_session();
    session.write("t.js", F96_JS_BODY);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    match searcher.search("pattern:q }") {
        Ok(response) => panic!(
            "js q }}: sg 0.45.2 rc8 (multi-root parse, m3b) — must stay \
             census-loud, answered {:?}",
            response.hits
        ),
        Err(err) => assert!(
            format!("{err:#}").contains("fail-closed"),
            "js q }} must take the fail-closed loud path: {err:#}"
        ),
    }
    // py corpus: sg ACCEPTS the same tail empty (m3b) — must stay an honest ok.
    let session = isolated_index_session();
    session.write("t.py", F96_PY_BODY);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    match searcher.search("pattern:q }") {
        Ok(response) => assert!(
            response.hits.is_empty(),
            "py q }}: sg accepts-empty — the honest empty is the agreement, \
             answered {:?}",
            response.hits
        ),
        Err(err) => panic!(
            "py q }}: sg 0.45.2 accepts this tail (rc0/rc1 empty, m3b) — the \
             search must not fail closed: {err:#}"
        ),
    }
}

/// RED (F97X-0072, search ingress): py `$A = µµ$A` must fold to its
/// `$`-twin's registered loud class (the µµ$ run composes to the $$$A
/// MultiCapture reading; m3c oracle sets). Pre-fix the mixed spelling walked
/// silent ok-empty while the twin failed closed — a twin-convergence break.
#[test]
fn f98_expando_composition_search_loud_like_twin() {
    for (file, body, pattern) in [
        ("t.py", F98_COMPOSE_PY, "$A = µµ$A"),
        ("lib.rs", F96_RUST_MAIN, "$A = µµ$A"),
    ] {
        let session = isolated_index_session();
        session.write(file, body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "{file} {pattern:?}: the composed µµ$ run must fold to the \
                 $$$A twin's registered loud class (m3c), answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "{file} {pattern:?} must be loud exactly like its $ twin: {err:#}"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// PASS 100 (r50) — FB-99A-1 / FB-99A-2 / FB-99B-1 search-surface pins.
// Oracle sets: artifacts/conformance/pass100/matrix/*.jsonl (ast-grep
// 0.45.2, probed 2026-09-09).
// ---------------------------------------------------------------------------

const F100_GLU_PY: &str = "\u{b5}A_ + 1\n\u{b5}A1 + 1\ny + 1\n\u{b5}\u{b5}\u{b5}$A + 1\n\u{b5}\u{b5}$A + 1\n$A\u{b5}\u{b5}B\nfoo\u{b5}\u{b5}\u{b5}$A\n$$$A + 1\n$$A + 1\n$A$$B\nfoo$$$A\n\u{b5}Ax\n";
const F100_GLU_JS: &str = "    \u{b5}A_ + 1\n    \u{b5}A1 + 1\n    y + 1\n    \u{b5}\u{b5}\u{b5}$A + 1\n    \u{b5}\u{b5}$A + 1\n    $A\u{b5}\u{b5}B\n    foo\u{b5}\u{b5}\u{b5}$A\n    $$$A + 1\n    $$A + 1\n    $A$$B\n    foo$$$A\n    \u{b5}Ax\n";
const F100_RUBY_SRC: &str = "\u{b5}A?\n\u{b5}A!\n\u{b5}Ab?\n\u{b5}Ab!\nx = \u{b5}A?\ndef \u{b5}A?\nend\ndef \u{b5}A!\nend\ny = 1\n";

/// RED (FB-99A-1, search ingress): the error-glued `µµµ$A + 1`-class rows.
/// sg 0.45.2 answers the meta patterns on the glued rows too — its parse
/// keeps the binary LHS as the named identifier fragment (the `$A` ERROR
/// text rides as an extra child, probed is_extra=true) and the meta binds
/// the fragment (`metaVariables.single.A_ = "µµµ"`). The search must answer
/// the full sg set, never the silent under-answer.
#[test]
fn f100_glued_fragment_search_answers_sg_set() {
    let session = isolated_index_session();
    session.write("t.py", F100_GLU_PY);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    for (pattern, want) in [
        ("\u{b5}A_ + 1", vec![1u32, 2, 3, 4, 5, 8, 9]),
        ("$$A + 1", vec![1, 2, 3, 4, 5, 8, 9]),
    ] {
        let hits = f96_hits(&session, pattern);
        assert_eq!(
            hits.iter().map(|(_, l)| *l).collect::<Vec<u32>>(),
            want,
            "py {pattern:?}: sg answers the glued `µµµ$A + 1`/`µµ$A + 1` rows \
             with the fragment bound (m1 oracle set) — never a silent \
             under-answer"
        );
    }
}

/// RED (FB-99A-2, search ingress): no-expando whole-token literal faces.
/// In js/ts/java `µµµ$A`-class tokens are ordinary identifiers; sg answers
/// the verbatim rows. Pre-fix the embedded `$A` glued a placeholder into the
/// identifier leaf (text compare could never match) or the language-free
/// ingress gate bailed loud. The multi-root spelling sg rc8s (`q µµµ$A`)
/// must stay fail-closed loud.
#[test]
fn f100_no_expando_literal_search_answers_sg_set() {
    let session = isolated_index_session();
    session.write("t.js", F100_GLU_JS);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    for (pattern, want) in [
        ("\u{b5}\u{b5}\u{b5}$A + 1", vec![4u32]),
        ("\u{b5}\u{b5}$A + 1", vec![5]),
        ("\u{b5}\u{b5}\u{b5}$A", vec![4]),
        ("$A\u{b5}\u{b5}B", vec![6]),
        ("$A$$B", vec![10]),
        ("foo\u{b5}\u{b5}\u{b5}$A", vec![7]),
        ("foo$$$A", vec![11]),
    ] {
        let hits = f96_hits(&session, pattern);
        assert_eq!(
            hits.iter().map(|(_, l)| *l).collect::<Vec<u32>>(),
            want,
            "js {pattern:?}: sg answers the whole-token literal row (m1 oracle \
             set) — never a silent empty or ingress loud"
        );
    }
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    match searcher.search("pattern:q \u{b5}\u{b5}\u{b5}$A") {
        Ok(response) => panic!(
            "js q µµµ$A: sg 0.45.2 rc8 (multi-root parse, m1) — must stay \
             fail-closed loud, answered {:?}",
            response.hits
        ),
        Err(err) => assert!(
            format!("{err:#}").contains("fail-closed"),
            "js q µµµ$A must stay loud exactly like sg's rc8: {err:#}"
        ),
    }
}

/// RED (FB-99B-1, search ingress): ruby folds the `?` method-call suffix
/// INTO the identifier node, so sg reads `µA?` as a whole-node LITERAL and
/// answers the verbatim rows {1,5,6} under both spellings. The `!` twin is
/// NOT a suffix in expression position (sg rc8s) and must stay loud.
#[test]
fn f100_ruby_suffix_search_answers_sg_set() {
    let session = isolated_index_session();
    session.write("t.rb", F100_RUBY_SRC);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    for (pattern, want) in [
        ("\u{b5}A?", vec![1u32, 5, 6]),
        ("$A?", vec![1, 5, 6]),
    ] {
        let hits = f96_hits(&session, pattern);
        assert_eq!(
            hits.iter().map(|(_, l)| *l).collect::<Vec<u32>>(),
            want,
            "ruby {pattern:?}: sg answers the `?`-suffix literal rows (m2 \
             oracle set) under both spellings — never a silent empty"
        );
    }
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    match searcher.search("pattern:\u{b5}A!") {
        Ok(response) => panic!(
            "ruby µA!: sg 0.45.2 rc8 (parse refuses, m2) — must stay loud, \
             answered {:?}",
            response.hits
        ),
        Err(err) => assert!(
            format!("{err:#}").contains("fail-closed"),
            "ruby µA! must stay loud exactly like sg's rc8: {err:#}"
        ),
    }
}

// ---------------------------------------------------------------------------
// PASS 102 (r52) — F-101A-1..4 search-surface pins. Oracle sets:
// artifacts/conformance/pass102/matrix/*.jsonl (ast-grep 0.45.2, probed
// 2026-09-08, --limit pinned, faithful-shape fixtures).
// ---------------------------------------------------------------------------

/// RED (F-101A-1, search ingress): the call-argument glued row
/// `q(µµµ$A)` (py) is answered by sg 0.45.2 for `q(µA_)` — the argument
/// list's candidate ERROR child rides as an extra and the meta binds the
/// identifier fragment (m1 oracle: hit text `q(µµµ$A)`, A_ = "µµµ").
/// Pre-fix the search answered a silent `ok:true []`.
#[test]
fn f102_call_argument_fragment_search_answers_sg_set() {
    let session = isolated_index_session();
    session.write("t.py", "q(\u{b5}\u{b5}\u{b5}$A)\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let hits = f96_hits(&session, "q(\u{b5}A_)");
    assert_eq!(
        hits,
        vec![("t.py".to_string(), 1u32)],
        "py q(µA_) on `q(µµµ$A)`: sg answers the row with the fragment \
         bound (m1 oracle) — never a silent under-answer"
    );
}

/// RED (F-101A-3, search ingress): `q(µA_)` must answer ONLY the
/// parenthesized ruby row. sg 0.45.2 answers [] on the paren-less command
/// call `q µµµ$A` (m3 oracle — the pattern's `(` token never aligns) while
/// the subject's meta binding over-answered both rows.
#[test]
fn f102_ruby_paren_call_gate_search_answers_sg_set() {
    let session = isolated_index_session();
    session.write("cmd.rb", "q \u{b5}\u{b5}\u{b5}$A\n");
    session.write("par.rb", "q(\u{b5}\u{b5}\u{b5}$A)\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let mut hits = f96_hits(&session, "q(\u{b5}A_)");
    hits.sort();
    assert_eq!(
        hits,
        vec![("par.rb".to_string(), 1u32)],
        "ruby q(µA_): sg answers ONLY the parenthesized row (m3 oracle) — \
         the paren-less command call is never a binding site"
    );
}

/// RED (F-101A-2, search ingress): multi-suffix ruby spellings
/// (`µA??`/`$A??`) are sg-rc8 patterns (m2 oracle: "Multiple AST nodes are
/// detected") that literal-answered the matching row. The search must fold
/// the class to the same loud fail-closed posture the registered `µA!`
/// twin already holds — for BOTH spellings (µ≡$ convergence).
#[test]
fn f102_ruby_multi_suffix_search_stays_loud() {
    let session = isolated_index_session();
    session.write("t.rb", "\u{b5}A??\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    for pattern in ["\u{b5}A??", "$A??"] {
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            limit: 8,
            ..session.search_options()
        });
        match searcher.search(&format!("pattern:{pattern}")) {
            Ok(response) => panic!(
                "ruby {pattern:?}: sg 0.45.2 rc8 (m2 oracle) — must fold to \
                 the loud class, answered {:?}",
                response.hits
            ),
            Err(err) => assert!(
                format!("{err:#}").contains("fail-closed"),
                "ruby {pattern:?} must fold to sg's rc8-loud class: {err:#}"
            ),
        }
    }
}

/// RED (F-101A-4, search ingress): the comment-carrying whole-token literal
/// call `q(µAble) // c` is answered by sg 0.45.2 on the identical js row
/// (m4 oracle: hit text `q(µAble) // c`) — the comment is transparent in
/// the pattern parse. The census rc2'd the whole meta-free comment class.
/// Go control: sg rc8s the same spelling there, so the loud class must
/// hold per language.
#[test]
fn f102_literal_trailing_comment_search_answers_sg_set() {
    let session = isolated_index_session();
    session.write("t.js", "q(\u{b5}Able) // c\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let hits = f96_hits(&session, "q(\u{b5}Able) // c");
    assert_eq!(
        hits,
        vec![("t.js".to_string(), 1u32)],
        "js q(µAble) // c: sg answers the identical row (m4 oracle) — never \
         a fail-closed rc2"
    );

    let go = isolated_index_session();
    go.write("t.go", "package main\nfunc f() { q(Able) }\n");
    go.index_all(IndexOptions {
        embed_semantic: false,
        ..go.index_options()
    });
    let searcher = go.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..go.search_options()
    });
    match searcher.search("pattern:q(Able) // c") {
        Ok(response) => panic!(
            "go q(Able) // c: sg 0.45.2 rc8s the spelling (m4 oracle) — the \
             loud class must hold per language, answered {:?}",
            response.hits
        ),
        Err(err) => assert!(
            format!("{err:#}").contains("fail-closed"),
            "go comment-carrying literal face must stay loud like sg's rc8: {err:#}"
        ),
    }
}

// ---------------------------------------------------------------------------
// PASS 105 (r55 remediation) — FB-104B-1 / FB-104A-1 / FB-104A-2 / FB-104A-3 /
// FB-104A-4. Probe matrices: artifacts/conformance/pass105/matrix/m{1..4}.jsonl
// (subject f85c58e7fc0bb604 pre-fix, oracle vendored ast-grep 0.45.2, both
// subject service lanes, --limit 1000 pinned).
// ---------------------------------------------------------------------------

/// RED (FB-104B-1, r44 94a run1 cell 94 + 104B live repro): a php member-call
/// pattern whose comment sits in the ARGUMENT slot
/// (`$obj->m(/* m */ $A)`) classifies NeverMatches, the php inline placement
/// is sg-accepted, and matcher_decides hands the file to a walk arm whose
/// member-call slot grammar refuses the comment trivia — every file answers
/// structurally empty and the face composed into a SILENT ok:true [] where
/// sg 0.45.2 answers the row. The walk cannot decide this face, so it must
/// take the loud census path (rc2 fail-closed), never a silent empty. The
/// no-comment control keeps answering (index-served, hot path untouched);
/// the f87a php hook family (assignment/operand faces) keeps its
/// walk-answered registered contract (pinned separately below).
#[test]
fn f105_php_comment_slot_member_call_takes_loud_census() {
    let session = isolated_index_session();
    session.write(
        "t.php",
        "<?php\n$r = $obj->m(/* m */ 5);\n$s = $obj->m(6);\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    // The silent-miss face: sg 0.45.2 answers t.php:2 (oracle, m1 matrix).
    // The subject cannot answer it natively (the slot grammar refuses the
    // comment), so the census must fail closed LOUD — the RED posture was a
    // silent Ok(empty).
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    match searcher.search("pattern:$obj->m(/* m */ $A)") {
        Ok(response) => panic!(
            "php $obj->m(/* m */ $A): the walk arm cannot answer comment-carrying \
             member-call slots (94a cell 94) — must take the loud census path, \
             got silent {:?}",
            response.hits
        ),
        Err(err) => assert!(
            format!("{err:#}").contains("fail-closed"),
            "php comment-slot face must keep the fail-closed class: {err:#}"
        ),
    }
    // No-comment control: index-served and answered exactly like sg.
    let hits = f96_hits(&session, "$obj->m($A)");
    let mut lines: Vec<u32> = hits.iter().map(|(_, line)| *line).collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        vec![2u32, 3],
        "php $obj->m($A) control must keep answering both rows: {:?}",
        hits
    );
    // The f87a php assignment-hook face (comment-transparent lane) keeps its
    // walk-answered contract through the same narrowed exemption.
    let hook = isolated_index_session();
    hook.write("h.php", "<?php\n$alpha = 1 + 1;\n$a = $alpha + /* c */ 1;\n");
    hook.index_all(IndexOptions {
        embed_semantic: false,
        ..hook.index_options()
    });
    let hook_hits = f96_hits(&hook, "$a = $alpha + /* c */ 1;");
    assert_eq!(
        hook_hits,
        vec![("h.php".to_string(), 3u32)],
        "f87a php assignment-hook comment face must keep answering (sg line 3): {:?}",
        hook_hits
    );
}

/// RED (FB-104A-2): rest-slot admission for PLAIN-call languages'
/// non-leading positions. `q($A, $$$B)` / `q($B, $$$A)` rc2'd while sg
/// 0.45.2 answers the 2- and 3-arg rows; `q($$$A, $B)` answers ONLY the
/// 1-arg row (sg binds the mid/leading rest to ZERO arguments and anchors
/// the singles at the end); `q($$$B, $$$A)` answers every arity. The grid
/// is uniform across js/py/rb/php/go/rs/ts (m3 matrix, 189 cells).
#[test]
fn f105_rest_position_call_patterns_answer_sg_grid() {
    let session = isolated_index_session();
    session.write("a1.js", "q(1);\n");
    session.write("a2.js", "q(1, 2);\n");
    session.write("a3.js", "q(1, 2, 3);\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let sorted = |hits: Vec<(String, u32)>| {
        let mut files: Vec<String> = hits.into_iter().map(|(f, _)| f).collect();
        files.sort();
        files
    };
    assert_eq!(
        sorted(f96_hits(&session, "q($A, $$$B)")),
        vec!["a2.js", "a3.js"],
        "trailing rest: sg answers the 2- and 3-arg rows only (m3 oracle)"
    );
    assert_eq!(
        sorted(f96_hits(&session, "q($$$A, $B)")),
        vec!["a1.js"],
        "leading rest + single: sg answers ONLY the 1-arg row (rest binds zero)"
    );
    assert_eq!(
        sorted(f96_hits(&session, "q($A, $$$B, $C)")),
        vec!["a2.js"],
        "mid rest: sg answers ONLY the exact-arity row (rest binds zero)"
    );
    assert_eq!(
        sorted(f96_hits(&session, "q($$$B, $$$A)")),
        vec!["a1.js", "a2.js", "a3.js"],
        "two rests: sg answers every arity (head/tail split)"
    );
    assert_eq!(
        sorted(f96_hits(&session, "q($$$A)")),
        vec!["a1.js", "a2.js", "a3.js"],
        "whole-list rest control: every arity (registered sg-exact face)"
    );
    assert_eq!(
        sorted(f96_hits(&session, "q($A, $B)")),
        vec!["a2.js"],
        "exact-arity control unchanged"
    );
}

/// RED (FB-104A-4 / R-102-5 fold): the bare-rest pattern `q($$$A)` was
/// INDEX-served (`call:q` signature rows cannot express the paren-token
/// contract), so the r52 paren gate never ran and the over-answer spread to
/// swift brace calls + ruby brace/do/symbol/string/receiver rows (m4: 7
/// fail-open row-faces x both lanes). The pattern must be walk-served, where
/// the paren gate refuses every paren-less candidate exactly like sg.
#[test]
fn f105_bare_rest_pattern_walk_refuses_parenless_calls() {
    let session = isolated_index_session();
    session.write("cmd.rb", "q x\n");
    session.write("brace.rb", "q { |a| a }\n");
    session.write("dobl.rb", "q do |a|\n  a\nend\n");
    session.write("sym.rb", "q :x\n");
    session.write("str.rb", "q \"s\"\n");
    session.write("recv.rb", "obj.w x\n");
    session.write("paren.rb", "q(1)\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    assert_eq!(
        f96_hits(&session, "q($$$A)"),
        vec![("paren.rb".to_string(), 1u32)],
        "q($$$A): sg answers ONLY the parenthesized row (m4 oracle) — the \
         index lane must never serve the paren-less over-answer"
    );
    assert_eq!(
        f96_hits(&session, "obj.w($$$A)"),
        Vec::<(String, u32)>::new(),
        "obj.w($$$A): the ruby receiver-dot paren-less row must stay empty (m4)"
    );
    assert_eq!(
        f96_hits(&session, "q($A)"),
        vec![("paren.rb".to_string(), 1u32)],
        "single-capture control unchanged"
    );
}

/// RED (FB-104A-1 + FB-104A-3): sg 0.45.2's swift grammar spells
/// `LHS + q(...)` as ONE call whose CALLEE is the additive compound — the
/// inner `q(...)` is never a standalone call node, so clean call patterns
/// answer [] on every arithmetic-binary row (m2: int/float/string LHS x
/// +,-,*,/ — including clean rows like `1 + q(1)`). The subject's walk
/// over-answered the recovered nested call (80 fail-open cells). Aligned
/// binary patterns DO answer (the compound callee aligns token-exactly):
/// `1 + q($A)` on literal-LHS rows, and — FB-104A-3 — `x + q($A)` on
/// ident-LHS rows (the general lane refused the bare-ident head outright,
/// rc2 where sg answers). python control: normal binary parse — the clean
/// call pattern MUST keep answering there.
#[test]
fn f105_swift_binary_call_faces_answer_sg_parse_class() {
    let session = isolated_index_session();
    session.write("lit.swift", "1 + q(\u{b5}\u{b5}\u{b5}$A)\n");
    session.write("ident.swift", "x + q(\u{b5}\u{b5}\u{b5}$A)\n");
    session.write("litclean.swift", "1 + q(1)\n");
    session.write("pyrow.py", "1 + q(\u{b5}\u{b5}\u{b5}$A)\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let sorted = |hits: Vec<(String, u32)>| {
        let mut files: Vec<String> = hits.into_iter().map(|(f, _)| f).collect();
        files.sort();
        files
    };
    assert_eq!(
        sorted(f96_hits(&session, "q($A)")),
        vec!["pyrow.py"],
        "q($A): sg answers ONLY the python row — swift arithmetic-binary \
          compounds never present a matchable inner call (m2 oracle)"
    );
    assert_eq!(
        sorted(f96_hits(&session, "1 + q($A)")),
        vec!["lit.swift", "litclean.swift", "pyrow.py"],
        "1 + q($A): sg answers the LHS-aligned compound-callee faces (m2 oracle)"
    );
    assert_eq!(
        sorted(f96_hits(&session, "x + q($A)")),
        vec!["ident.swift"],
        "x + q($A): sg answers the ident-LHS aligned face — the bare-ident \
         binary pattern must be served, not rc2 (FB-104A-3, m2 oracle)"
    );
}

/// RED (FB-106A-1 / FB-106B-1): the same-name two-rest plain-call face
/// `q($$$A, $$$A)` over-answered every >= 2-argument row on BOTH service
/// lanes — the pass-105 slot admission covered the Meta-vs-Rest collision
/// only, and the two-rest match arm bound BOTH rests to the identical
/// whole-args text. The registered sg truth (m1_samespace oracle, 11
/// languages): sg answers ONLY the 1-arg rows. The collision face must keep
/// its pre-r55 classifier-REJECTED loud class end-to-end (the hconf036
/// rest+single contract), never an over-answering hit set.
#[test]
fn f107_same_name_two_rest_collision_keeps_loud_class() {
    let session = isolated_index_session();
    session.write("a1.js", "q(1);\n");
    session.write("a2.js", "q(1, 2);\n");
    session.write("a3.js", "q(1, 2, 3);\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    // The face itself: loud fail-closed, never the over-answer set.
    match searcher.search("pattern:q($$$A, $$$A)") {
        Ok(response) => panic!(
            "q($$$A, $$$A): the same-name two-rest collision must keep the \
             registered loud class (sg answers only the 1-arg rows — \
             m1_samespace), got silent {:?}",
            response.hits
        ),
        Err(err) => assert!(
            format!("{err:#}").contains("fail-closed"),
            "rejection must state fail-closed: {err:#}"
        ),
    }
    // Distinct-name control keeps the classified slot lane: sg answers every
    // non-empty arity (m3 oracle).
    let sorted = |hits: Vec<(String, u32)>| {
        let mut files: Vec<String> = hits.into_iter().map(|(f, _)| f).collect();
        files.sort();
        files
    };
    assert_eq!(
        sorted(f96_hits(&session, "q($$$A, $$$B)")),
        vec!["a1.js", "a2.js", "a3.js"],
        "q($$$A, $$$B): distinct-name two rests keep answering every arity"
    );
    assert_eq!(
        sorted(f96_hits(&session, "q($$$A)")),
        vec!["a1.js", "a2.js", "a3.js"],
        "q($$$A): sole-rest control unchanged"
    );
}

/// RED (FB-106A-3): swift prefix-unary compound callees (`-q(1)`, `!q(1)`,
/// `&q(1)`) fold into ONE call whose callee child is a `prefix_expression` —
/// the r55 compound-callee refusal was scoped to additive/multiplicative, so
/// the walk over-answered those rows where sg 0.45.2 answers [] (m2
/// _swiftprefix: 19 over cells). `try q(1)` keeps a REAL inner call node and
/// must keep answering, exactly like sg; the python unary control keeps its
/// normal parse.
#[test]
fn f107_swift_prefix_compound_faces_answer_sg_parse_class() {
    let session = isolated_index_session();
    session.write("minus.swift", "let r = -q(1)\n");
    session.write("not.swift", "let r = !q(1)\n");
    session.write("amp.swift", "let r = &q(1)\n");
    session.write("try.swift", "let r = try q(1)\n");
    session.write("plain.swift", "let r = q(9)\n");
    session.write("pyrow.py", "y = q(7)\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let sorted = |hits: Vec<(String, u32)>| {
        let mut files: Vec<String> = hits.into_iter().map(|(f, _)| f).collect();
        files.sort();
        files
    };
    assert_eq!(
        sorted(f96_hits(&session, "q($A)")),
        vec!["plain.swift", "pyrow.py", "try.swift"],
        "q($A): sg answers only the real-call rows — swift prefix-compound \
         callees never present a matchable inner call (m2_swiftprefix oracle)"
    );
}

/// RED (FB-106A-4): swift member-chain PATTERNS silently answered rc0
/// ok:true [] on both lanes where sg 0.45.2 answers the aligned rows
/// (`a.b($A)` on `a.b(1)` — the FB-104B-1 silent class). Root cause: the
/// `navigation_suffix` wrapper kind is absent from the faithful member-kind
/// resolution, so every chain candidate was capture-vetoed. sg is connector
/// token-exact: a `.` template never answers a `?.` site and mismatches
/// stay empty.
#[test]
fn f107_swift_member_chain_pattern_answers_sg_aligned() {
    let session = isolated_index_session();
    session.write("chain.swift", "let r = a.b(1)\n");
    session.write("deep.swift", "let r = a.b.c(1)\n");
    session.write("opt.swift", "let r = a?.b(1)\n");
    session.write("pychain.py", "r = a.b(1)\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let sorted = |hits: Vec<(String, u32)>| {
        let mut files: Vec<String> = hits.into_iter().map(|(f, _)| f).collect();
        files.sort();
        files
    };
    assert_eq!(
        sorted(f96_hits(&session, "a.b($A)")),
        vec!["chain.swift", "pychain.py"],
        "a.b($A): sg answers the aligned 2-segment chains (m3_chains oracle) — \
         never the ?. site and never the 3-chain (token/length exact)"
    );
    assert_eq!(
        sorted(f96_hits(&session, "a.b.c($A)")),
        vec!["deep.swift"],
        "a.b.c($A): sg answers the aligned 3-chain row (m3_chains oracle)"
    );
}

/// PASS 122 (F3, f122c): keyword-literal roots (`true`/`false` rust, js
/// `null`, py `None`/`True`) are admitted by the ident signature
/// (`is_pattern_ident`) but have NO `pattern_nodes` rows — the ident-exact
/// index serve composed a silent `ok:true []` where sg 0.45.2 answers the
/// keyword leaf node (oracle grid /tmp/phase122/f3). The core ingress
/// routes the class to the native walk via the
/// `pattern_is_keyword_literal_root` guard. Mutant kill: removing the
/// guard re-serves the silent empty and this test fails.
#[test]
fn f122c_keyword_literal_roots_answer_via_native_walk() {
    let session = indexed_rs(
        "fn alpha(x: bool) {\n    if x { beta(true); }\n}\nfn beta(v: bool) {}\n",
    );
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    let hits = searcher
        .search("pattern:true")
        .expect("keyword literal must answer via the walk, not fail closed");
    assert_eq!(hits.hits.len(), 1, "sg answers the leaf node: {:?}", hits.hits);
    assert_eq!(hits.hits[0].line_start, 2, "{:?}", hits.hits[0]);
}

/// PASS 131 (130A-F4): the native-lane same-line dedup key must carry the
/// hit's byte span. Two INDEPENDENT calls on one line are two sg hits
/// (sg n2, oracle /tmp/phase131/live/c_finding2); the old
/// (file, line_start, line_end) key collapsed them to one hit at the
/// native-lane union site. The SAME node found by two overlapping query
/// arms must still dedup to one — the span distinguishes the two cases.
#[test]
fn f131_same_line_independent_calls_answer_per_node() {
    let session = isolated_index_session();
    session.write("mod.js", "q(1); q(2);\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let resp = searcher.search("pattern:q($A)").expect("q($A) answers");
    assert_eq!(resp.hits.len(), 2, "{:?}", resp.hits);
    assert_eq!(resp.hits[0].line_start, 1, "{:?}", resp.hits);
    assert_eq!(resp.hits[1].line_start, 1, "{:?}", resp.hits);
}

/// RED (F-131E-2): the BARE `debugger` spelling through the CLI `--pattern`
/// ingress (`Searcher::search("pattern:debugger")` — the exact
/// `run_multi_pattern_search` token) answered a silent `ok:true []` where
/// sg 0.45.2 answers the js/ts debugger_statement n1 (oracle
/// /tmp/phase131E/fixfaces dbg_{js,ts}_bare; 132 grid sg=1 subject=0 on
/// both grammars). Root cause: `cached_pattern_signatures` admits the bare
/// keyword as an "ident-exact" signature and `index_can_serve_pattern`
/// early-returns the (always empty) ident rows — `pattern_nodes` stores no
/// rows for keyword statements — because `debugger` was missing from the
/// F62-2 STATEMENT_KEYWORDS escape that routes `break`/`continue`/
/// `throw`/… to the native walk, where the f131d debugger_statement kinds
/// arm answers sg-exactly. The `;`-ful spelling (not an ident) already
/// rides the walk and must keep answering.
#[test]
fn f132a_bare_debugger_cli_pattern_answers_debugger_statement() {
    let session = isolated_index_session();
    session.write("dbg.js", "function f() { debugger; }\n");
    session.write("dbg.ts", "function g() { debugger; }\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let bare = searcher
        .search("pattern:debugger")
        .expect("bare debugger answers via the native walk, never a silent empty");
    let mut files: Vec<&str> = bare.hits.iter().map(|h| h.file.as_str()).collect();
    files.sort_unstable();
    assert_eq!(
        files,
        vec!["dbg.js", "dbg.ts"],
        "sg answers the debugger_statement n1 per js/ts file: {:?}",
        bare.hits
    );
    assert!(bare.hits.iter().all(|h| h.line_start == 1), "{:?}", bare.hits);
    let semi = searcher
        .search("pattern:debugger;")
        .expect("debugger; answers");
    assert_eq!(
        semi.hits.len(),
        2,
        "the `;`-ful spelling must keep answering both files: {:?}",
        semi.hits
    );
}

/// RED (F-131E-1, phase-132 correction): `SearchHit::byte_span` exists for
/// the fusion `DedupKey` (same-node vs independent-same-line collapse,
/// 130A-F4) and is routing-internal — the golden T1-SEARCH-PATTERN-ENVELOPE
/// froze the byte-exact `search --json` stdout, and the 131 additive wire
/// field (`"byte_span": [113,122]`, byte-equal to sg's byteOffset) flipped
/// it PASS→FAIL. The pass-131 claim "serde-skipped, wire bytes unchanged"
/// was wrong: the attribute was the CONDITIONAL
/// `skip_serializing_if = "Option::is_none"`, so every native-walk hit
/// (which carries `Some(span)`) serialized it. The wire contract is: the
/// span never serializes; the deserializer (`SearchHitWire`) never reads it.
/// PASS 133 per-file hit census shared by the f133a/b/c bare-keyword grid
/// pins (oracle sg 0.45.2, 52-cell grid /tmp/phase133/live/grid.json).
fn f133_per_file(hits: &[ast_sgrep_core::SearchHit]) -> std::collections::BTreeMap<String, usize> {
    let mut out = std::collections::BTreeMap::new();
    for hit in hits {
        *out.entry(hit.file.clone()).or_insert(0) += 1;
    }
    out
}

/// RED (F-132E class sweep, pass 133): the BARE `return` spelling answered
/// only the childless form through the general lane's childless-template
/// face while sg 0.45.2 answers EVERY statement of the family (grid
/// js/ts:return sg n3 vs subject n1; rs n2 vs 1; go n3 vs 2; rb n2 vs 1 —
/// oracle /tmp/phase133/live/grid.json). The bare spelling is ident-admitted
/// and escaped to the walk (F62-2), but `match_bare_statement_kind` had no
/// `return` arm, so the arg-ful forms dropped. Same silence for ruby
/// `break`: the PASS 65 arm listed only `break_statement`/
/// `break_expression`, neither of which exists in tree-sitter-ruby (ruby
/// names the node `break`), so the arm silenced the sg-answered face
/// (grid rb:break sg n1 subject n0) instead of falling through to the
/// literal lane like arm-less `next` did. Java `return` must keep
/// answering n2 through its existing lane (regression guard).
#[test]
fn f133a_bare_return_family_and_rb_break_answer_sg_aligned() {
    let session = isolated_index_session();
    session.write(
        "ret.js",
        "function f() {\n  return;\n}\nfunction g() {\n  return 1;\n}\n",
    );
    session.write(
        "ret.ts",
        "function f() {\n  return;\n}\nfunction g() {\n  return 1;\n}\n",
    );
    session.write(
        "ret.rs",
        "fn f() {\n    return;\n}\nfn g() -> i32 {\n    return 1;\n}\n",
    );
    session.write(
        "ret.go",
        "package main\n\nfunc f() {\n\treturn\n}\n\nfunc g() int {\n\treturn 1\n}\n",
    );
    session.write("ret.rb", "def f\n  return\nend\n\ndef g\n  return 1\nend\n");
    session.write(
        "ret.java",
        "class K {\n  int g() {\n    return 1;\n  }\n  void f() {\n    return;\n  }\n}\n",
    );
    session.write("brk.rb", "x = 5\nwhile x > 0\n  break\nend\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 64,
        ..session.search_options()
    });
    let ret = searcher
        .search("pattern:return")
        .expect("bare return answers via the native walk, never a silent under-answer");
    let per = f133_per_file(&ret.hits);
    assert_eq!(
        per.get("ret.js"),
        Some(&2),
        "sg answers both js return_statements (grid js:return sg n3): {per:?}"
    );
    assert_eq!(
        per.get("ret.ts"),
        Some(&2),
        "sg answers both ts return_statements (grid ts:return sg n3): {per:?}"
    );
    assert_eq!(
        per.get("ret.rs"),
        Some(&2),
        "rust return_expression is the sg row (grid rs:return sg n2): {per:?}"
    );
    assert_eq!(
        per.get("ret.go"),
        Some(&2),
        "go return_statement family incl. `return 1` (grid go:return sg n3): {per:?}"
    );
    assert_eq!(
        per.get("ret.rb"),
        Some(&2),
        "ruby `return`/`return 1` are both kind `return` (grid rb:return sg n2): {per:?}"
    );
    assert_eq!(
        per.get("ret.java"),
        Some(&2),
        "java return keeps answering through its existing lane: {per:?}"
    );
    let mut js_lines: Vec<u32> = ret
        .hits
        .iter()
        .filter(|h| h.file == "ret.js")
        .map(|h| h.line_start)
        .collect();
    js_lines.sort_unstable();
    assert_eq!(js_lines, vec![2, 5], "{:?}", ret.hits);
    let brk = searcher
        .search("pattern:break")
        .expect("bare break answers");
    let per = f133_per_file(&brk.hits);
    assert_eq!(
        per.get("brk.rb"),
        Some(&1),
        "ruby names the node `break`; the kind arm must not silence it (grid rb:break sg n1): {per:?}"
    );
    assert_eq!(brk.hits.len(), 1, "{:?}", brk.hits);
    assert_eq!(brk.hits[0].line_start, 3, "{:?}", brk.hits);
}

/// RED (F-132E class sweep, pass 133): bare `import` (js/ts/py), `use`
/// (rust) and the python simple-statement keywords `pass`/`global`/`del`/
/// `assert` were ident-admitted with NO `pattern_nodes` rows — the
/// ident-exact index early-return composed silent `ok:true []` where sg
/// 0.45.2 answers the statement family (grid /tmp/phase133/live/grid.json:
/// js/ts/py:import, rs:use, py:pass/global/del/assert all sg n1, py:assert
/// n2, subject n0 on every face). Fix shape = F62-2 escape; the walk's
/// literal lane answers each face at the keyword-token leaf (byte-equal to
/// sg's row) — no kinds arm needed (mutant M-133c: the arms were
/// observationally redundant, deleted).
#[test]
fn f133b_import_use_and_python_simple_statements_answer_sg_aligned() {
    let session = isolated_index_session();
    session.write("imp.js", "import fs from \"fs\";\n");
    session.write("imp.ts", "import fs from \"fs\";\n");
    session.write("imp.py", "import os\n");
    session.write("use.rs", "use std::fmt;\n");
    session.write(
        "pyfive.py",
        "def f():\n    pass\n\ndef i():\n    global v\n\ndef j():\n    del d[k]\n\ndef a():\n    assert y\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 64,
        ..session.search_options()
    });
    let imp = searcher
        .search("pattern:import")
        .expect("bare import answers via the native walk");
    let per = f133_per_file(&imp.hits);
    assert_eq!(per.get("imp.js"), Some(&1), "grid js:import sg n1: {per:?}");
    assert_eq!(per.get("imp.ts"), Some(&1), "grid ts:import sg n1: {per:?}");
    assert_eq!(per.get("imp.py"), Some(&1), "grid py:import sg n1: {per:?}");
    assert_eq!(imp.hits.len(), 3, "no import hits outside the three grammars: {:?}", imp.hits);
    let use_hits = searcher
        .search("pattern:use")
        .expect("bare use answers via the native walk");
    assert_eq!(
        f133_per_file(&use_hits.hits).get("use.rs"),
        Some(&1),
        "grid rs:use sg n1: {:?}",
        use_hits.hits
    );
    assert_eq!(use_hits.hits.len(), 1, "{:?}", use_hits.hits);
    for (pattern, why) in [
        ("pattern:pass", "grid py:pass sg n1"),
        ("pattern:global", "grid py:global sg n1"),
        ("pattern:del", "grid py:del sg n1"),
        ("pattern:assert", "grid py:assert sg n2 family"),
    ] {
        let resp = searcher
            .search(pattern)
            .unwrap_or_else(|e| panic!("{pattern} must answer via the native walk: {e}"));
        assert_eq!(
            resp.hits.len(),
            1,
            "{pattern} ({why}): {:?}",
            resp.hits
        );
        assert_eq!(resp.hits[0].file, "pyfive.py", "{pattern}: {:?}", resp.hits);
    }
}

/// RED (F-132E class sweep, pass 133): bare go `fallthrough`/`goto` and
/// ruby `redo`/`retry` — ident-admitted keyword spellings with no
/// `pattern_nodes` rows and no walk arm; the ident-exact early-return
/// answered silent empty where sg 0.45.2 answers the statement (grid
/// /tmp/phase133/live/grid.json: all four faces sg n1, subject n0).
#[test]
fn f133c_go_and_ruby_tail_keywords_answer_sg_aligned() {
    let session = isolated_index_session();
    session.write(
        "gokw.go",
        "package main\n\nfunc h() {\n\tfor {\n\t\tbreak\n\t}\n\tswitch {\n\tcase true:\n\t\tfallthrough\n\tcase false:\n\t\tgoto done\n\t}\ndone:\n\treturn\n}\n",
    );
    session.write(
        "rbkw.rb",
        "loop do\n  redo\nend\n\nbegin\n  raise \"e\"\nrescue\n  retry\nend\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 64,
        ..session.search_options()
    });
    for (pattern, file, why) in [
        ("pattern:fallthrough", "gokw.go", "grid go:fallthrough sg n1"),
        ("pattern:goto", "gokw.go", "grid go:goto sg n1"),
        ("pattern:redo", "rbkw.rb", "grid rb:redo sg n1"),
        ("pattern:retry", "rbkw.rb", "grid rb:retry sg n1"),
    ] {
        let resp = searcher
            .search(pattern)
            .unwrap_or_else(|e| panic!("{pattern} must answer via the native walk: {e}"));
        assert_eq!(resp.hits.len(), 1, "{pattern} ({why}): {:?}", resp.hits);
        assert_eq!(
            resp.hits[0].file, file,
            "{pattern} ({why}): {:?}",
            resp.hits
        );
    }
}

#[test]
fn f132b_search_hit_wire_omits_dedup_byte_span() {
    let hit = ast_sgrep_core::SearchHit::span(ast_sgrep_core::search::SpanHitInput {
        kind: ast_sgrep_core::HitKind::Pattern,
        file: "calc.rs".into(),
        line_start: 10,
        line_end: 10,
        score: 1.0,
        excerpt: "add(1, 2)".into(),
        symbol: None,
        language: Some("rust".into()),
        byte_span: Some((113, 122)),
    });
    let wire = serde_json::to_string(&hit).expect("SearchHit serializes");
    assert!(
        !wire.contains("byte_span"),
        "dedup byte_span leaked to the search --json wire: {wire}"
    );
}

/// f135a (134B-F1/F2, routing seam): the bare go `defer`/`go` spellings are
/// ident-admitted with no `pattern_nodes` rows, so `index_can_serve_pattern`
/// early-returned the empty ident rows as complete — silent `ok:true []`
/// where sg 0.45.2 answers every defer/go statement (grids G_go_defer /
/// G_go_go: sg n2 vs subject n0, oracle /tmp/phase135/cells). The bare ts
/// `await` is escaped but arm-less: the STATEMENT_HEAD general lane's
/// childless-template face answers only the operand-less form, so the
/// operand-ful await rows dropped (grid X_ts_await_bare_src: sg n2 vs
/// subject n0). js `await` stays BOTH-empty like sg (grid G_js_await: sg
/// rc1 valid empty) — the js arm must not exist. Ident faces of the same
/// spellings keep their literal-lane answers (grids Z_js_go_ident /
/// Z_rs_defer_ident: sg n2 == subject n2 — the pass-132 zero-drift
/// doctrine).
#[test]
fn f135a_go_defer_go_and_ts_await_bare_spellings_answer_sg_aligned() {
    let session = isolated_index_session();
    session.write(
        "dfr.go",
        "package main\n\nfunc main() {\n\tdefer f()\n\tdefer g()\n}\n\nfunc f() {}\n\nfunc g() {}\n",
    );
    session.write(
        "gow.go",
        "package main\n\nfunc main() {\n\tgo job()\n\tgo other()\n}\n\nfunc job() {}\n\nfunc other() {}\n",
    );
    session.write(
        "awt.ts",
        "async function f() {\n  await g();\n  const x = await h();\n}\nasync function k() {\n  await m();\n}\n",
    );
    session.write("awj.js", "async function f() {\n  await g();\n}\n");
    session.write("goident.js", "function go() {}\nlet x = go();\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 64,
        ..session.search_options()
    });
    let defer = searcher
        .search("pattern:defer")
        .expect("bare defer answers via the walk, never a silent empty");
    assert_eq!(
        f135_per_file(&defer.hits).get("dfr.go"),
        Some(&2),
        "sg answers every defer_statement n2: {:?}",
        defer.hits
    );
    let go = searcher
        .search("pattern:go")
        .expect("bare go answers via the walk");
    assert_eq!(
        f135_per_file(&go.hits).get("gow.go"),
        Some(&2),
        "sg answers every go_statement n2: {:?}",
        go.hits
    );
    // Ident faces of `go` in js keep the literal-lane answer (n2, no drift).
    assert_eq!(
        f135_per_file(&go.hits).get("goident.js"),
        Some(&2),
        "the go escape must not drift the js ident faces: {:?}",
        go.hits
    );
    let await_ts = searcher
        .search("pattern:await")
        .expect("bare ts await answers via the await_expression arm");
    assert_eq!(
        f135_per_file(&await_ts.hits).get("awt.ts"),
        Some(&3),
        "sg answers every await_expression n3 on the ts fixture: {:?}",
        await_ts.hits
    );
    assert_eq!(
        f135_per_file(&await_ts.hits).get("awj.js"),
        None,
        "js bare await stays valid-empty like sg (rc1): {:?}",
        await_ts.hits
    );
}

fn f135_per_file(
    hits: &[ast_sgrep_core::SearchHit],
) -> std::collections::BTreeMap<String, usize> {
    let mut out = std::collections::BTreeMap::new();
    for hit in hits {
        *out.entry(hit.file.clone()).or_insert(0) += 1;
    }
    out
}

/// f135b (134A-F2, routing seam): the `;`-ful return spelling over-answered
/// operand-ful returns through the CLI-pattern ingress (grid B_js_return;:
/// sg n1 vs subject n3; B_ts_return;: sg n1 vs subject n2). The
/// empty-operand discrimination must hold end-to-end, not only at the
/// library seam.
#[test]
fn f135b_return_semi_answers_operand_less_only_end_to_end() {
    let session = isolated_index_session();
    session.write(
        "ret.js",
        "function f() {\n  return;\n}\nfunction g() {\n  return 1;\n}\n",
    );
    session.write(
        "ret.ts",
        "function f() {\n  return;\n}\nfunction g() {\n  return 1;\n}\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 64,
        ..session.search_options()
    });
    let semi = searcher
        .search("pattern:return;")
        .expect("return; answers via the native walk");
    let per_file = f135_per_file(&semi.hits);
    assert_eq!(per_file.get("ret.js"), Some(&1), "sg n1: the empty return only: {:?}", semi.hits);
    assert_eq!(per_file.get("ret.ts"), Some(&1), "sg n1: the empty return only: {:?}", semi.hits);
}

/// f135c (134A-F1/F7 ingress seam, grids A13/A1/X_py_del_two, oracle
/// re-verified 2026-09-11): the CLI-pattern ingress must not rc2 faces sg
/// 0.45.2 ANSWERS. The language-free census union
/// (`general_lane_text_eligible`) never received the csharp `lock`/`using`
/// and python `del` head admissions its per-language arm
/// (`general_lane_text_eligible_for`) carries, so `needs_ast_grep_fallback`
/// stayed true and `Searcher::search` rejected LOUD before the per-file walk
/// whose `match_pattern` answers each face sg-exactly (n1).
#[test]
fn f135c_ingress_answers_sg_answering_statement_faces() {
    let session = isolated_index_session();
    session.write(
        "u.cs",
        "class C {\n  void M() {\n    using (var d = f()) { g(); }\n  }\n}\n",
    );
    session.write(
        "l.cs",
        "class C {\n  void M() {\n    lock (o) { x(); }\n  }\n}\n",
    );
    session.write("d.py", "x = 1\ny = 2\ndel x, y\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 64,
        ..session.search_options()
    });
    let using = searcher
        .search("pattern:using ($R) { g(); }")
        .expect("sg A13 answers n1; the ingress must admit the walk");
    assert_eq!(
        f135_per_file(&using.hits).get("u.cs"),
        Some(&1),
        "sg n1 on the using meta-resource face: {:?}",
        using.hits
    );
    let lock = searcher
        .search("pattern:lock ($X) { x(); }")
        .expect("sg A1 answers n1; the ingress must admit the walk");
    assert_eq!(
        f135_per_file(&lock.hits).get("l.cs"),
        Some(&1),
        "sg n1 on the lock statement face: {:?}",
        lock.hits
    );
    let del = searcher
        .search("pattern:del $X")
        .expect("sg X_py_del_two answers n1 (X=`x, y`); the ingress must admit the walk");
    assert_eq!(
        f135_per_file(&del.hits).get("d.py"),
        Some(&1),
        "sg n1, X bound to the WHOLE operand-list text: {:?}",
        del.hits
    );
}

/// f136 (F-135E-1 + F-135E-2, routing seam): the CLI-ingress-to-walk chain
/// must END in hits for the faces sg answers. 135E probes at BOTH official
/// binaries and BOTH ingress paths (indexed db + auto-index): `del $X` on
/// `del x` / `del d[k]` answered ok:true-0 where sg 0.45.2 answers n2, and
/// `public interface $N { $B }` answered ok:true-0 where sg answers n1
/// (N/B) — the 135 admissions flipped the ingress verdict without a binding
/// arm behind it (F-135E-2's loud→silent honesty regression).
#[test]
fn f136_del_meta_and_modifier_interface_bind_end_to_end() {
    let session = isolated_index_session();
    session.write("del.py", "del x\ndel d[k]\n");
    session.write("k.cs", "public interface K {\n    void P();\n}\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 64,
        ..session.search_options()
    });
    let del = searcher
        .search("pattern:del $X")
        .expect("sg n2 on the single-operand deletes; the walk must bind them");
    assert_eq!(
        f135_per_file(&del.hits).get("del.py"),
        Some(&2),
        "sg n2 (X=`x`, X=`d[k]`): {:?}",
        del.hits
    );
    let iface = searcher
        .search("pattern:public interface $N { $B }")
        .expect("sg n1 on the modifier-matching interface root");
    assert_eq!(
        f135_per_file(&iface.hits).get("k.cs"),
        Some(&1),
        "sg n1 (N=K, B=`void P();`): {:?}",
        iface.hits
    );
    // f136c routing face: js bare `delete` is a keyword token — never a
    // `pattern_nodes` identifier row — so the ident-exact index serve
    // answered silent ok:true-0 where sg 0.45.2 answers the
    // delete_expression family n1 (the F-131E-2/F-132E escape genus).
    session.write("dv.js", "const o = {k: 1, j: 2};\ndelete o.k;\ndelete o.j;\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 64,
        ..session.search_options()
    });
    let del = searcher
        .search("pattern:delete")
        .expect("sg n2 on the delete_expression family; the escape must reach the walk");
    assert_eq!(
        f135_per_file(&del.hits).get("dv.js"),
        Some(&2),
        "sg n2 (the family rows): {:?}",
        del.hits
    );
}

/// PASS 137 (137A-F2 bare cells, routing end-to-end): the csharp
/// statement-keyword escape (signature.rs `STATEMENT_KEYWORDS`) keeps bare
/// `lock` off the ident-exact index serve — with the escape the search
/// walks natively and answers the keyword-token row sg answers (grid137
/// A-lane: cs bare `lock` sg n1, subject n0 pre-fix). M-137l (escape
/// deleted) re-traps the class in the ident-serve silence and flips this
/// test RED.
#[test]
fn f137m_csharp_bare_statement_keyword_escape_reaches_the_walk() {
    let session = isolated_index_session();
    session.write(
        "l.cs",
        "class C {\n  void M() {\n    lock (o) {\n      x();\n    }\n  }\n}\n",
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
    let bare = searcher
        .search("pattern:lock")
        .expect("bare cs `lock` answers the keyword-token row via the native walk");
    assert_eq!(
        f135_per_file(&bare.hits).get("l.cs"),
        Some(&1),
        "sg n1 on the lock_statement site: {:?}",
        bare.hits
    );
}

/// PASS 139 (139A-F5, grid139 E): the java class member-count face must
/// reach the native walk at the CLI — sg 0.45.2 binds `public abstract
/// class $N { $B }` n1 on the modifier-matching single-member candidate
/// (E01/E02 oracle receipts), and the walk answers sg-exactly (f139f), but
/// pre-fix the language-free ingress gate (`needs_ast_grep_fallback` via
/// `general_lane_supported`) rc2'd the spelling because no general template
/// can substitute a bare-meta class body — the f127a genus (CLI loud where
/// the walk answers). RED 2026-09-11: `search` returned Err (loud) on this
/// corpus. The 137A-F2 csharp statement-head admission is the registered
/// remedy shape: a language-free "some lane serves this" arm keyed on the
/// lane parse ([`classify_java_class_member_count`]); the walk + census
/// keep per-file/per-language honesty.
#[test]
fn f139_java_class_member_count_reaches_the_walk() {
    let session = isolated_index_session();
    session.write(
        "k.java",
        "public abstract class K {\n    void n();\n}\n",
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
    let hits = searcher
        .search("pattern:public abstract class $N { $B }")
        .expect("the class member-count face must answer via the walk, not ingress-rc2");
    assert_eq!(
        f135_per_file(&hits.hits).get("k.java"),
        Some(&1),
        "sg n1 on the single-member class: {:?}",
        hits.hits
    );
}

/// PASS 146 (145B-F1, true root): the required-literal byte prefilter must
/// never carry layout — `namespace A { f(); $B }` produced the literal
/// `namespace A { f` and the memmem prefilter silently dropped the
/// pretty-printed namespace body sg answers (145B grid k1, sg n1).
#[test]
fn php_pretty_printed_namespace_mixed_body_survives_the_prefilter() {
    let session = isolated_index_session();
    session.write(
        "a.php",
        "<?php\nnamespace A {\n  f( );\n  g( );\n}\n",
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
        .search("pattern:namespace A { f(); $B }")
        .expect("native pattern search");
    assert_eq!(
        response.hits.len(),
        1,
        "pretty-printed namespace body must survive the prefilter: {:?}",
        response.hits
    );
}


/// f147a (146E-F1 TRUE ROOT, ingress byte-fidelity): the `pattern:`-query
/// scope split must deliver the user's pattern BYTES to the pattern lane.
/// The pre-fix `split_whitespace().join(" ")` collapsed interior layout, so
/// `pattern:return \n$A` reached the pattern lane as the sg-ACCEPTED
/// single-line spelling `return $A`.
#[test]
fn f147a_pattern_query_preserves_interior_layout() {
    let parsed = ParsedQuery::parse("pattern:return \n$A");
    assert_eq!(parsed.mode, QueryMode::Pattern);
    let target = parsed.target.as_deref().unwrap_or_default();
    assert!(
        target.contains('\n'),
        "interior layout must survive the ingress scope split, got {target:?}"
    );
}

/// f147b (146E-F1, the repro of record): py `return \n$A` is sg 0.45.2 rc8
/// ("Cannot parse query as a valid pattern … Multiple AST nodes are
/// detected") — the indexed serve must fail closed (loud refusal or honest
/// empty), NEVER compose the sg-accepted twin's rows. Pre-fix this exact
/// query answered ok:true with 2 wrong hits (`return msg`,
/// `return text.upper()`) at BOTH official binaries and in-process.
#[test]
fn f147b_pattern_query_newline_seam_fails_closed_sg_exact() {
    let session = isolated_index_session();
    session.write(
        "greet.py",
        "def greet():\n    return msg\n\ndef loud():\n    return text.upper()\n",
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
    let result = searcher.search("pattern:return \n$A");
    assert!(
        result.is_err() || result.as_ref().unwrap().hits.is_empty(),
        "sg refuses the newline-seam pattern (rc8 multi-root); subject served {:?}",
        result.as_ref().map(|r| &r.hits)
    );
    // Control of record: the single-line spelling is sg-ACCEPTED and keeps
    // binding both corpus rows through the same ingress.
    let resp = searcher.search("pattern:return $A").unwrap();
    assert_eq!(
        resp.hits.len(),
        2,
        "single-line control must keep binding: {:?}",
        resp.hits
    );
}
