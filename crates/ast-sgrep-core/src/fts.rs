pub fn escape_fts_term(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}
pub fn escape_fts_query(terms: &[String]) -> String {
    terms.iter().map(|t| escape_fts_term(t)).collect::<Vec<_>>().join(" OR ")
}
