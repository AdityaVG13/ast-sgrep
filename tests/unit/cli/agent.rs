use super::*;
use clap::Parser;

fn status_with_durability(durability: &str) -> ast_sgrep_core::IndexStatus {
    ast_sgrep_core::IndexStatus {
        root: "/tmp".into(),
        index_path: "/tmp/.asgrep/index.db".into(),
        file_count: 1,
        line_count: 1,
        symbol_count: 0,
        caller_count: 0,
        import_count: 0,
        semantic_chunk_count: 0,
        embed_backend: None,
        embed_dim: None,
        embed_cache_entries: 0,
        embed_cache_capacity: 0,
        embed_cache_hits: 0,
        embed_cache_misses: 0,
        semantic_ivf_present: false,
        durability: durability.into(),
        writer_generation: 0,
    }
}

#[test]
fn doctor_surfaces_fast_unsafe_from_status() {
    let cli = Cli::try_parse_from(["asgrep", "doctor", "."]).expect("parse");
    let issue = doctor_fast_unsafe_issue(&cli, Some(&status_with_durability("fast-unsafe")));
    assert_eq!(issue.as_ref().unwrap()["kind"], "durability_fast_unsafe");
}

#[test]
fn doctor_surfaces_fast_unsafe_from_cli_flag() {
    let cli = Cli::try_parse_from(["asgrep", "--durability", "fast-unsafe", "doctor", "."])
        .expect("parse");
    let issue = doctor_fast_unsafe_issue(&cli, Some(&status_with_durability("balanced")));
    assert_eq!(issue.as_ref().unwrap()["kind"], "durability_fast_unsafe");
}

#[test]
fn doctor_surfaces_silent_on_balanced() {
    let cli = Cli::try_parse_from(["asgrep", "doctor", "."]).expect("parse");
    assert!(
        doctor_fast_unsafe_issue(&cli, Some(&status_with_durability("balanced"))).is_none()
    );
}
