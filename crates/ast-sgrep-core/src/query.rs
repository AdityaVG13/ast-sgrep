#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    pub raw: String,
    pub mode: QueryMode,
    pub target: Option<String>,
    pub terms: Vec<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryMode {
    Hybrid,
    Callers,
    Defs,
    Imports,
    Pattern,
    Literal,
    Regex,
    Word,
}
impl ParsedQuery {
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim();
        for (prefix, mode) in [
            ("callers:", QueryMode::Callers),
            ("defs:", QueryMode::Defs),
            ("imports:", QueryMode::Imports),
        ] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                let target = rest.trim().to_string();
                return Self {
                    raw: trimmed.to_string(),
                    mode,
                    target: Some(target.clone()),
                    terms: tokenize_for_scoring(&target),
                };
            }
        }
        if let Some(rest) = trimmed.strip_prefix("pattern:") {
            let t = rest.trim().to_string();
            return Self {
                raw: trimmed.to_string(),
                mode: QueryMode::Pattern,
                target: Some(t.clone()),
                terms: vec![t],
            };
        }
        for (prefix, mode) in [
            ("literal:", QueryMode::Literal),
            ("regex:", QueryMode::Regex),
            ("word:", QueryMode::Word),
        ] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                let target = rest.trim().to_string();
                let terms = if mode == QueryMode::Word {
                    vec![target.to_lowercase()]
                } else {
                    // eh5a: literal/regex terms keep case (case-sensitive match).
                    vec![target.clone()]
                };
                // 54if: `raw` always keeps the full user query including mode prefix.
                return Self {
                    raw: trimmed.to_string(),
                    mode,
                    target: Some(target),
                    terms,
                };
            }
        }
        Self {
            raw: trimmed.to_string(),
            mode: QueryMode::Hybrid,
            target: None,
            terms: tokenize_for_scoring(trimmed),
        }
    }
    /// Build a mode-specific query. `raw` is the trimmed payload (no synthetic
    /// prefix) because constructors are invoked without a `literal:`/`regex:`/
    /// `word:` prefix. Terms preserve case for Literal/Regex (eh5a); Word lowercases.
    fn mode_query(mode: QueryMode, query: &str) -> Self {
        let trimmed = query.trim();
        let terms = match mode {
            QueryMode::Word => vec![trimmed.to_lowercase()],
            QueryMode::Literal | QueryMode::Regex => vec![trimmed.to_string()],
            _ => vec![trimmed.to_lowercase()],
        };
        Self {
            raw: trimmed.to_string(),
            mode,
            target: Some(trimmed.to_string()),
            terms,
        }
    }
    pub fn literal(query: &str) -> Self {
        Self::mode_query(QueryMode::Literal, query)
    }
    pub fn regex(query: &str) -> Self {
        Self::mode_query(QueryMode::Regex, query)
    }
    pub fn word(query: &str) -> Self {
        Self::mode_query(QueryMode::Word, query)
    }
    pub fn lookup_symbol(&self) -> String {
        self.target
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| self.primary_symbol().unwrap_or_default().to_string())
    }
    pub fn primary_symbol(&self) -> Option<&str> {
        let cased = self
            .raw
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .find(|w| w.chars().any(char::is_uppercase))
            .map(str::to_lowercase);
        cased
            .as_deref()
            .and_then(|id| self.terms.iter().find(|t| t.as_str() == id))
            .or_else(|| self.terms.iter().find(|t| looks_like_symbol(t)))
            .map(String::as_str)
    }
}
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "at", "be", "by", "do", "does", "for", "from", "how", "in", "into",
    "is", "it", "of", "on", "or", "that", "the", "this", "to", "what", "when", "where", "which",
    "who", "why", "with",
];
fn tokenize_for_scoring(input: &str) -> Vec<String> {
    let terms = tokenize_words(input, true);
    if terms.is_empty() {
        tokenize_words(input, false)
    } else {
        terms
    }
}
fn tokenize_words(input: &str, drop_stopwords: bool) -> Vec<String> {
    let mut terms = Vec::new();
    for word in input.split(|c: char| !c.is_alphanumeric() && c != '_' && c != ':') {
        let w = word.trim();
        if w.is_empty() {
            continue;
        }
        let lower = w.to_lowercase();
        if drop_stopwords && STOPWORDS.contains(&lower.as_str()) {
            continue;
        }
        terms.push(lower);
        if w.contains('_') {
            for part in w.split('_').filter(|p| !p.is_empty()) {
                terms.push(part.to_lowercase());
            }
        }
        let mut parts = Vec::new();
        for segment in w.split('_').filter(|s| !s.is_empty()) {
            let mut cur = String::new();
            for ch in segment.chars() {
                if ch.is_uppercase() && !cur.is_empty() {
                    parts.push(std::mem::take(&mut cur).to_lowercase());
                }
                cur.push(ch);
            }
            if !cur.is_empty() {
                parts.push(cur.to_lowercase());
            }
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
    term.contains('_') || term.len() > 3
}
