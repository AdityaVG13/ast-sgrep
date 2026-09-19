//! Consolidated oracle suite: pattern gates, admission tables,
//! classification, signature-serve consistency, and hostile/codec totality.
//!
//! Replaces `oracle_foundry_pass{1,2,3}.rs` pattern legs plus the `other`
//! legs (byte codec, language table, adversarial vectors/extraction).
//! Expectations are hand-computed; errors assert discriminants
//! (`is_err`/`is_none`/emptiness), never message text.

use ast_sgrep_embed::{
    cosine_similarity, dot_similarity, embed_from_bytes, embed_to_bytes, top_k_flat_similarity,
};
use ast_sgrep_lang::{
    cached_pattern_signatures, candidate_kind_signatures, classify_native, detect_language,
    index_can_serve_pattern, is_pattern_ident, match_literal_pattern, match_pattern,
    needs_ast_grep_fallback, pattern_is_keyword_literal_root, required_pattern_literal,
    structural_term_signatures, Language, ParserRegistry,
};

/// INTENT: empty/literal/`$-less`/connector patterns stay in-process and
/// match totally (Ok, never Err): empty/ws-only/absent yield honest empty,
/// present literals hit, BOM is stripped, unified and direct literal lanes
/// agree, and hits grow monotonically with occurrences.
///
/// KILLS: literal-fallback-mistable, empty-pattern-Err, connector-match-breach
/// (`->`/`::`), `$-less`-fallback-breach, literal-lane-removal,
/// trim/BOM-strip-removal, error-on-absent, lane-divergence,
/// non-monotonic-match, unicode-Err/empty.
///
/// ABSORBS: pass1::pattern_gates_treat_empty_and_literal_as_native,
/// pass3::connector_carve_and_fallback_class,
/// pass2::match_pattern_literal_and_trivia_edges,
/// pass3::literal_differential_and_match_monotonicity.
#[test]
fn native_gates_and_literal_matching_stay_total() {
    // Empty and plain-literal patterns stay in-process; empty is Ok-empty.
    assert!(!needs_ast_grep_fallback(""));
    assert!(!needs_ast_grep_fallback("process_request"));
    let hits = match_pattern(Language::Rust, "fn foo() {}\n", "").expect("empty ok");
    assert!(hits.is_empty());
    // Bare connectors are lenient-parse garbage: silent empty even where the
    // bytes occur in the source.
    let arrow = match_pattern(Language::Rust, "fn f() -> i32 { 1 }\n", "->").expect("arrow");
    assert!(arrow.is_empty());
    let scope = match_pattern(Language::Rust, "use a::b;\nfn f() {}\n", "::").expect("scope");
    assert!(scope.is_empty());
    // The $-less class never needs the external engine, whatever the bytes.
    for pattern in [
        "",
        "   ",
        "αβ",
        "->",
        "::",
        ";",
        "foo(1, 2)",
        "fn foo() { }",
        "if (x) { y; }",
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern:?} must stay in-process"
        );
    }
    // Literal present hits; ws-only/BOM/absent behave (Ok-empty, never Err).
    let source = "fn foo() { foo(); }\n";
    assert!(!match_pattern(Language::Rust, source, "foo").expect("literal").is_empty());
    assert!(match_pattern(Language::Rust, source, "   ").expect("ws").is_empty());
    assert!(!match_pattern(Language::Rust, source, "\u{feff}foo").expect("bom").is_empty());
    assert!(match_pattern(Language::Rust, source, "absent_ident").expect("absent").is_empty());
    // Differential: unified and direct literal lanes agree bit-exactly.
    let unified = match_pattern(Language::Rust, source, "foo").expect("unified");
    let direct = match_literal_pattern(Language::Rust, source, "foo").expect("direct");
    assert!(!unified.is_empty());
    assert_eq!(unified, direct);
    // Empty source: both lanes agree on the honest empty.
    assert!(
        match_pattern(Language::Rust, "", "foo")
            .expect("empty unified")
            .is_empty()
    );
    assert!(
        match_literal_pattern(Language::Rust, "", "foo")
            .expect("empty direct")
            .is_empty()
    );
    // Monotonicity: appending another occurrence strictly grows the hit set.
    let one = match_pattern(Language::Rust, "fn foo() {}\n", "foo").expect("one");
    let two = match_pattern(Language::Rust, "fn foo() {}\nfn foo() {}\n", "foo").expect("two");
    assert!(!one.is_empty());
    assert!(two.len() > one.len(), "{} vs {}", two.len(), one.len());
    // Adversarial unicode ident: total (no error) and deterministic.
    let uni = "fn αβ() {}\n";
    let first = match_pattern(Language::Rust, uni, "αβ").expect("unicode ok");
    let second = match_pattern(Language::Rust, uni, "αβ").expect("unicode ok");
    assert_eq!(first, second);
    assert!(!first.is_empty(), "unicode ident must match");
}

