use crate::query::ParsedQuery; use crate::rank::{best_symbol_score, score_caller, score_def, SCORE_ANCHOR, SCORE_GRAPH}; use crate::search::types::matches_lang; use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::store::sql::{caller_terms_filter, like_terms_filter, query_limit_map}; use crate::store::IndexStore; use crate::Result; const SYMBOL_SQL_LIMIT: usize = 500; const CALLER_SQL_LIMIT: usize = 500; const MODE_SQL_LIMIT: usize = 200; const TYPE_SYMBOL_WEIGHT: f64 = 0.65; const SYMBOL_KIND_ORDER: &str =
    " ORDER BY CASE WHEN s.kind IN ('function','method') THEN 0 ELSE 1 END, s.id";
const SYMBOL_SELECT: &str = "SELECT f.path, f.language, s.name, s.kind, s.line_start, s.line_end, s.byte_start, s.byte_end,
         (SELECT GROUP_CONCAT(l.content, char(10)) FROM lines l
          WHERE l.file_id = f.id AND l.line_no >= s.line_start AND l.line_no <= s.line_end ORDER BY l.line_no) AS excerpt
         FROM symbols s JOIN files f ON f.id = s.file_id";
const CALLER_SELECT: &str = "SELECT f.path, f.language, c.caller, c.callee, c.line_no, l.content
         FROM callers c JOIN files f ON f.id = c.file_id JOIN lines l ON l.file_id = c.file_id AND l.line_no = c.line_no";
