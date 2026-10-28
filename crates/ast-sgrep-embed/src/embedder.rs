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
    let out = ENGINE.embed_batch(texts);
    if let Ok(spec) = std::env::var("ASGREP_EMBED_TRACE") {
        if spec == "1" {
            let flat = |text: &str| {
                text.chars()
                    .map(|c| if c.is_whitespace() { ' ' } else { c })
                    .collect::<String>()
            };
            for (i, text) in texts.iter().enumerate() {
                eprintln!("[embed-trace] job {i} len={} {}", text.len(), flat(text));
            }
        } else if let Some(path) = spec.strip_prefix("file:") {
            use std::io::Write as _;
            let esc = |t: &str| {
                t.replace('\\', "\\\\")
                    .replace('\n', "\\n")
                    .replace('\t', "\\t")
            };
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                for (text, vec) in texts.iter().zip(out.iter()) {
                    let head = vec
                        .iter()
                        .take(6)
                        .map(|v| format!("{v:e}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    let _ = writeln!(f, "{}\u{1}{}", esc(text), head);
                }
            }
        }
    }
    out
}
