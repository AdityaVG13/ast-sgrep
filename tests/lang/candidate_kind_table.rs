//! Indexed `pattern:` search narrows the native walk to files that hold a
//! `kind:<tree-sitter-kind>` row. A missing kind in `CACHED_DECL_KIND_TABLE`
//! silently empties search for languages that emit that kind (H-CONF-012).
//! Over-broad entries are sound: the native matcher still decides every hit.

use ast_sgrep_lang::candidate_kind_signatures;

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
}
