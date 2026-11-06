//! F11: `embed_texts` must not read the process-global `ASGREP_EMBED_TRACE` env var.
//!
//! KILLS: hidden env input in the batch embed path (H1), the speculative
//! debug sinks in a library hot path (S1), silent trace-sink loss (C1), and
//! the stringly-typed trace mini-language (C2). Fails before F11 (the trace
//! block appends vectors to the `file:` sink); passes after (process env is
//! never consulted — all config flows through `CoreConfig::from_env`).

use ast_sgrep_embed::embed_texts;

#[test]
fn embed_texts_ignores_embed_trace_env() {
    let prior = std::env::var("ASGREP_EMBED_TRACE").ok();
    let sink =
        std::env::temp_dir().join(format!("asgrep_f11_no_sink_{}.trace", std::process::id()));
    let _ = std::fs::remove_file(&sink);

    // `file:` sink arm: nothing may be created.
    std::env::set_var(
        "ASGREP_EMBED_TRACE",
        format!("file:{}", sink.display()),
    );
    let filed = embed_texts(&["hello world", "second line here"]);

    // `1` stderr arm: output must equal the un-traced run.
    std::env::set_var("ASGREP_EMBED_TRACE", "1");
    let flagged = embed_texts(&["hello world", "second line here"]);

    match prior {
        Some(value) => std::env::set_var("ASGREP_EMBED_TRACE", value),
        None => std::env::remove_var("ASGREP_EMBED_TRACE"),
    }

    let plain = embed_texts(&["hello world", "second line here"]);
    assert_eq!(filed, plain, "trace sink must not change vectors");
    assert_eq!(flagged, plain, "trace flag must not change vectors");
    assert!(
        !sink.exists(),
        "embed_texts must not create ASGREP_EMBED_TRACE sink {}",
        sink.display()
    );
    let _ = std::fs::remove_file(&sink);
}
