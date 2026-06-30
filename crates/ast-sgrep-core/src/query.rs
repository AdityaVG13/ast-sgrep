/// Parsed user query with mode and search terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    pub raw: String,
    pub mode: QueryMode,
    pub terms: Vec<String>,
}

/// Query mode derived from prefix or natural language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryMode {
    /// Default hybrid search across all passes.
    Hybrid,
    /// `callers:symbol` — who calls this symbol.
    Callers,
    /// `defs:symbol` — definition sites.
    Defs,
    /// `imports:module` — import statements.
    Imports,
}

impl ParsedQuery {
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim();
        if let Some(rest) = trimmed.strip_prefix("callers:") {
            return Self {
                raw: trimmed.to_string(),
                mode: QueryMode::Callers,
                terms: tokenize(rest),
            };
        }
        if let Some(rest) = trimmed.strip_prefix("defs:") {
            return Self {
                raw: trimmed.to_string(),
                mode: QueryMode::Defs,
                terms: tokenize(rest),
            };
        }
        if let Some(rest) = trimmed.strip_prefix("imports:") {
            return Self {
                raw: trimmed.to_string(),
                mode: QueryMode::Imports,
                terms: tokenize(rest),
            };
        }

        Self {
            raw: trimmed.to_string(),
            mode: QueryMode::Hybrid,
            terms: tokenize(trimmed),
        }
    }

    pub fn primary_symbol(&self) -> Option<&str> {
        self.terms.iter().find(|t| looks_like_symbol(t)).map(|s| s.as_str())
    }
}

fn tokenize(input: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for word in input.split(|c: char| !c.is_alphanumeric() && c != '_' && c != ':') {
        let w = word.trim();
        if w.is_empty() {
            continue;
        }
        terms.push(w.to_lowercase());
        if w.contains('_') {
            for part in w.split('_') {
                if !part.is_empty() {
                    terms.push(part.to_lowercase());
                }
            }
        }
        // camelCase splitting
        let mut parts = Vec::new();
        let mut current = String::new();
        for ch in w.chars() {
            if ch.is_uppercase() && !current.is_empty() {
                parts.push(current.to_lowercase());
                current.clear();
            }
            current.push(ch);
        }
        if !current.is_empty() {
            parts.push(current.to_lowercase());
        }
        for part in parts {
            if part.len() > 1 && !terms.contains(&part) {
                terms.push(part);
            }
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

fn looks_like_symbol(term: &str) -> bool {
    term.contains('_')
        || term
            .chars()
            .any(|c| c.is_uppercase())
        || term.len() > 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_callers_prefix() {
        let q = ParsedQuery::parse("callers:process_request");
        assert_eq!(q.mode, QueryMode::Callers);
        assert!(q.terms.contains(&"process_request".to_string()));
    }

    #[test]
    fn tokenizes_natural_language() {
        let q = ParsedQuery::parse("how does auth refresh work");
        assert!(q.terms.contains(&"auth".to_string()));
        assert!(q.terms.contains(&"refresh".to_string()));
    }
}