/// INTENT: keyword-literal roots, ident admission, and native classification
/// match exact hand tables; classification is invariant under trim and decl
/// modifiers and rejects case/garbage/empty.
///
/// KILLS: keyword-matches-arm add/drop, trim-removal, case-fold,
/// ident-admission-table add/drop, trim-sensitivity, modifier-strip-drop,
/// permissive-head.
///
/// ABSORBS: pass2::keyword_literal_roots_match_exact_table,
/// pass3::is_pattern_ident_admission_table,
/// pass3::classify_native_trim_and_modifier_invariance.
#[test]
fn keyword_ident_and_classify_admission_tables() {
    // Keyword-literal roots: exact 9-in/7-out table incl trim/case edges.
    for word in [
        "null", "true", "false", "True", "False", "None", "this", "super", " null ",
    ] {
        assert!(pattern_is_keyword_literal_root(word), "{word:?}");
    }
    for word in ["", "undefined", "self", "nul", "NULL", "Truex", "none"] {
        assert!(!pattern_is_keyword_literal_root(word), "{word:?}");
    }
    // Ident admission: 6-in/8-out incl unicode/digit/punct edges.
    for ident in ["foo", "_", "_a1", "A", "αβ", "café"] {
        assert!(is_pattern_ident(ident), "{ident:?}");
    }
    for ident in ["", "1a", "a-b", "a b", "foo(", "$A", "::", "->"] {
        assert!(!is_pattern_ident(ident), "{ident:?}");
    }
    // Surrounding trivia never changes classification.
    assert_eq!(
        classify_native("  fn $NAME($$$)  "),
        classify_native("fn $NAME($$$)")
    );
    assert!(classify_native("fn $NAME($$$)").is_some());
    // Declaration modifiers strip to the bare spelling.
    assert_eq!(
        classify_native("pub fn $NAME"),
        classify_native("fn $NAME")
    );
    assert_eq!(
        classify_native("export function $NAME"),
        classify_native("function $NAME")
    );
    assert_eq!(
        classify_native("pub(crate) fn $NAME"),
        classify_native("fn $NAME")
    );
    // Loud rejects: case-sensitive prefixes, garbage meta heads, empty.
    assert!(classify_native("Fn $NAME").is_none());
    assert!(classify_native("fn $3").is_none());
    assert!(classify_native("").is_none());
}

