# Known losses

> **Published record** of measured results. No runnable harnesses ship in this tree.

Measured 2026-07-10 on Apple M5 Max, 48 GiB RAM. The corpus is ripgrep 14.1.1 at `4649aa9700619f94cf9c66876e9549d83420e16c`; the 14-query gold fixture is unchanged. Full machine, corpus, and tool provenance is in [`BASELINES.md`](baselines.md). Machine-readable aggregate and per-query results are under `bakeoff.corpora.ripgrep` in *(historical dump; not in-tree)*.

The default local neural plus cross-encoder pipeline reaches **0.605 MRR**, above the committed semgrep hand-pattern reference at **0.536 MRR**. This comparison is intentionally difficult for asgrep: semgrep receives a hand-authored structural pattern for each natural-language intent, while asgrep receives the natural-language query directly.

## Reproduce

```bash
cargo build --profile release-perf -p ast-sgrep-cli \
  --features neural-embed,rerank
ASGREP_NEURAL_EMBED=true ASGREP_RERANK=true \
ASGREP_RERANK_WEIGHT=20 ASGREP_RERANK_BATCH_SIZE=1 \
RAYON_NUM_THREADS=1 ASGREP_NEURAL_INTRA_THREADS=1 \
ASGREP_RERANK_INTRA_THREADS=1 \
    --bin target/release-perf/asgrep
```

The harness processes one query/tool subprocess at a time and writes every query result to `benchmarks/results.json`.

## All 14 ripgrep queries

`--` means the tool did not return a relevant result in the evaluated top 10. RR is reciprocal rank.

| query id | asgrep rank | asgrep RR | semgrep rank | semgrep RR | outcome |
|----------|------------:|----------:|-------------:|-----------:|---------|
| `rg_gitignore_impl` | 1 | 1.0000 | -- | 0.0000 | asgrep win |
| `rg_cli_parse` | 6 | 0.1667 | -- | 0.0000 | asgrep win |
| `rg_walker` | 1 | 1.0000 | 1 | 1.0000 | tie |
| `rg_search_core` | -- | 0.0000 | -- | 0.0000 | shared miss |
| `rg_regex_builder` | 1 | 1.0000 | 2 | 0.5000 | asgrep win |
| `rg_std_printer` | 10 | 0.1000 | 1 | 1.0000 | **asgrep loss** |
| `rg_json_output` | 2 | 0.5000 | 1 | 1.0000 | **asgrep loss** |
| `rg_glob_compile` | 1 | 1.0000 | 1 | 1.0000 | tie |
| `rg_decompress` | 1 | 1.0000 | 1 | 1.0000 | tie |
| `rg_file_types` | 1 | 1.0000 | 1 | 1.0000 | tie |
| `rg_main_entry` | 1 | 1.0000 | -- | 0.0000 | asgrep win |
| `rg_overrides` | 5 | 0.2000 | 1 | 1.0000 | **asgrep loss** |
| `rg_mmap_search` | 4 | 0.2500 | -- | 0.0000 | asgrep win |
| `rg_multi_line` | 4 | 0.2500 | -- | 0.0000 | asgrep win |

## Losses

- **`rg_std_printer`**: asgrep finds the correct printer path only at rank 10; semgrep's hand-authored printer pattern ranks it first. The cross-encoder recovers this query from a no-rerank miss, but not near the top.
- **`rg_json_output`**: asgrep ranks the correct JSON printer second; semgrep's exact structural pattern ranks it first.
- **`rg_overrides`**: asgrep ranks the override matcher fifth; semgrep's exact override pattern ranks it first.
- **`rg_search_core`**: both tools miss the relevant target in the top 10. This is the remaining shared retrieval failure.

No gold query, relevant target, corpus pin, or competitor pattern was changed to reach the gate. The gain over asgrep's 0.594 no-rerank A/B baseline comes from recovering `rg_std_printer` at rank 10 and moving `rg_mmap_search` from rank 6 to rank 4, partly offset by `rg_cli_parse` moving from rank 5 to rank 6.
