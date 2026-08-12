use super::clamp_lsp_search_limit;

#[test]
fn remaps_zero_and_caps_ceiling() {
    let def = ast_sgrep_core::SearchOptions::default_limit().max(1);
    assert_eq!(clamp_lsp_search_limit(0), def.min(1000));
    assert_eq!(clamp_lsp_search_limit(32), 32);
    assert_eq!(clamp_lsp_search_limit(500), 500);
    assert_eq!(clamp_lsp_search_limit(10_000), 1000);
}
