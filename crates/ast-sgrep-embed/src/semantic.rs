use crate::dot_similarity;
use std::collections::HashSet;
pub const SEMANTIC_DIM: usize = 256;
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
fn split_ident(ident: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    for ch in ident.chars() {
        if ch == '_' {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur).to_lowercase());
            }
            continue;
        }
        if ch.is_ascii_uppercase() && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur).to_lowercase());
        }
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
fn char_trigrams(text: &str) -> Vec<String> {
    let compact: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    if compact.len() < 3 {
        return vec![];
    }
    compact
        .as_bytes()
        .windows(3)
        .map(|w| String::from_utf8_lossy(w).into_owned())
        .collect()
}
fn hash_feature(feature: &str, vec: &mut [f32], weight: f32) {
    // Use BLAKE3 XOF so each dimension gets an independent bit. The previous
    // `digest[i % 32]` tiling made every vector period-32 (effective rank 32, not 256).
    let mut hasher = blake3::Hasher::new();
    hasher.update(feature.as_bytes());
    let mut bytes = vec![0u8; vec.len()];
    hasher.finalize_xof().fill(&mut bytes);
    for (slot, &b) in vec.iter_mut().zip(bytes.iter()) {
        *slot += if b & 1 == 0 { weight } else { -weight };
    }
}
fn normalize(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vec {
            *x /= norm;
        }
    }
}
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
        dot_similarity(a, b)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/embed/semantic__hash_rank_tests.rs"]
mod hash_rank_tests;
