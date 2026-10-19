use std::collections::HashSet;
pub fn expand_concepts(text: &str) -> String {
    let tokens = tokenize(text);
    let mut expanded: HashSet<String> = tokens.iter().cloned().collect();
    for (triggers, terms) in CONCEPT_GROUPS {
        if triggers.iter().any(|t| tokens.iter().any(|x| x == *t)) {
            expanded.extend(terms.iter().map(|s| (*s).to_string()));
        }
    }
    let mut parts: Vec<_> = expanded.into_iter().collect();
    parts.sort();
    format!("{text} {}", parts.join(" "))
}
const CONCEPT_GROUPS: &[(&[&str], &[&str])] = &[
    (
        &[
            "auth",
            "authentication",
            "login",
            "credential",
            "session",
            "bearer",
        ],
        &[
            "auth",
            "authentication",
            "login",
            "credential",
            "session",
            "token",
            "bearer",
            "oauth",
            "identity",
        ],
    ),
    (
        &["refresh", "renewal", "renew", "rotate", "revoke"],
        &[
            "refresh", "renewal", "renew", "rotate", "revoke", "update", "reissue",
        ],
    ),
    (
        &["token", "jwt", "apikey", "api_key", "secret"],
        &[
            "token",
            "jwt",
            "apikey",
            "api_key",
            "secret",
            "credential",
            "key",
        ],
    ),
    (
        &["request", "http", "fetch", "client", "api"],
        &[
            "request", "http", "fetch", "client", "api", "endpoint", "call",
        ],
    ),
    (
        &["validate", "validation", "verify", "check", "sanitize"],
        &[
            "validate",
            "validation",
            "verify",
            "check",
            "sanitize",
            "guard",
        ],
    ),
    (
        &["store", "persist", "save", "cache", "database", "db"],
        &[
            "store", "persist", "save", "cache", "database", "db", "write",
        ],
    ),
    (
        &["error", "exception", "panic", "fail", "fault"],
        &["error", "exception", "panic", "fail", "fault", "handler"],
    ),
    (
        &["test", "spec", "mock", "fixture", "assert"],
        &["test", "spec", "mock", "fixture", "assert", "unittest"],
    ),
    (
        &["evict", "eviction", "prune", "purge"],
        &["evict", "eviction", "prune", "purge", "cache", "stale"],
    ),
    (
        &["redact", "redaction", "scrub", "mask"],
        &["redact", "redaction", "scrub", "mask", "secret", "sanitize"],
    ),
    (
        &["rank", "ranking", "fusion", "rrf", "critic"],
        &[
            "rank",
            "ranking",
            "fusion",
            "rrf",
            "reciprocal",
            "score",
            "critic",
            "collision",
        ],
    ),
    (
        &["snapshot", "generation", "consistency"],
        &["snapshot", "generation", "consistency", "revision", "stamp"],
    ),
    (
        &["durability", "wal", "durable"],
        &["durability", "wal", "durable", "write", "sync", "persist"],
    ),
    (
        &["hybrid", "cascade"],
        &["hybrid", "cascade", "search", "lexical", "semantic"],
    ),
    (
        &["intern", "interning"],
        &["intern", "interning", "path", "compact"],
    ),
    (
        &["derive", "follow", "planner", "command"],
        &[
            "derive",
            "follow",
            "follow_up",
            "planner",
            "suggested",
            "next",
            "command",
        ],
    ),
    (
        &["remember", "reuse", "embedding", "embeddings"],
        &[
            "remember",
            "reuse",
            "embed",
            "embedding",
            "embeddings",
            "cache",
            "query",
            "vector",
        ],
    ),
    (
        &["combine", "conjunction", "intersect", "intersection"],
        &["combine", "conjunction", "intersect", "intersection"],
    ),
    // cg5: rate / event / retry paraphrases used by invent-path gold.
    (
        &["throttle", "throttling", "ratelimit", "rate_limit", "quota"],
        &[
            "throttle",
            "throttling",
            "ratelimit",
            "rate_limit",
            "rate",
            "limit",
            "quota",
            "inbound",
            "client",
        ],
    ),
    (
        &["debounce", "debouncing", "coalesce", "coalescing"],
        &[
            "debounce",
            "debouncing",
            "coalesce",
            "coalescing",
            "noisy",
            "watch",
            "events",
            "quiet",
        ],
    ),
    (
        &["retry", "retries", "backoff", "transient"],
        &[
            "retry",
            "retries",
            "backoff",
            "transient",
            "attempt",
            "failure",
            "delay",
        ],
    ),
];
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = HashSet::new();
    for raw in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if raw.len() < 2 {
            continue;
        }
        out.insert(raw.to_lowercase());
        for part in split_ident(raw) {
            if part.len() >= 2 {
                out.insert(part);
            }
        }
    }
    let mut tokens: Vec<_> = out.into_iter().collect();
    tokens.sort();
    tokens
}

/// Split an identifier on `_`/`-` and lower→upper camel boundaries.
///
/// `refreshToken` → ["refresh", "token"]. All-caps runs stay together until a
/// lower→upper edge (`HTTPStatusCode` → ["httpstatus", "code"]), matching the
/// lexicon splitter so PPMI observations and hashed features agree.
pub fn split_ident(ident: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in ident.chars() {
        if ch == '_' || ch == '-' {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur).to_lowercase());
            }
            prev_lower = false;
            continue;
        }
        if ch.is_ascii_uppercase() && prev_lower && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur).to_lowercase());
        }
        prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        cur.push(ch);
    }
    if !cur.is_empty() {
        parts.push(cur.to_lowercase());
    }
    if parts.is_empty() {
        parts.push(ident.to_lowercase());
    }
    parts
}