type CallerQueryRow = (String, Option<String>, String, String, u32, String); type CallerFilter = fn(&[String], Option<&str>) -> (String, Vec<String>); type SymbolSpanRow = (String, Option<String>, String, String, u32, u32, String);
enum CallerMatchMode { Hybrid, CalleeOnly, } fn query_caller_rows(
    store: &IndexStore, filter: CallerFilter, terms: &[String], lang_filter: Option<&str>, limit: usize, ) -> Result<Vec<CallerQueryRow>> {
    let (where_clause, bind) = filter(terms, lang_filter); let sql = format!("{CALLER_SELECT}{where_clause} LIMIT ?{}", bind.len() + 1); query_limit_map(store.connection(), &sql, bind, limit, |row| { Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)) })
} fn caller_rows_to_hits( rows: Vec<CallerQueryRow>, options: &SearchOptions, parsed: &ParsedQuery, mode: CallerMatchMode,
) -> Result<Vec<SearchHit>> {
    let primary_lower = parsed.primary_symbol().map(|s| s.to_lowercase()); let mut hits = Vec::new(); for (path, language, caller, callee, line_no, text) in rows {
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) { continue; } let callee_score = best_symbol_score(&parsed.terms, &callee); let matched = match mode {
            CallerMatchMode::Hybrid => { callee_score > 0.0 || best_symbol_score(&parsed.terms, &caller) > 0.0 } CallerMatchMode::CalleeOnly => callee_score > 0.0,
        }; if !matched { continue; } hits.push(SearchHit::caller(
            path.clone(), language.clone(), caller.clone(), callee.clone(), line_no, text, score_caller(&parsed.terms, &callee), )); let graph = match mode {
            CallerMatchMode::CalleeOnly => Some(SCORE_GRAPH), CallerMatchMode::Hybrid => {
                let exact = callee_score >= crate::rank::SCORE_EXACT_SYMBOL
                    || primary_lower.as_ref().is_some_and(|s| callee.to_lowercase() == *s);
                exact.then_some(SCORE_GRAPH)
            }
        }; if let Some(graph_score) = graph { hits.push(SearchHit::graph_scored(path, language, caller, callee, line_no, graph_score)); }
    } Ok(hits)
} fn read_symbol_span_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolSpanRow> {
    Ok(( row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get::<_, Option<String>>(8)?.unwrap_or_default(),
    ))
} fn query_symbol_spans( store: &IndexStore, where_clause: &str, bind: Vec<String>, limit: usize,
) -> Result<Vec<SymbolSpanRow>> { let sql = format!("{SYMBOL_SELECT}{where_clause}{SYMBOL_KIND_ORDER} LIMIT ?{}", bind.len() + 1); query_limit_map(store.connection(), &sql, bind, limit, read_symbol_span_row) } fn kind_weight(kind: &str) -> f64 {
    match kind { "function" | "method" => 1.0, _ => TYPE_SYMBOL_WEIGHT, }
} fn symbol_span_rows_to_hits( rows: Vec<SymbolSpanRow>, options: &SearchOptions, kind: HitKind, score_for: impl Fn(&str) -> f64,
) -> Result<Vec<SearchHit>> {
    Ok(rows
        .into_iter() .filter(|(_, language, ..)| matches_lang(language.as_deref(), options.lang_filter.as_deref())) .map(|(path, language, name, sym_kind, line_start, line_end, text)| {
            SearchHit::span(SpanHitInput { kind, file: path, line_start, line_end, score: score_for(&name) * kind_weight(&sym_kind), excerpt: text, symbol: Some(name), language, })
        }) .collect())
} pub fn symbol_pass(store: &IndexStore, options: &SearchOptions, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> { let mut hits = def_hits_for_terms(store, options, parsed, SYMBOL_SQL_LIMIT)?; hits.extend(caller_hits_for_terms(store, options, parsed, CALLER_SQL_LIMIT)?); Ok(hits) } pub fn anchor_pass(store: &IndexStore, options: &SearchOptions, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
    let anchor_symbol = match parsed.primary_symbol() { Some(s) => s.to_string(), None => parsed.terms.iter().find(|t| t.len() > 3).cloned().unwrap_or_default(), }; if anchor_symbol.is_empty() { return Ok(vec![]); } let terms = vec![anchor_symbol]; let (where_clause, bind) = like_terms_filter("s.name", &terms, options.lang_filter.as_deref());
    let rows = query_symbol_spans(store, &where_clause, bind, SYMBOL_SQL_LIMIT)?; let term_count = parsed.terms.len(); symbol_span_rows_to_hits(rows, options, HitKind::Anchor, |name| {
        let matched = parsed.terms.iter().filter(|t| crate::rank::score_symbol(t, name) > 0.0).count(); if matched == 0 { 0.0 } else {
            SCORE_ANCHOR * (matched as f64 / term_count as f64).sqrt()
        }
    })
} pub(crate) fn def_hits_for_terms( store: &IndexStore, options: &SearchOptions, parsed: &ParsedQuery, limit: usize,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() { return Ok(vec![]); } let (where_clause, bind) = like_terms_filter("s.name", &parsed.terms, options.lang_filter.as_deref());
    let rows = query_symbol_spans(store, &where_clause, bind, limit)?; symbol_span_rows_to_hits(rows, options, HitKind::Def, |name| score_def(&parsed.terms, name))
} fn exact_name_filter(name: &str, lang: Option<&str>) -> (String, Vec<String>) {
    use crate::store::sql::{append_lang_filter, where_clause}; let mut bind = vec![name.to_string()]; let mut parts = vec!["lower(s.name) = lower(?)".into()]; append_lang_filter(&mut parts, &mut bind, lang); (where_clause(&parts), bind)
} pub(crate) fn caller_hits_for_terms( store: &IndexStore, options: &SearchOptions, parsed: &ParsedQuery, limit: usize,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() { return Ok(vec![]); } caller_rows_to_hits(
        query_caller_rows(store, caller_terms_filter, &parsed.terms, options.lang_filter.as_deref(), limit)?, options, parsed, CallerMatchMode::Hybrid, )
} fn prefixed_mode_query(parsed: &ParsedQuery) -> Option<ParsedQuery> { let symbol = parsed.lookup_symbol(); (!symbol.is_empty()).then(|| ParsedQuery { terms: vec![symbol], ..parsed.clone() }) } pub fn search_callers(store: &IndexStore, options: &SearchOptions, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
    let Some(q) = prefixed_mode_query(parsed) else { return Ok(vec![]); };
    // callers:Name uses equality on indexed callee column.
    let name = q.terms.first().map(String::as_str).unwrap_or("");
    if name.is_empty() { return Ok(vec![]); }
    use crate::store::sql::{append_lang_filter, where_clause}; let mut bind = vec![name.to_string()]; let mut parts = vec!["lower(c.callee) = lower(?)".into()];
    append_lang_filter(&mut parts, &mut bind, options.lang_filter.as_deref()); let where_clause = where_clause(&parts);
    let sql = format!("{CALLER_SELECT}{where_clause} LIMIT ?{}", bind.len() + 1);
    let rows = query_limit_map(store.connection(), &sql, bind, MODE_SQL_LIMIT, |row| {
        Ok(( row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?,
        ))
    })?; caller_rows_to_hits(rows, options, &q, CallerMatchMode::CalleeOnly)
} pub fn search_defs(store: &IndexStore, options: &SearchOptions, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
    let Some(q) = prefixed_mode_query(parsed) else { return Ok(vec![]); };
    // defs:Name is an exact symbol lookup — equality uses the name index.
    let name = q.terms.first().map(String::as_str).unwrap_or(""); if name.is_empty() { return Ok(vec![]); }
    let (where_clause, bind) = exact_name_filter(name, options.lang_filter.as_deref()); let rows = query_symbol_spans(store, &where_clause, bind, MODE_SQL_LIMIT)?;
    symbol_span_rows_to_hits(rows, options, HitKind::Def, |n| score_def(&q.terms, n))
} pub fn search_imports(store: &IndexStore, options: &SearchOptions, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
    let module = parsed.lookup_symbol(); let module = (!module.is_empty()).then_some(module.as_str()); Ok(store
        .query_imports(module, options.lang_filter.as_deref(), MODE_SQL_LIMIT)? .into_iter() .map(|(path, language, module_path, line_no)| SearchHit::import(path, language, module_path, line_no)) .collect())
}