/// INTENT: signature derivation and index-serve agree byte-exactly
/// (ident/decl serve, kind-only never, statement keywords escape) and
/// prefilter/structural keys stay byte-identical.
///
/// KILLS: serve-table flip, prefilter-literal mistable, kind-signature drift,
/// structural-key drift.
///
/// ABSORBS: pass3::signature_serve_consistency.
#[test]
fn signature_serve_and_prefilter_keys_are_byte_exact() {
    // Ident and concrete decl shapes are index-exact.
    assert_eq!(
        cached_pattern_signatures("foo"),
        Some(vec!["foo".to_string()])
    );
    assert!(index_can_serve_pattern("foo", &["foo".to_string()]));
    assert_eq!(
        cached_pattern_signatures("fn greet"),
        Some(vec!["decl:fn:greet".to_string()])
    );
    assert!(index_can_serve_pattern("fn greet", &["decl:fn:greet".to_string()]));
    // Kind-only shapes always need native confirmation.
    let kinds = vec![
        "kind:function_item".to_string(),
        "kind:function_definition".to_string(),
        "kind:impl_definition".to_string(),
        "kind:named_lambda_expression".to_string(),
    ];
    assert_eq!(cached_pattern_signatures("fn $NAME"), Some(kinds.clone()));
    assert!(!index_can_serve_pattern("fn $NAME", &kinds));
    assert!(!index_can_serve_pattern("fn $NAME() { $$$BODY }", &[]));
    // Statement keywords escape ident-serve even with ident rows.
    assert!(!index_can_serve_pattern("return", &["return".to_string()]));
    assert!(!index_can_serve_pattern("debugger", &["debugger".to_string()]));
    assert_eq!(cached_pattern_signatures(""), Some(vec![]));
    // Prefilter literals: exact ident, longest token, meta-name hole, if head.
    assert_eq!(required_pattern_literal("foo"), Some("foo".to_string()));
    assert_eq!(
        required_pattern_literal("fn greet"),
        Some("greet".to_string())
    );
    assert_eq!(required_pattern_literal("fn $NAME"), None);
    assert_eq!(
        required_pattern_literal("if $C { $B }"),
        Some("if".to_string())
    );
    assert_eq!(required_pattern_literal(""), None);
    // Braced decl templates narrow by kind but never serve exactly.
    assert_eq!(
        candidate_kind_signatures("fn $N($$$) { $$$ }"),
        Some(kinds)
    );
    assert_eq!(candidate_kind_signatures(""), None);
    // Structural boost keys stay byte-identical.
    assert_eq!(
        structural_term_signatures("foo"),
        [
            "call-name:foo".to_string(),
            "call:foo".to_string(),
            "decl:fn:foo".to_string(),
            "decl:def:foo".to_string(),
            "decl:function:foo".to_string(),
            "foo".to_string(),
        ]
    );
}

