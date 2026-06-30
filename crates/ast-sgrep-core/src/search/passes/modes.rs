use crate::query::ParsedQuery;
use crate::store::IndexStore;
use crate::Result;
use crate::search::hits::import_hit;
use crate::search::types::SearchHit;

use super::symbol::{caller_hits_for_terms, def_hits_for_terms};

const MODE_SQL_LIMIT: usize = 200;

pub fn search_callers(
    store: &IndexStore,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
) -> Result<Vec<SearchHit>> {
    let symbol = parsed.lookup_symbol();
    if symbol.is_empty() {
        return Ok(Vec::new());
    }
    let mode_query = ParsedQuery {
        terms: vec![symbol],
        ..parsed.clone()
    };
    caller_hits_for_terms(
        store,
        &mode_search_options(),
        &mode_query,
        excerpt,
        MODE_SQL_LIMIT,
    )
}

pub fn search_defs(
    store: &IndexStore,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
) -> Result<Vec<SearchHit>> {
    let symbol = parsed.lookup_symbol();
    if symbol.is_empty() {
        return Ok(Vec::new());
    }
    let mode_query = ParsedQuery {
        terms: vec![symbol],
        ..parsed.clone()
    };
    def_hits_for_terms(
        store,
        &mode_search_options(),
        &mode_query,
        excerpt,
        MODE_SQL_LIMIT,
    )
}

pub fn search_imports(store: &IndexStore, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
    let module = parsed.lookup_symbol();
    let module = if module.is_empty() { None } else { Some(module.as_str()) };
    Ok(store
        .query_imports(module, MODE_SQL_LIMIT)?
        .into_iter()
        .map(|(path, language, module_path, line_no)| {
            import_hit(path, language, module_path, line_no)
        })
        .collect())
}

fn mode_search_options() -> crate::search::types::SearchOptions {
    crate::search::types::SearchOptions {
        lang_filter: None,
        ..crate::search::types::SearchOptions::default()
    }
}
