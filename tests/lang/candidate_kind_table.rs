//! Indexed `pattern:` search narrows the native walk to files that hold a
//! `kind:<tree-sitter-kind>` row. A missing kind in `CACHED_DECL_KIND_TABLE`
//! silently empties search for languages that emit that kind (H-CONF-012).
//! Over-broad entries are sound: the native matcher still decides every hit.

use ast_sgrep_lang::{
    cached_pattern_signatures, candidate_kind_signatures, match_pattern, Language, ParserRegistry,
};

fn kinds(pattern: &str) -> Vec<String> {
    candidate_kind_signatures(pattern).unwrap_or_else(|| panic!("native-classifiable: {pattern}"))
}

fn has(pattern: &str, kind: &str) {
    let got = kinds(pattern);
    assert!(
        got.iter().any(|s| s == kind),
        "{pattern:?} candidates must include {kind}; got {got:?}"
    );
}

#[test]
fn fn_pattern_candidates_cover_rust_and_moonbit_function_kinds() {
    // Fails if `"fn "` is only `function_item`: MoonBit indexes
    // `function_definition` / `impl_definition`, never `function_item`.
    has("fn $NAME", "kind:function_item");
    has("fn $NAME", "kind:function_definition");
    has("fn $NAME", "kind:impl_definition");
    has("fn $NAME", "kind:named_lambda_expression");
}

#[test]
fn struct_pattern_candidates_cover_moonbit_struct_kinds() {
    has("struct $NAME", "kind:struct_item");
    has("struct $NAME", "kind:struct_definition");
    has("struct $NAME", "kind:tuple_struct_definition");
}

#[test]
fn interface_pattern_candidates_cover_dart_mixin_and_moonbit_trait() {
    has("interface $NAME", "kind:trait_item");
    has("interface $NAME", "kind:mixin_declaration");
    has("interface $NAME", "kind:trait_definition");
}

#[test]
fn function_pattern_candidates_cover_dart_local_function() {
    has("function $NAME", "kind:function_declaration");
    has("function $NAME", "kind:local_function_declaration");
    has("function $NAME", "kind:getter_declaration");
    has("function $NAME", "kind:external_function_declaration");
}

#[test]
fn type_pattern_candidates_cover_c_moonbit_and_dart_extensions() {
    has("type $NAME", "kind:type_definition");
    has("type $NAME", "kind:type_item");
    has("type $NAME", "kind:extension_declaration");
    has("type $NAME", "kind:extension_type_declaration");
}

fn signatures(lang: Language, source: &str) -> Vec<String> {
    ParserRegistry::new()
        .parse(lang, source)
        .unwrap_or_else(|err| panic!("parse {lang}: {err}"))
        .pattern_nodes
        .into_iter()
        .map(|node| node.signature)
        .collect()
}

fn has_sig(got: &[String], want: &str) {
    assert!(got.iter().any(|s| s == want), "missing {want}; got {got:?}");
}

#[test]
fn dart_nested_names_emit_decl_function_rows() {
    let src = include_str!("fixtures/extract/dart.dart");
    let got = signatures(Language::Dart, src);
    has_sig(&got, "decl:function:makeWidget");
    has_sig(&got, "decl:function:formatWidget");
    has_sig(&got, "decl:function:render");
    has_sig(&got, "decl:function:label");
    has_sig(&got, "call:name.trim");
    assert_eq!(
        cached_pattern_signatures("function makeWidget"),
        Some(vec!["decl:function:makeWidget".into()])
    );
}

#[test]
fn moonbit_positional_names_emit_fn_and_type_decl_rows() {
    let src = include_str!("fixtures/extract/moonbit.mbt");
    let got = signatures(Language::MoonBit, src);
    has_sig(&got, "decl:fn:make_widget");
    has_sig(&got, "decl:fn:format_widget");
    has_sig(&got, "decl:fn:render");
    has_sig(&got, "decl:fn:decorate");
    has_sig(&got, "decl:struct:GoldenWidget");
    has_sig(&got, "decl:type:GoldenAlias");
    has_sig(&got, "decl:enum:GoldenState");
    has_sig(&got, "decl:interface:GoldenRenderable");
    has_sig(&got, "call:name.trim");
    assert!(
        !got.iter().any(|s| s == "call:trim"),
        "dot-apply must not index the trailing accessor alone; got {got:?}"
    );
    assert_eq!(
        cached_pattern_signatures("fn make_widget"),
        Some(vec!["decl:fn:make_widget".into()])
    );
}

#[test]
fn moonbit_native_queries_bind_trait_and_reject_type_as_fn_name() {
    let src = include_str!("fixtures/extract/moonbit.mbt");
    let iface = match_pattern(Language::MoonBit, src, "interface $NAME").expect("interface");
    assert!(
        iface
            .iter()
            .any(|hit| hit.excerpt.contains("GoldenRenderable")),
        "interface $NAME must match trait GoldenRenderable; got {iface:?}"
    );
    let decorate = match_pattern(Language::MoonBit, src, "fn decorate($$$)").expect("decorate");
    assert!(
        !decorate.is_empty(),
        "fn decorate($$$) must match named_lambda_expression; got {decorate:?}"
    );
    let type_as_fn =
        match_pattern(Language::MoonBit, src, "fn GoldenWidget($$$)").expect("type-as-fn");
    assert!(
        type_as_fn.is_empty(),
        "fn GoldenWidget must not match Type::method; got {type_as_fn:?}"
    );
}
