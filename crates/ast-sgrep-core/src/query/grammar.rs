use std::fmt;
use super::{ParsedQuery, QueryMode};
const LEGACY_PREFIXES: &[&str] = &[
    "callers:", "defs:", "imports:", "pattern:", "literal:", "regex:", "word:",
];
const CLAUSE_PREFIXES: &[&str] = &[
    "sem", "pattern", "path", "lang", "callers", "defs", "imports", "literal", "regex", "word",
];
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryPlan {
    pub raw: String,
    pub lexical: Option<ParsedQuery>,
    pub pattern: Option<String>,
    pub semantic: Option<String>,
    pub path_filter: Option<String>,
    pub lang_filter: Option<String>,
    pub legacy: Option<ParsedQuery>,
}
impl QueryPlan {
    pub fn parse(input: &str) -> Result<Self, QueryParseError> {
        let raw = input.trim();
        if raw.is_empty() { return Err(QueryParseError::new(0, "query is empty")); }
        if is_sole_legacy_query(raw) { return Ok(Self {
                raw: raw.to_owned(),
                lexical: None,
                pattern: None,
                semantic: None,
                path_filter: None,
                lang_filter: None,
                legacy: Some(ParsedQuery::parse(raw)),
            }); }
        let tokens = lex(input)?;
        let mut builder = PlanBuilder::default();
        let mut expect_clause = true;
        let mut last_and = None;
        let mut index = 0;
        while index < tokens.len() {
            let token = &tokens[index];
            if token.is_and() {
                if expect_clause { return Err(QueryParseError::new(token.offset, "expected a clause before AND")); }
                expect_clause = true;
                last_and = Some(token.offset);
                index += 1;
                continue;
            }
            expect_clause = false;
            index += builder.push_clause(&tokens, index)?;
        }
        if expect_clause { return Err(QueryParseError::new(last_and.unwrap_or(0), "expected a clause after AND")); }
        builder.finish(raw)
    }

