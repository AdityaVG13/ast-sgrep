//! The single embedding engine: the native static table in
//! `static_engine`. There is no backend selection — every embed call in
//! the workspace goes through these two functions, so index-time and
//! query-time vectors always live in the same space by construction.
//!
//! Legacy stores record other `embed_backend` / `embed_model` meta
//! values (`semantic-v2`, `neural`, ...). Those strings are recognized
//! only by the store layer, which refuses or migrates them; this crate
//! never embeds with a retired engine.

use crate::static_engine::StaticEmbedder;

/// Embed one text with the native static engine. Infallible.
pub fn embed_text(text: &str) -> Vec<f32> {
    StaticEmbedder::new().embed(text)
}

/// Embed a batch of texts with the native static engine. Infallible.
pub fn embed_texts(texts: &[&str]) -> Vec<Vec<f32>> {
    StaticEmbedder::new().embed_batch(texts)
}
