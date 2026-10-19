//! The single embedding engine: the native static table in
//! `static_engine`. There is no backend selection — every embed call in
//! the workspace goes through these two functions, so index-time and
//! query-time vectors always live in the same space by construction.
//!
//! Legacy stores record other `embed_backend` / `embed_model` meta
//! values (`semantic-v2`, `neural`, ...). Those strings are recognized
//! only by the store layer, which refuses or migrates them; this crate
//! never embeds with a retired engine.

use std::sync::LazyLock;

use crate::static_engine::StaticEmbedder;

static ENGINE: LazyLock<StaticEmbedder> = LazyLock::new(StaticEmbedder::new);

/// Embed one text with the native static engine. Infallible.
pub fn embed_text(text: &str) -> Vec<f32> {
    ENGINE.embed(text)
}

/// Embed a batch of texts with the native static engine. Infallible.
pub fn embed_texts(texts: &[&str]) -> Vec<Vec<f32>> {
    ENGINE.embed_batch(texts)
}
