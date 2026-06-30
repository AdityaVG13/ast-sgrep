//! Code-aware local semantic embeddings with concept expansion.
//!
//! Replaces the old hash bag-of-words embedder with token + char-ngram features
//! and a built-in code-domain concept lexicon so synonym queries like
//! "credential renewal" can match `auth_refresh` without neural models.

use std::collections::HashSet;

/// Vector dimension for local semantic embeddings.
pub const SEMANTIC_DIM: usize = 256;

/// Expand text with related code-domain concepts for richer retrieval.
pub fn expand_concepts(text: &str) -> String {
    let tokens = tokenize(text);
    let mut expanded: HashSet<String> = tokens.iter().cloned().collect();

    for group in CONCEPT_GROUPS {
        if group.trigger.iter().any(|t| tokens.contains(&t.to_string())) {
            for term in group.terms {
                expanded.insert(term.to_string());
            }
        }
    }

    let mut parts: Vec<String> = expanded.into_iter().collect();
    parts.sort();
    format!("{text} {}", parts.join(" "))
}

struct ConceptGroup {
    trigger: &'static [&'static str],
    terms: &'static [&'static str],
}

/// Code-domain synonym groups used at index and query time.
const CONCEPT_GROUPS: &[ConceptGroup] = &[
    ConceptGroup {
        trigger: &["auth", "authentication", "login", "credential", "session", "bearer"],
        terms: &[
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
    },
    ConceptGroup {
        trigger: &["refresh", "renewal", "renew", "rotate", "revoke"],
        terms: &[
            "refresh",
            "renewal",
            "renew",
            "rotate",
            "revoke",
            "update",
            "reissue",
        ],
    },
    ConceptGroup {
        trigger: &["token", "jwt", "apikey", "api_key", "secret"],
        terms: &["token", "jwt", "apikey", "api_key", "secret", "credential", "key"],
    },
    ConceptGroup {
        trigger: &["request", "http", "fetch", "client", "api"],
        terms: &["request", "http", "fetch", "client", "api", "endpoint", "call"],
    },
    ConceptGroup {
        trigger: &["validate", "validation", "verify", "check", "sanitize"],
        terms: &["validate", "validation", "verify", "check", "sanitize", "guard"],
    },
    ConceptGroup {
        trigger: &["store", "persist", "save", "cache", "database", "db"],
        terms: &["store", "persist", "save", "cache", "database", "db", "write"],
    },
    ConceptGroup {
        trigger: &["error", "exception", "panic", "fail", "fault"],
        terms: &["error", "exception", "panic", "fail", "fault", "handler"],
    },
    ConceptGroup {
        trigger: &["test", "spec", "mock", "fixture", "assert"],
        terms: &["test", "spec", "mock", "fixture", "assert", "unittest"],
    },
];

/// Split identifiers and normalize tokens from source text.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = HashSet::new();
    for raw in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if raw.len() < 2 {
            continue;
        }
        out.insert(raw.to_lowercase());
        for part in split_identifier(raw) {
            if part.len() >= 2 {
                out.insert(part);
            }
        }
    }
    let mut tokens: Vec<String> = out.into_iter().collect();
    tokens.sort();
    tokens
}

fn split_identifier(ident: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();

    for ch in ident.chars() {
        if ch == '_' {
            if !current.is_empty() {
                parts.push(current.to_lowercase());
                current.clear();
            }
            continue;
        }
        if ch.is_ascii_uppercase() && !current.is_empty() {
            parts.push(current.to_lowercase());
            current.clear();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        parts.push(current.to_lowercase());
    }
    if parts.is_empty() {
        parts.push(ident.to_lowercase());
    }
    parts
}

fn char_trigrams(text: &str) -> Vec<String> {
    let compact: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    if compact.len() < 3 {
        return Vec::new();
    }
    compact
        .as_bytes()
        .windows(3)
        .map(|w| String::from_utf8_lossy(w).into_owned())
        .collect()
}

fn hash_feature(feature: &str, vec: &mut [f32], weight: f32) {
    let hash = blake3::hash(feature.as_bytes());
    let bytes = hash.as_bytes();
    for i in 0..vec.len() {
        let b = bytes[i % bytes.len()];
        vec[i] += if b & 1 == 0 { weight } else { -weight };
    }
}

fn normalize(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vec.iter_mut() {
            *x /= norm;
        }
    }
}

/// Code-aware local semantic embedding — fully offline, no hash bag-of-words.
#[derive(Debug, Clone, Default)]
pub struct SemanticLocalEmbedding;

impl SemanticLocalEmbedding {
    pub fn embed_text(&self, text: &str) -> Vec<f32> {
        let expanded = expand_concepts(text);
        let mut vec = vec![0.0_f32; SEMANTIC_DIM];

        for token in tokenize(&expanded) {
            hash_feature(&format!("tok:{token}"), &mut vec, 1.0);
        }
        for tri in char_trigrams(&expanded) {
            hash_feature(&format!("tri:{tri}"), &mut vec, 0.35);
        }

        normalize(&mut vec);
        vec
    }

    pub fn similarity(&self, a: &[f32], b: &[f32]) -> f32 {
        cosine_similarity(a, b)
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    if len == 0 {
        return 0.0;
    }
    a.iter()
        .zip(b.iter())
        .take(len)
        .map(|(x, y)| x * y)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synonym_queries_match_auth_refresh_chunk() {
        let embedder = SemanticLocalEmbedding;
        let chunk = embedder.embed_text(
            "symbol: auth_refresh kind: function calls: fetch_token store_token \
             called_by: main excerpt: fn auth_refresh() { let token = fetch_token(); store_token(token); }",
        );
        let query = embedder.embed_text("credential renewal");
        let unrelated = embedder.embed_text("database migration schema");

        assert!(
            embedder.similarity(&query, &chunk) > embedder.similarity(&query, &unrelated),
            "credential renewal should match auth_refresh better than unrelated"
        );
    }

    #[test]
    fn similar_phrasing_scores_higher_than_unrelated() {
        let embedder = SemanticLocalEmbedding;
        let a = embedder.embed_text("auth refresh token");
        let b = embedder.embed_text("refresh auth token");
        let c = embedder.embed_text("unrelated database schema");
        assert!(embedder.similarity(&a, &b) > embedder.similarity(&a, &c));
    }

    #[test]
    fn expand_concepts_adds_related_terms() {
        let expanded = expand_concepts("credential renewal");
        assert!(expanded.contains("token") || expanded.contains("refresh"));
    }
}