/// INTENT: out-of-kernel surfaces stay total with exact hand discriminants:
/// the byte codec roundtrips bit-exactly and rejects ragged lengths, the
/// language parse/extension/detect tables agree per the alias table with
/// shebang sniffing and silent-None unknowns, and hostile vectors plus
/// deep-nesting extraction collapse loudly (truncation flag) but never Err.
///
/// KILLS: width/endianness mutant, ragged-length-accept, alias-table mistable
/// (ext-vs-name lanes), detect/parse divergence, sniffing-drop,
/// NaN-poisoning at SIMD length, huge-dim-overflow, silent-depth-breach,
/// empty-Err.
///
/// ABSORBS: pass1::embed_bytes_roundtrip_is_bit_exact,
/// pass3::language_parse_roundtrip_and_extension_table,
/// pass3::adversarial_vectors_and_deep_extraction.
#[test]
fn codec_language_and_hostile_inputs_stay_total() {
    // Byte codec: bit-exact roundtrip incl nonfinite; ragged lengths Err.
    let vec = vec![0.0, 1.0, -2.5, f32::INFINITY, f32::NEG_INFINITY, f32::NAN];
    let bytes = embed_to_bytes(&vec);
    assert_eq!(bytes.len(), 4 * vec.len());
    let back = embed_from_bytes(&bytes).expect("roundtrip");
    assert_eq!(back.len(), vec.len());
    for (a, b) in vec.iter().zip(back.iter()) {
        assert_eq!(a.to_bits(), b.to_bits());
    }
    assert_eq!(embed_from_bytes(&[]).expect("empty ok"), Vec::<f32>::new());
    assert!(embed_from_bytes(&[0u8; 5]).is_err());
    assert!(embed_from_bytes(&[0u8; 3]).is_err());

    // Every stored id parses back to itself and canonicalizes to itself.
    for lang in Language::all() {
        assert_eq!(Language::parse(lang.as_str()), Some(*lang));
        assert_eq!(
            Language::canonical_filter(Some(lang.as_str())).as_deref(),
            Some(lang.as_str())
        );
    }
    // Hand alias table (case/whitespace-tolerant). Extension spellings
    // resolve through from_extension; name spellings only through parse.
    let ext_aliases: &[(&str, Language)] = &[
        ("RS", Language::Rust),
        ("HPP", Language::Cpp),
        ("Ts", Language::TypeScript),
        (" h ", Language::C),
    ];
    for (raw, lang) in ext_aliases {
        assert_eq!(Language::parse(raw), Some(*lang), "{raw:?}");
        assert_eq!(Language::from_extension(raw), Some(*lang), "{raw:?}");
    }
    let name_aliases: &[(&str, Language)] = &[
        ("golang", Language::Go),
        ("C#", Language::CSharp),
        ("c++", Language::Cpp),
        ("cuda", Language::Cpp),
    ];
    for (raw, lang) in name_aliases {
        assert_eq!(Language::parse(raw), Some(*lang), "{raw:?}");
        assert_eq!(Language::from_extension(raw), None, "{raw:?}");
    }
    // Differential: detect_language agrees with from_extension on every
    // indexed extension, in both cases.
    for (ext, lang) in Language::SOURCE_EXTENSIONS {
        let rel = format!("n.{ext}");
        assert_eq!(
            detect_language(std::path::Path::new(&rel), None),
            Some(*lang),
            "detect({ext})"
        );
        assert_eq!(
            detect_language(std::path::Path::new(&rel), None),
            Language::from_extension(ext)
        );
        assert_eq!(
            Language::from_extension(&ext.to_ascii_uppercase()),
            Some(*lang)
        );
    }
    // Shebang/content sniffing when no extension matches.
    let no_ext = std::path::Path::new("Makefile");
    assert_eq!(
        detect_language(no_ext, Some("package main\n")),
        Some(Language::Go)
    );
    assert_eq!(
        detect_language(no_ext, Some("#!/usr/bin/env python\n")),
        Some(Language::Python)
    );
    assert_eq!(
        detect_language(no_ext, Some("#!/usr/bin/ruby\n")),
        Some(Language::Ruby)
    );
    assert_eq!(
        detect_language(no_ext, Some("<?php echo 1;")),
        Some(Language::Php)
    );
    // Unknown extension and unrecognized content stay silent None.
    assert_eq!(
        detect_language(std::path::Path::new("x.zzz"), None),
        None
    );
    assert_eq!(detect_language(no_ext, Some("just text")), None);
    assert_eq!(Language::parse(""), None);
    assert_eq!(Language::parse("fortran"), None);

    // SIMD-length all-NaN vectors collapse to zero, never NaN.
    let nan70 = vec![f32::NAN; 70];
    let one70 = vec![1.0f32; 70];
    assert_eq!(dot_similarity(&nan70, &one70), 0.0);
    assert_eq!(cosine_similarity(&nan70, &one70), 0.0);
    // A NaN query scores every row exactly 0.0 and the unthresholded lane
    // keeps both rows, tie by index.
    assert_eq!(
        top_k_flat_similarity(&[f32::NAN, f32::NAN], &[1.0, 0.0, 0.0, 1.0], 2, 5, None),
        vec![(0, 0.0), (1, 0.0)]
    );
    // Huge dim stays finite and exact-class: 100k ones sum to 100000.
    let huge = vec![1.0f32; 100_000];
    let total = dot_similarity(&huge, &huge);
    assert!((total - 100_000.0).abs() < 1.0, "total={total}");
    // Extraction: shallow file is complete; 300-deep parens breach the
    // 256 cap (loud truncation flag, still Ok); empty is Ok-empty.
    let registry = ParserRegistry::new();
    let shallow = registry.parse(Language::Rust, "fn f() {}\n").expect("shallow");
    assert!(!shallow.depth_truncated);
    assert_eq!(shallow.symbols.len(), 1);
    assert_eq!(shallow.symbols[0].name, "f");
    let deep_src = format!("fn f() {{ let x = {}1{}; }}\n", "(".repeat(300), ")".repeat(300));
    let deep = registry.parse(Language::Rust, &deep_src).expect("deep ok");
    assert!(deep.depth_truncated, "300-deep parens must truncate");
    let empty = registry.parse(Language::Rust, "").expect("empty ok");
    assert!(empty.symbols.is_empty());
    assert!(!empty.depth_truncated);
}
