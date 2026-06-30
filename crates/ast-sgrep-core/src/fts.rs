pub fn escape_fts_term(term: &str) -> String {
    let escaped = term.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

pub fn escape_fts_query(terms: &[String]) -> String {
    terms
        .iter()
        .map(|t| escape_fts_term(t))
        .collect::<Vec<_>>()
        .join(" OR ")
}
