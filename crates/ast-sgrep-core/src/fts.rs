//! FTS5 query term escaping for safe MATCH clauses.

/// Escape a single term for SQLite FTS5 MATCH (wrap in double quotes, escape internal quotes).
pub fn escape_fts_term(term: &str) -> String {
    let escaped = term.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

/// Build an OR-joined FTS5 query from multiple terms.
pub fn escape_fts_query(terms: &[String]) -> String {
    terms
        .iter()
        .map(|t| escape_fts_term(t))
        .collect::<Vec<_>>()
        .join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_quotes_and_special_chars() {
        assert_eq!(escape_fts_term("foo"), "\"foo\"");
        assert_eq!(escape_fts_term("foo\"bar"), "\"foo\"\"bar\"");
    }

    #[test]
    fn joins_terms_with_or() {
        let q = escape_fts_query(&["auth".into(), "refresh".into()]);
        assert_eq!(q, "\"auth\" OR \"refresh\"");
    }
}
