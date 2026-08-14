use super::*;
use crate::cli_args::{Cli, Commands, SearchTuning};
use clap::Parser;
use std::path::Path;

fn parse_search(args: &[&str]) -> Cli {
    Cli::try_parse_from(std::iter::once("asgrep").chain(args.iter().copied())).expect("parse")
}

fn search_cli_with(mut apply: impl FnMut(&mut SearchTuning)) -> Cli {
    let mut cli = parse_search(&["search", "q", "."]);
    apply(&mut cli.tuning);
    if let Some(Commands::Search(cmd)) = cli.command.as_mut() {
        apply(&mut cmd.tuning);
    }
    cli
}

fn assert_exclusive(opts: &SearchOptions, backend: EmbedBackend) {
    assert_eq!(opts.embed_backend(), backend);
    let (cloud, ollama, neural, semantic) = backend.to_flags();
    assert_eq!(opts.use_cloud_embed, cloud);
    assert_eq!(opts.use_ollama_embed, ollama);
    assert_eq!(opts.use_neural_embed, neural);
    assert_eq!(opts.use_semantic_only, semantic);
}

#[test]
fn search_options_collapses_concurrent_embed_flags_to_cloud() {
    let cli = search_cli_with(|t| {
        t.cloud_embed = true;
        t.ollama_embed = true;
        t.neural_embed = true;
        t.semantic_only = true;
    });
    assert_exclusive(&search_options(Path::new("."), &cli), EmbedBackend::Cloud);
}

#[test]
fn search_options_collapses_ollama_over_neural_and_semantic() {
    let cli = search_cli_with(|t| {
        t.cloud_embed = false;
        t.ollama_embed = true;
        t.neural_embed = true;
        t.semantic_only = true;
    });
    assert_exclusive(&search_options(Path::new("."), &cli), EmbedBackend::Ollama);
}

#[test]
fn search_options_collapses_neural_over_semantic() {
    let cli = search_cli_with(|t| {
        t.cloud_embed = false;
        t.ollama_embed = false;
        t.neural_embed = true;
        t.semantic_only = true;
    });
    assert_exclusive(&search_options(Path::new("."), &cli), EmbedBackend::Neural);
}

#[test]
fn search_options_semantic_only_is_exclusive() {
    let cli = search_cli_with(|t| {
        t.cloud_embed = false;
        t.ollama_embed = false;
        t.neural_embed = false;
        t.semantic_only = true;
    });
    assert_exclusive(
        &search_options(Path::new("."), &cli),
        EmbedBackend::Semantic,
    );
}

#[test]
fn search_options_no_embed_flags_are_auto() {
    let cli = search_cli_with(|t| {
        t.cloud_embed = false;
        t.ollama_embed = false;
        t.neural_embed = false;
        t.semantic_only = false;
    });
    assert_exclusive(&search_options(Path::new("."), &cli), EmbedBackend::Auto);
}

#[test]
fn search_options_collapses_parent_and_subcommand_flag_forms() {
    let parent = parse_search(&["--cloud-embed", "--ollama-embed", "search", "q", "."]);
    assert_exclusive(
        &search_options(Path::new("."), &parent),
        EmbedBackend::Cloud,
    );

    let sub = parse_search(&["search", "--cloud-embed", "--ollama-embed", "q", "."]);
    assert_exclusive(&search_options(Path::new("."), &sub), EmbedBackend::Cloud);
}
