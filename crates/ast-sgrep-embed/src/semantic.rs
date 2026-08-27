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
fn hash_feature_bytes(prefix: &[u8], feature: &[u8], vec: &mut [f32], weight: f32) {
    // Use BLAKE3 XOF so each dimension gets an independent bit. The previous
    // `digest[i % 32]` tiling made every vector period-32 (effective rank 32, not 256).
    // Prefix+feature is byte-identical to hashing `format!("{prefix}{feature}")`.
    let mut hasher = blake3::Hasher::new();
    hasher.update(prefix);
    hasher.update(feature);
    let mut bytes = [0u8; SEMANTIC_DIM];
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
            hash_feature_bytes(b"tok:", token.as_bytes(), &mut vec, 1.0);
        }
        // Same windows as the previous `char_trigrams` helper, without per-window
        // String allocations. Compact is lowercase alphanumeric, so 3-byte
        // windows are identical to `format!("tri:{tri}")` UTF-8.
        let compact: String = expanded
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        if compact.len() >= 3 {
            for window in compact.as_bytes().windows(3) {
                hash_feature_bytes(b"tri:", window, &mut vec, 0.35);
            }
        }
        normalize(&mut vec);
        vec
    }
    pub fn similarity(&self, a: &[f32], b: &[f32]) -> f32 {
        dot_similarity(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_feature_old(feature: &str, vec: &mut [f32], weight: f32) {
        let mut hasher = blake3::Hasher::new();
        hasher.update(feature.as_bytes());
        let mut bytes = vec![0u8; vec.len()];
        hasher.finalize_xof().fill(&mut bytes);
        for (slot, &b) in vec.iter_mut().zip(bytes.iter()) {
            *slot += if b & 1 == 0 { weight } else { -weight };
        }
    }

    fn embed_text_old(text: &str) -> Vec<f32> {
        let expanded = expand_concepts(text);
        let mut vec = vec![0.0_f32; SEMANTIC_DIM];
        for token in tokenize(&expanded) {
            hash_feature_old(&format!("tok:{token}"), &mut vec, 1.0);
        }
        let compact: String = expanded
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        if compact.len() >= 3 {
            for window in compact.as_bytes().windows(3) {
                hash_feature_old(
                    &format!("tri:{}", String::from_utf8_lossy(window)),
                    &mut vec,
                    0.35,
                );
            }
        }
        normalize(&mut vec);
        vec
    }

    #[test]
    fn alloc_free_hash_matches_format_concat_identity() {
        let embedder = SemanticLocalEmbedding;
        for q in [
            "credential renewal",
            "sanitize user input",
            "FooBar_baz",
            "a",
            "ab",
            "abc",
        ] {
            let fresh = embedder.embed_text(q);
            let old = embed_text_old(q);
            assert_eq!(fresh, old, "identity drift on {q:?}");
        }
    }

    #[test]
    fn embed_text_short_query_timing() {
        let embedder = SemanticLocalEmbedding;
        let q = "credential renewal variant 42";
        for _ in 0..20 {
            let _ = embedder.embed_text(q);
        }
        let start = std::time::Instant::now();
        const N: u32 = 200;
        for _ in 0..N {
            let _ = embedder.embed_text(q);
        }
        let us = start.elapsed().as_secs_f64() * 1.0e6 / f64::from(N);
        eprintln!("embed_text mean {us:.1} us over {N} runs of {q:?}");
    }
}