    pub fn response_query(&self) -> ParsedQuery {
        self.legacy
            .clone()
            .or_else(|| self.lexical.clone())
            .or_else(|| self.semantic.as_deref().map(ParsedQuery::parse))
            .or_else(|| self.pattern.as_deref().map(ParsedQuery::parse))
            .unwrap_or_else(|| ParsedQuery::parse(&self.raw))
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryParseError {
    offset: usize,
    message: String,
}
impl QueryParseError {
    fn new(offset: usize, message: impl Into<String>) -> Self {
        Self { offset, message: message.into() }
    }
    pub fn offset(&self) -> usize {
        self.offset
    }
    pub fn position(&self) -> usize {
        self.offset
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}
impl fmt::Display for QueryParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "query syntax error at byte {}: {}", self.offset + 1, self.message)
    }
}
impl std::error::Error for QueryParseError {}
#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    value: String,
    offset: usize,
    quoted: bool,
}
impl Token {
    fn is_and(&self) -> bool {
        !self.quoted && self.value == "AND"
    }
}
fn lex(input: &str) -> Result<Vec<Token>, QueryParseError> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();
    while let Some(&(offset, ch)) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        if ch == '"' {
            chars.next();
            tokens.push(Token { value: read_quoted(offset, &mut chars)?, offset, quoted: true });
            continue;
        }
        let mut value = String::new();
        while let Some(&(_, current)) = chars.peek() {
            if current.is_whitespace() || current == '"' {
                break;
            }
            chars.next();
            if current == '\\' {
                let Some((escape_offset, escaped)) = chars.next() else {
                    return Err(QueryParseError::new(
                        input.len() - 1,
                        "trailing escape; add a character after backslash",
                    ));
                };
                value.push(decode_escape(escape_offset, escaped)?);
            } else {
                value.push(current);
            }
        }
        tokens.push(Token { value, offset, quoted: false });
    }
    Ok(tokens)
}
fn read_quoted(
    quote_offset: usize,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
) -> Result<String, QueryParseError> {
    let mut value = String::new();
    while let Some((offset, ch)) = chars.next() {
        match ch {
            '"' => return Ok(value),
            '\\' => {
                let Some((escape_offset, escaped)) = chars.next() else { return Err(QueryParseError::new(offset, "trailing escape in quoted phrase")); };
                value.push(decode_escape(escape_offset, escaped)?);
            }
            _ => value.push(ch),
        }
    }
    Err(QueryParseError::new(quote_offset, "unterminated quoted phrase; add a closing double quote"))
}
fn decode_escape(offset: usize, escaped: char) -> Result<char, QueryParseError> {
    match escaped {
        '\\' => Ok('\\'),
        '"' => Ok('"'),
        'n' => Ok('\n'),
        't' => Ok('\t'),
        ' ' => Ok(' '),
        other => Err(QueryParseError::new(
            offset.saturating_sub(1),
            format!(r#"unsupported escape \{other}; use \\, \", \n, \t, or escaped space"#),
        )),
    }
}
#[derive(Default)]
struct PlanBuilder {
    lexical_mode: Option<(QueryMode, String)>,
    lexical_terms: Vec<String>,
    pattern: Option<String>,
    semantic: Option<String>,
    path_filter: Option<String>,
    lang_filter: Option<String>,
}
impl PlanBuilder {
    fn push_clause(&mut self, tokens: &[Token], index: usize) -> Result<usize, QueryParseError> {
        let token = &tokens[index];
        if token.quoted {
            self.lexical_terms.push(token.value.clone());
            return Ok(1);
        }
        if let Some((prefix, inline)) = split_clause(&token.value) {
            let (value, consumed) = if inline.is_empty() {
                let Some(next) = tokens.get(index + 1) else { return Err(QueryParseError::new(
                        token.offset + token.value.len(),
                        format!("{prefix}: requires a value"),
                    )); };
                if next.is_and() { return Err(QueryParseError::new(next.offset, format!("{prefix}: requires a value before AND"))); }
                (next.value.clone(), 2)
            } else {
                (inline.to_owned(), 1)
            };
            if value.is_empty() { return Err(QueryParseError::new(token.offset, format!("{prefix}: requires a non-empty value"))); }
            self.set_clause(prefix, value, token.offset)?;
            return Ok(consumed);
        }
        if looks_like_unknown_clause(&token.value) {
            let prefix = token.value.split(':').next().unwrap_or_default();
            return Err(QueryParseError::new(
                token.offset,
                format!("unknown clause {prefix}:; expected sem:, pattern:, path:, lang:, or a mode prefix"),
            ));
        }
        if token.value == "OR" {
            return Err(QueryParseError::new(token.offset, "OR is not supported; combine clauses with AND"));
        }
        if token.value.contains('(') || token.value.contains(')') {
            return Err(QueryParseError::new(
                token.offset,
                "parentheses are not supported; all clauses are conjoined",
            ));
        }
        self.lexical_terms.push(token.value.clone());
        Ok(1)
    }

    fn set_clause(&mut self, prefix: &str, value: String, offset: usize) -> Result<(), QueryParseError> {
        match prefix {
            "pattern" => set_once(&mut self.pattern, value, prefix, offset),
            "sem" => set_once(&mut self.semantic, value, prefix, offset),
            "path" => set_once(&mut self.path_filter, value, prefix, offset),
            "lang" => set_once(&mut self.lang_filter, value, prefix, offset),
            "callers" => self.set_mode(QueryMode::Callers, value, prefix, offset),
            "defs" => self.set_mode(QueryMode::Defs, value, prefix, offset),
            "imports" => self.set_mode(QueryMode::Imports, value, prefix, offset),
            "literal" => self.set_mode(QueryMode::Literal, value, prefix, offset),
            "regex" => self.set_mode(QueryMode::Regex, value, prefix, offset),
            "word" => self.set_mode(QueryMode::Word, value, prefix, offset),
            _ => unreachable!("known prefix"),
        }
    }

    fn set_mode(
        &mut self,
        mode: QueryMode,
        value: String,
        prefix: &str,
        offset: usize,
    ) -> Result<(), QueryParseError> {
        if self.lexical_mode.is_some() {
            return Err(QueryParseError::new(
                offset,
                format!("duplicate mode clause {prefix}:; use only one mode prefix"),
            ));
        }
        self.lexical_mode = Some((mode, value));
        Ok(())
    }

    fn finish(self, raw: &str) -> Result<QueryPlan, QueryParseError> {
        let lexical = if let Some((mode, target)) = self.lexical_mode {
            let mut parsed = ParsedQuery::parse(&format!("{}:{target}", mode_prefix(mode)));
            if !self.lexical_terms.is_empty() {
                parsed.terms.extend(ParsedQuery::parse(&self.lexical_terms.join(" ")).terms);
                parsed.terms.sort();
                parsed.terms.dedup();
            }
            Some(parsed)
        } else if self.lexical_terms.is_empty() {
            None
        } else {
            Some(ParsedQuery::parse(&self.lexical_terms.join(" ")))
        };
        if lexical.is_none() && self.pattern.is_none() && self.semantic.is_none() {
            return Err(QueryParseError::new(
                0,
                "query has filters but no searchable clause; add terms, pattern:, or sem:",
            ));
        }
        Ok(QueryPlan {
            raw: raw.to_owned(),
            lexical,
            pattern: self.pattern,
            semantic: self.semantic,
            path_filter: self.path_filter,
            lang_filter: self.lang_filter,
            legacy: None,
        })
    }
}
fn set_once(
    slot: &mut Option<String>,
    value: String,
    prefix: &str,
    offset: usize,
) -> Result<(), QueryParseError> {
    if slot.is_some() {
        return Err(QueryParseError::new(offset, format!("duplicate {prefix}: clause; provide it only once")));
    }
    *slot = Some(value);
    Ok(())
}
fn split_clause(token: &str) -> Option<(&str, &str)> {
    let (prefix, value) = token.split_once(':')?;
    CLAUSE_PREFIXES.contains(&prefix).then_some((prefix, value))
}
fn looks_like_unknown_clause(token: &str) -> bool {
    let Some((prefix, _)) = token.split_once(':') else { return false; };
    !token.contains("::") && !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_alphabetic())
}
fn is_sole_legacy_query(input: &str) -> bool {
    let Some(prefix) = LEGACY_PREFIXES.iter().find(|p| input.starts_with(**p)) else { return false; };
    !has_composable_marker(&input[prefix.len()..])
}
fn has_composable_marker(input: &str) -> bool {
    let mut token_start = None;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            token_start.get_or_insert(offset);
            escaped = true;
            continue;
        }
        if ch == '"' {
            token_start.get_or_insert(offset);
            quoted = !quoted;
            continue;
        }
        if ch.is_whitespace() && !quoted {
            if token_start.take().is_some_and(|start| is_composable_marker(&input[start..offset])) { return true; }
        } else {
            token_start.get_or_insert(offset);
        }
    }
    token_start.is_some_and(|start| is_composable_marker(&input[start..]))
}
fn is_composable_marker(token: &str) -> bool {
    token == "AND" || token.split_once(':').is_some_and(|(name, _)| CLAUSE_PREFIXES.contains(&name))
}
fn mode_prefix(mode: QueryMode) -> &'static str {
    match mode {
        QueryMode::Callers => "callers",
        QueryMode::Defs => "defs",
        QueryMode::Imports => "imports",
        QueryMode::Literal => "literal",
        QueryMode::Regex => "regex",
        QueryMode::Word => "word",
        QueryMode::Hybrid | QueryMode::Pattern => unreachable!("not a composable lexical mode"),
    }
}
