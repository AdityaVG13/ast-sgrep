use crate::query::ParsedQuery;
use crate::store::IndexStore;
use crate::Result;
use crate::search::hits::import_hit;
use crate::search::types::SearchHit;

use super::symbol::{callee_hits_for_terms, def_hits_for_terms};

const MODE_SQL_LIMIT: usize = 200;

fn prefixed_mode_query(parsed: &ParsedQuery) -> Option<ParsedQuery> {
    let symbol = parsed.lookup_symbol();
    (!symbol.is_empty()).then(|| ParsedQuery {
        terms: vec![symbol],
        ..parsed.clone()
    })
}

pub fn search_callers(
    store: &IndexStore,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
) -> Result<Vec<SearchHit>> {
    let Some(mode_query) = prefixed_mode_query(parsed) else {
        return Ok(Vec::new());
    };
    callee_hits_for_terms(
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
    let Some(mode_query) = prefixed_mode_query(parsed) else {
        return Ok(Vec::new());
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
