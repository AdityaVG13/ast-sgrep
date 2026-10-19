#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    pub raw: String,
    pub mode: QueryMode,
    pub target: Option<String>,
    pub terms: Vec<String>,
    /// Directory or glob from an `in:path` token. Applied as a file filter.
    pub path_scope: Option<String>,
    /// Set when an `in:` token was present but unresolvable at parse time
    /// — bare `in:`, a `..` segment,
    /// an absolute path, or a duplicate token. Search must refuse loudly;
    /// the historic silent drop silently ran the query UNSCOPED.
    pub path_scope_error: Option<String>,
    /// Set by the Searcher when the scope resolves to a FILE under the
    /// index root — finish filters by exact rel-path equality
    /// instead of the impossible `file/**` glob.
    pub path_scope_exact: bool,
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
        // L-Q1c (2026-10-17): recognized short-question scaffolding is
        // stripped from the raw query BEFORE any lane sees it — the sim
        // measured whole-query replace, not a lexical-only strip.
        let rewritten;
        let trimmed = match rewrite_short_question(trimmed) {
            Some(s) => {
                rewritten = s;
                rewritten.as_str()
            }
            None => trimmed,
        };
        let (without_scope, path_scope, path_scope_error) = split_in_path_scope(trimmed);
        let mut parsed = Self::parse_mode(without_scope.trim());
        parsed.path_scope = path_scope;
        parsed.path_scope_error = path_scope_error;
        parsed
    }

    fn parse_mode(trimmed: &str) -> Self {
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
                    path_scope: None,
                    path_scope_error: None,
                    path_scope_exact: false,
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
                path_scope: None,
                path_scope_error: None,
                path_scope_exact: false,
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
                    path_scope: None,
                    path_scope_error: None,
                    path_scope_exact: false,
                };
            }
        }
        Self {
            raw: trimmed.to_string(),
            mode: QueryMode::Hybrid,
            target: None,
            terms: tokenize_for_scoring(trimmed),
            path_scope: None,
            path_scope_error: None,
            path_scope_exact: false,
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
            path_scope: None,
            path_scope_error: None,
            path_scope_exact: false,
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
    /// Identifier as the user typed it (`Searcher`, `auth_refresh`), not folded.
    pub fn identifier_spelling(&self) -> Option<&str> {
        if let Some(target) = self.target.as_deref() {
            let trimmed = target.trim();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
        self.raw
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .find(|word| {
                !word.is_empty()
                    && (word.chars().any(char::is_uppercase)
                        || word.contains('_')
                        || looks_like_symbol(word))
            })
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
/// Genre boilerplate of long natural-language code-search queries: words
/// that describe the question rather than the code ("purpose", "verify").
/// Only applied to long queries (see `QUERY_BOILERPLATE_LEN_THRESHOLD`).
/// Probe: stripping these from 20 embed-won/no-embed-lost RepoQA queries
/// recovered 3 to top-10 and never meaningfully hurt; the full-query
/// fallback below keeps the empty-strip case safe.
const QUERY_BOILERPLATE: &[&str] = &[
    "allow",
    "allows",
    "appropriate",
    "attribute",
    "attributes",
    "certain",
    "class",
    "correct",
    "correctly",
    "described",
    "design",
    "designed",
    "ensures",
    "ensure",
    "following",
    "function",
    "functions",
    "given",
    "input",
    "intended",
    "method",
    "methods",
    "object",
    "output",
    "particular",
    "provide",
    "provides",
    "purpose",
    "relevant",
    "responsible",
    "retrieve",
    "retrieves",
    "return",
    "returns",
    "specific",
    "use",
    "used",
    "using",
    "value",
    "values",
    "various",
    "verify",
    "verifies",
];
/// Minimum tokenized-query length before genre-boilerplate stripping
/// applies. Short queries keep every term.
const QUERY_BOILERPLATE_LEN_THRESHOLD: usize = 15;
/// Frozen stop list of the L-Q1c sim (2026-10-17). Deliberately NOT the
/// lexical `STOPWORDS` below: this exact list produced the measured
/// CoSQA +.0102 / RepoQA bit-identical arm and is part of the lever's
/// pre-declared contract; changing it is a new lever needing a new arm.
const SHORT_QUESTION_STOPWORDS: &[&str] = &[
    "a", "an", "the", "of", "to", "in", "on", "for", "with", "without", "using", "and", "or", "is",
    "are", "be", "been", "do", "does", "did", "can", "could", "should", "would", "will", "how",
    "what", "why", "when", "where", "which", "who", "i", "you", "we", "they", "it", "its", "this",
    "that", "these", "those", "my", "your", "our", "their", "there", "here", "best", "way",
    "python", "code", "function", "method", "returns", "return", "get", "getting",
];

/// L-Q1c short-question boilerplate rewrite (pre-declared 2026-10-17,
/// plan Phase 5; offline sim: CoSQA MRR@10 .3169 → .3271, RepoQA
/// bit-identical, 0/600 RepoQA queries match). A bare natural-language
/// question whose scaffolding is recognized — a leading
/// "how to / how do i / what('s| is)[ the][ best| easiest| right] way [to]
/// / is there a way to" or a trailing "in|using|with|for python" — is
/// replaced by its stripped form (scaffolding + frozen sim stop list
/// dropped, whitespace collapsed). Never touches mode-prefixed queries
/// (pattern/literal text is byte-faithful by contract) and never yields
/// an empty query (no rewrite instead).
fn rewrite_short_question(trimmed: &str) -> Option<String> {
    const MODE_PREFIXES: &[&str] = &[
        "callers:", "defs:", "imports:", "pattern:", "literal:", "regex:", "word:",
    ];
    if MODE_PREFIXES.iter().any(|p| trimmed.starts_with(p)) {
        return None;
    }
    fn lead_pattern() -> &'static regex::Regex {
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        RE.get_or_init(|| {
            regex::Regex::new(
                r"(?i)^(?:how (?:to|do i|can i|does one)|what(?:'s| is)(?: the)?(?: best| easiest| right)? way to?|is there a way to)\s+",
            )
            .expect("static lead pattern compiles")
        })
    }
    fn tail_pattern() -> &'static regex::Regex {
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        RE.get_or_init(|| {
            regex::Regex::new(r"(?i)\s+(?:in|using|with|for) python\s*$")
                .expect("static tail pattern compiles")
        })
    }
    let mut rest = trimmed;
    let mut matched = false;
    if let Some(m) = lead_pattern().find(rest) {
        if m.start() == 0 {
            rest = &rest[m.end()..];
            matched = true;
        }
    }
    if let Some(m) = tail_pattern().find(rest) {
        rest = &rest[..m.start()];
        matched = true;
    }
    if !matched {
        return None;
    }
    let kept: Vec<&str> = rest
        .split_whitespace()
        .filter(|t| !SHORT_QUESTION_STOPWORDS.contains(&t.to_lowercase().as_str()))
        .collect();
    if kept.is_empty() {
        return None;
    }
    Some(kept.join(" "))
}

fn tokenize_for_scoring(input: &str) -> Vec<String> {
    let terms = tokenize_words(input, true);
    let terms = if terms.is_empty() {
        tokenize_words(input, false)
    } else {
        terms
    };
    // Long natural-language queries carry genre boilerplate ("purpose",
    // "designed to", "verify that ... correctly") that matches hundreds of
    // files and drowns the discriminative terms. Drop it only when the
    // query is long enough that dilution dominates; short queries keep
    // every term. Never return empty: fall back to the full set.
    if terms.len() > QUERY_BOILERPLATE_LEN_THRESHOLD {
        let kept: Vec<String> = terms
            .iter()
            .filter(|t| !QUERY_BOILERPLATE.contains(&t.as_str()))
            .cloned()
            .collect();
        if !kept.is_empty() {
            return kept;
        }
    }
    terms
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

fn split_in_path_scope(input: &str) -> (String, Option<String>, Option<String>) {
    // The scope split LOCATES `in:` tokens by whitespace word but must
    // splice the REMAINING RAW BYTES. The old `split_whitespace().join(" ")`
    // collapsed the query's interior layout, so a `pattern:` query carrying
    // an interior line break (`pattern:return \n$A`) reached the pattern
    // lane as the reference-accepted single-line spelling `return $A` and
    // over-served rows the reference refuses to parse ("Multiple AST nodes
    // are detected"; the oracle pins the py/js/go newline-seam cells). Byte
    // fidelity is the reference-exact contract on every mode prefix: the
    // reference parses the RAW pattern text, so the ingress must never
    // rewrite it.
    //
    // An `in:` token is honored only (a) at a whitespace word start AND (b)
    // outside double quotes —
    // an odd `"` count before the token means it sits inside a quoted
    // literal and stays pattern text (single-quoted literals remain a
    // documented boundary). Honored tokens that are bare, carry a `..`
    // segment, are absolute, or duplicate an earlier token set
    // `path_scope_error` for the serve lane to refuse loudly; the historic
    // silent drop ran the query UNSCOPED (fail-open).
    let mut scope = None;
    let mut scope_error = None;
    let mut out = String::with_capacity(input.len());
    let mut copy_from = 0usize;
    let mut cursor = 0usize;
    let mut quotes_scanned = 0usize;
    while cursor < input.len() {
        let tail = &input[cursor..];
        let lead_ws = tail.len() - tail.trim_start().len();
        let start = cursor + lead_ws;
        if start >= input.len() {
            break;
        }
        let token_len = input[start..].split_whitespace().next().map_or(0, str::len);
        let end = start + token_len;
        let inside_quotes = quotes_scanned % 2 == 1;
        quotes_scanned += input[start..end].matches('"').count();
        if let Some(path) = input[start..end].strip_prefix("in:") {
            if !inside_quotes {
                // Drop the token together with the separator run that led to
                // it, so the spliced payload never gains a doubled separator
                // where an `in:` sat between payload words.
                out.push_str(&input[copy_from..start]);
                copy_from = end;
                let escape = path.split(['/', '\\']).any(|seg| seg == "..");
                if path.is_empty() {
                    scope_error =
                        Some("in: token has no path; scope queries look like `in:src`".into());
                } else if escape {
                    scope_error = Some(format!(
                        "in: scope '{path}' escapes the index root: '..' segments are not allowed"
                    ));
                } else if std::path::Path::new(path).is_absolute() {
                    scope_error = Some(format!(
                        "in: scope '{path}' is absolute; scopes are relative to the index root"
                    ));
                } else if scope.is_some() {
                    scope_error = Some(format!(
                        "multiple in: scopes (first: '{}'); keep exactly one",
                        scope.as_deref().unwrap_or_default()
                    ));
                } else {
                    scope = Some(path.to_string());
                }
            }
        }
        cursor = end;
    }
    out.push_str(&input[copy_from..]);
    (out.trim().to_string(), scope, scope_error)
}

/// Turn an `in:path` token into a file_filter glob (`src` → `src/**`).
pub fn path_scope_glob(scope: &str) -> String {
    if scope.contains('*') || scope.contains('?') {
        return scope.to_string();
    }
    if scope.ends_with('/') {
        format!("{scope}**")
    } else {
        format!("{scope}/**")
    }
}
