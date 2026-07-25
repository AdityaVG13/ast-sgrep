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
    let digest = blake3::hash(feature.as_bytes());
    let bytes = digest.as_bytes();
    // Use all 256 bits of the blake3 digest as independent sign bits: bit (i%8)
    // of byte (i/8). The previous `bytes[i % bytes.len()] & 1` only used the LSB
    // of each of the 32 bytes, tiling a 32-bit sign pattern 8x across the 256
    // slots and collapsing the effective dimension to 32 (bead ast-sgrep-e2hc.13).
    for (i, slot) in vec.iter_mut().enumerate() {
        let byte = bytes[i / 8];
        let sign_bit = (byte >> (i % 8)) & 1;
        *slot += if sign_bit == 0 { weight } else { -weight };
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
mod tests {
    use super::*;

    // Regression for bead ast-sgrep-e2hc.13: the old `bytes[i % 32] & 1` sign
    // extraction tiled a 32-bit sign pattern 8x across the 256 slots, so slots
    // [0..32] and [32..64] were always identical. The fix uses bit (i%8) of byte
    // (i/8), giving 256 independent sign bits. This test hashes a single feature
    // and asserts the two 32-slot windows differ, which only holds under the fix.
    #[test]
    fn hash_feature_uses_all_256_sign_bits_not_32_tiled() {
        let mut vec = vec![0.0_f32; SEMANTIC_DIM];
        hash_feature("feature-under-test", &mut vec, 1.0);
        let first_window: Vec<f32> = vec[0..32].to_vec();
        let second_window: Vec<f32> = vec[32..64].to_vec();
        assert_ne!(
            first_window, second_window,
            "slots [0..32] and [32..64] must differ; equality means the old 32-bit tiling bug is back"
        );
        // Stronger: across all 8 windows of 32 slots, at least two distinct
        // windows exist (the old bug produced 8 identical windows).
        let windows: Vec<&[f32]> = (0..8).map(|w| &vec[w * 32..(w + 1) * 32]).collect();
        let distinct = windows
            .iter()
            .enumerate()
            .any(|(i, w)| windows.iter().enumerate().any(|(j, o)| i != j && *w != *o));
        assert!(
            distinct,
            "at least two 32-slot windows must differ; the old bug made all 8 identical"
        );
    }

    // The fix must not change the output dimension or produce non-finite values.
    #[test]
    fn embed_text_produces_256_dim_normalized_vector() {
        let emb = SemanticLocalEmbedding;
        let v = emb.embed_text("auth refresh token");
        assert_eq!(v.len(), SEMANTIC_DIM);
        assert!(v.iter().all(|x| x.is_finite()));
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "vector must be normalized, got norm {norm}");
    }
}
