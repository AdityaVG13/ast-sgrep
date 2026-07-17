# Offline bake-off

> **Reproducibility status:** Every numeric row in this report is a historical
> published value and is **unreproducible from this source tree**: the generating
> harnesses, raw corpora, and raw result artifacts are absent. The external
> artifact location is the [Speed benchmark workflow](https://github.com/AdityaVG13/ast-sgrep/actions/workflows/speed.yml).
> No retained artifact is identified there for these historical runs, so this
> link is a storage location, not evidence that a row can currently be regenerated.

> **Published record** of measured results. No runnable harnesses ship in this tree.

This report publishes the complete committed `bakeoff` section of *(historical dump; not in-tree)*, measured on **2026-07-10**. All effectiveness metrics use k=10. Only the `ripgrep` and `flask` corpora are present in that snapshot; no results are reported for `tokio`, `express`, or `self` because the committed bake-off contains none.

## Methodology and scope

Each tool run against the same gold queries across two foreign corpora. ast-grep and semgrep scores cover structurally-expressible queries (count reported). All metrics use k=10.

- **asgrep hybrid**: default hybrid retrieval under the local neural embedding and reranker environment shown below.
- **asgrep `--no-embed`**: embedding-disabled ablation.
- **embedding-only (vector baseline)**: asgrep `--semantic-only`; this is an embedding-only ablation, not a hosted vector service.
- **semgrep**: one hand-authored structural pattern per natural-language intent.
- **ripgrep (file order)**: best-effort query-term OR search, deduplicated and ranked in file traversal order; ripgrep is not a ranked retriever.
- **ast-grep structural**: one hand-authored structural pattern per intent, ranked in file traversal order.

A hosted-model baseline is deliberately out of scope: project policy requires this bake-off to run offline without credentials, network-dependent inference, provider drift, or usage charges. This report therefore makes no claim against hosted embedding or reranking APIs.

### Caveats

- Gold queries authored by ast-sgrep team (potential positive bias toward code-search tools).
- ast-grep and semgrep are structural pattern matchers; they cannot parse NL queries. Scores reflect best-effort structural patterns derived from each query's intent.
- ripgrep is not a ranked retrieval tool; scored with best-effort term-OR, file-order ranking.
- Single machine: Apple M5 Max, 48 GiB. Timing is informational only; not statistically rigorous.
- ast-grep and semgrep scores use file-traversal order (first match wins), same as ripgrep methodology.

Hardware recorded in the snapshot: `Apple M5 Max (18 cores, arm64), 48 GiB, macOS, APFS SSD`. Commit provenance: `b5bdf8a plus working tree`.

## Aggregate results

| corpus | queries | competitor | MRR | Recall@10 | nDCG@10 | mean wall ms |
|---|---:|---|---:|---:|---:|---:|
| ripgrep | 14 | asgrep hybrid | 0.605 | 0.929 | 0.684 | 211.1 |
| ripgrep | 14 | asgrep --no-embed | 0.375 | 0.607 | 0.430 | 194.4 |
| ripgrep | 14 | ripgrep (file order) | 0.000 | 0.000 | 0.000 | 11.1 |
| ripgrep | 14 | ast-grep structural | 0.179 | 0.214 | 0.188 | 28.5 |
| ripgrep | 14 | semgrep | 0.536 | 0.571 | 0.545 | 1051.8 |
| ripgrep | 14 | embedding-only (vector baseline) | 0.000 | 0.000 | 0.000 | 39.9 |
| flask | 15 | asgrep hybrid | 0.374 | 0.667 | 0.447 | 204.2 |
| flask | 15 | asgrep --no-embed | 0.276 | 0.667 | 0.372 | 171.8 |
| flask | 15 | ripgrep (file order) | 0.162 | 0.600 | 0.259 | 10.2 |
| flask | 15 | ast-grep structural | 0.967 | 1.000 | 0.975 | 15.7 |
| flask | 15 | semgrep | 0.033 | 0.067 | 0.042 | 987.9 |
| flask | 15 | embedding-only (vector baseline) | 0.000 | 0.000 | 0.000 | 20.8 |

## ripgrep: per-query results

`--` means no relevant result in the evaluated top 10. MRR (RR) is the per-query reciprocal-rank contribution to aggregate MRR.

| query id | competitor | rank | MRR (RR) | Recall@10 | nDCG@10 |
|---|---|---:|---:|---:|---:|
| `rg_gitignore_impl` | asgrep hybrid | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_gitignore_impl` | asgrep --no-embed | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_gitignore_impl` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_gitignore_impl` | ast-grep structural | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_gitignore_impl` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_gitignore_impl` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_cli_parse` | asgrep hybrid | 6 | 0.1667 | 1.0000 | 0.3562 |
| `rg_cli_parse` | asgrep --no-embed | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_cli_parse` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_cli_parse` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_cli_parse` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_cli_parse` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_walker` | asgrep hybrid | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_walker` | asgrep --no-embed | 2 | 0.5000 | 1.0000 | 0.6309 |
| `rg_walker` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_walker` | ast-grep structural | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_walker` | semgrep | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_walker` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_search_core` | asgrep hybrid | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_search_core` | asgrep --no-embed | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_search_core` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_search_core` | ast-grep structural | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_search_core` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_search_core` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_regex_builder` | asgrep hybrid | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_regex_builder` | asgrep --no-embed | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_regex_builder` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_regex_builder` | ast-grep structural | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_regex_builder` | semgrep | 2 | 0.5000 | 1.0000 | 0.6309 |
| `rg_regex_builder` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_std_printer` | asgrep hybrid | 10 | 0.1000 | 1.0000 | 0.2891 |
| `rg_std_printer` | asgrep --no-embed | 2 | 0.5000 | 1.0000 | 0.6309 |
| `rg_std_printer` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_std_printer` | ast-grep structural | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_std_printer` | semgrep | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_std_printer` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_json_output` | asgrep hybrid | 2 | 0.5000 | 1.0000 | 0.6309 |
| `rg_json_output` | asgrep --no-embed | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_json_output` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_json_output` | ast-grep structural | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_json_output` | semgrep | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_json_output` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_glob_compile` | asgrep hybrid | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_glob_compile` | asgrep --no-embed | 2 | 0.5000 | 1.0000 | 0.6309 |
| `rg_glob_compile` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_glob_compile` | ast-grep structural | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_glob_compile` | semgrep | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_glob_compile` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_decompress` | asgrep hybrid | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_decompress` | asgrep --no-embed | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_decompress` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_decompress` | ast-grep structural | 2 | 0.5000 | 1.0000 | 0.6309 |
| `rg_decompress` | semgrep | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_decompress` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_file_types` | asgrep hybrid | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_file_types` | asgrep --no-embed | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_file_types` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_file_types` | ast-grep structural | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_file_types` | semgrep | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_file_types` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_main_entry` | asgrep hybrid | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_main_entry` | asgrep --no-embed | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_main_entry` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_main_entry` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_main_entry` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_main_entry` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_overrides` | asgrep hybrid | 5 | 0.2000 | 1.0000 | 0.3869 |
| `rg_overrides` | asgrep --no-embed | 4 | 0.2500 | 1.0000 | 0.4307 |
| `rg_overrides` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_overrides` | ast-grep structural | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_overrides` | semgrep | 1 | 1.0000 | 1.0000 | 1.0000 |
| `rg_overrides` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_mmap_search` | asgrep hybrid | 4 | 0.2500 | 1.0000 | 0.4825 |
| `rg_mmap_search` | asgrep --no-embed | 4 | 0.2500 | 0.5000 | 0.2641 |
| `rg_mmap_search` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_mmap_search` | ast-grep structural | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_mmap_search` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_mmap_search` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_multi_line` | asgrep hybrid | 4 | 0.2500 | 1.0000 | 0.4307 |
| `rg_multi_line` | asgrep --no-embed | 4 | 0.2500 | 1.0000 | 0.4307 |
| `rg_multi_line` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_multi_line` | ast-grep structural | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_multi_line` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `rg_multi_line` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |

## flask: per-query results

`--` means no relevant result in the evaluated top 10. MRR (RR) is the per-query reciprocal-rank contribution to aggregate MRR.

| query id | competitor | rank | MRR (RR) | Recall@10 | nDCG@10 |
|---|---|---:|---:|---:|---:|
| `fl_routing_dispatch` | asgrep hybrid | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_routing_dispatch` | asgrep --no-embed | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_routing_dispatch` | ripgrep (file order) | 7 | 0.1429 | 1.0000 | 0.3333 |
| `fl_routing_dispatch` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_routing_dispatch` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_routing_dispatch` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_blueprint` | asgrep hybrid | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_blueprint` | asgrep --no-embed | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_blueprint` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_blueprint` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_blueprint` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_blueprint` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_session_cookie` | asgrep hybrid | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_session_cookie` | asgrep --no-embed | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_session_cookie` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_session_cookie` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_session_cookie` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_session_cookie` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_templates` | asgrep hybrid | 5 | 0.2000 | 1.0000 | 0.3869 |
| `fl_templates` | asgrep --no-embed | 4 | 0.2500 | 1.0000 | 0.4307 |
| `fl_templates` | ripgrep (file order) | 8 | 0.1250 | 1.0000 | 0.3155 |
| `fl_templates` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_templates` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_templates` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_cli_run` | asgrep hybrid | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_cli_run` | asgrep --no-embed | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_cli_run` | ripgrep (file order) | 9 | 0.1111 | 1.0000 | 0.3010 |
| `fl_cli_run` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_cli_run` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_cli_run` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_config_load` | asgrep hybrid | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_config_load` | asgrep --no-embed | 3 | 0.3333 | 1.0000 | 0.5000 |
| `fl_config_load` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_config_load` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_config_load` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_config_load` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_app_context` | asgrep hybrid | 2 | 0.5000 | 1.0000 | 0.6309 |
| `fl_app_context` | asgrep --no-embed | 2 | 0.5000 | 1.0000 | 0.6309 |
| `fl_app_context` | ripgrep (file order) | 5 | 0.2000 | 1.0000 | 0.3869 |
| `fl_app_context` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_app_context` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_app_context` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_json_provider` | asgrep hybrid | 3 | 0.3333 | 1.0000 | 0.5000 |
| `fl_json_provider` | asgrep --no-embed | 3 | 0.3333 | 1.0000 | 0.5000 |
| `fl_json_provider` | ripgrep (file order) | 8 | 0.1250 | 1.0000 | 0.3155 |
| `fl_json_provider` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_json_provider` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_json_provider` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_signals` | asgrep hybrid | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_signals` | asgrep --no-embed | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_signals` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_signals` | ast-grep structural | 2 | 0.5000 | 1.0000 | 0.6309 |
| `fl_signals` | semgrep | 2 | 0.5000 | 1.0000 | 0.6309 |
| `fl_signals` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_class_views` | asgrep hybrid | 4 | 0.2500 | 1.0000 | 0.4307 |
| `fl_class_views` | asgrep --no-embed | 4 | 0.2500 | 1.0000 | 0.4307 |
| `fl_class_views` | ripgrep (file order) | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_class_views` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_class_views` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_class_views` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_helpers` | asgrep hybrid | 3 | 0.3333 | 1.0000 | 0.5000 |
| `fl_helpers` | asgrep --no-embed | 3 | 0.3333 | 1.0000 | 0.5000 |
| `fl_helpers` | ripgrep (file order) | 2 | 0.5000 | 1.0000 | 0.6309 |
| `fl_helpers` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_helpers` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_helpers` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_wrappers` | asgrep hybrid | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_wrappers` | asgrep --no-embed | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_wrappers` | ripgrep (file order) | 8 | 0.1250 | 1.0000 | 0.3155 |
| `fl_wrappers` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_wrappers` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_wrappers` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_sansio_app` | asgrep hybrid | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_sansio_app` | asgrep --no-embed | 7 | 0.1429 | 1.0000 | 0.3333 |
| `fl_sansio_app` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_sansio_app` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_sansio_app` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_sansio_app` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_scaffold_url_rules` | asgrep hybrid | 2 | 0.5000 | 1.0000 | 0.6309 |
| `fl_scaffold_url_rules` | asgrep --no-embed | 2 | 0.5000 | 1.0000 | 0.6309 |
| `fl_scaffold_url_rules` | ripgrep (file order) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_scaffold_url_rules` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_scaffold_url_rules` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_scaffold_url_rules` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_json_tag` | asgrep hybrid | 2 | 0.5000 | 1.0000 | 0.6309 |
| `fl_json_tag` | asgrep --no-embed | 2 | 0.5000 | 1.0000 | 0.6309 |
| `fl_json_tag` | ripgrep (file order) | 10 | 0.1000 | 1.0000 | 0.2891 |
| `fl_json_tag` | ast-grep structural | 1 | 1.0000 | 1.0000 | 1.0000 |
| `fl_json_tag` | semgrep | -- | 0.0000 | 0.0000 | 0.0000 |
| `fl_json_tag` | embedding-only (vector baseline) | -- | 0.0000 | 0.0000 | 0.0000 |

## Losses

The detailed narrative for the published ripgrep comparison is [`losses.md`](losses.md). It records the same 14 committed queries and discusses `rg_std_printer`, `rg_json_output`, `rg_overrides`, and the shared `rg_search_core` miss.

The table below lists every committed per-corpus query where asgrep hybrid has lower reciprocal rank than a named competitor. Equal scores and shared misses are not losses.

| corpus | query id | competitor | asgrep hybrid RR | competitor RR |
|---|---|---|---:|---:|
| ripgrep | `rg_cli_parse` | asgrep --no-embed | 0.1667 | 1.0000 |
| ripgrep | `rg_std_printer` | asgrep --no-embed | 0.1000 | 0.5000 |
| ripgrep | `rg_overrides` | asgrep --no-embed | 0.2000 | 0.2500 |
| ripgrep | `rg_cli_parse` | ast-grep structural | 0.1667 | 1.0000 |
| ripgrep | `rg_std_printer` | semgrep | 0.1000 | 1.0000 |
| ripgrep | `rg_json_output` | semgrep | 0.5000 | 1.0000 |
| ripgrep | `rg_overrides` | semgrep | 0.2000 | 1.0000 |
| flask | `fl_templates` | asgrep --no-embed | 0.2000 | 0.2500 |
| flask | `fl_sansio_app` | asgrep --no-embed | 0.0000 | 0.1429 |
| flask | `fl_routing_dispatch` | ripgrep (file order) | 0.0000 | 0.1429 |
| flask | `fl_class_views` | ripgrep (file order) | 0.2500 | 1.0000 |
| flask | `fl_helpers` | ripgrep (file order) | 0.3333 | 0.5000 |
| flask | `fl_wrappers` | ripgrep (file order) | 0.0000 | 0.1250 |
| flask | `fl_routing_dispatch` | ast-grep structural | 0.0000 | 1.0000 |
| flask | `fl_blueprint` | ast-grep structural | 0.0000 | 1.0000 |
| flask | `fl_templates` | ast-grep structural | 0.2000 | 1.0000 |
| flask | `fl_app_context` | ast-grep structural | 0.5000 | 1.0000 |
| flask | `fl_json_provider` | ast-grep structural | 0.3333 | 1.0000 |
| flask | `fl_signals` | ast-grep structural | 0.0000 | 0.5000 |
| flask | `fl_class_views` | ast-grep structural | 0.2500 | 1.0000 |
| flask | `fl_helpers` | ast-grep structural | 0.3333 | 1.0000 |
| flask | `fl_wrappers` | ast-grep structural | 0.0000 | 1.0000 |
| flask | `fl_sansio_app` | ast-grep structural | 0.0000 | 1.0000 |
| flask | `fl_scaffold_url_rules` | ast-grep structural | 0.5000 | 1.0000 |
| flask | `fl_json_tag` | ast-grep structural | 0.5000 | 1.0000 |
| flask | `fl_signals` | semgrep | 0.0000 | 0.5000 |

## Reproduce

The corpus restoration script reads `corpora.toml` and checks out its pins; compare the resulting `corpora.lock` with the committed lock. The harness runs one query/tool subprocess at a time and writes aggregate and per-query records to `benchmarks/results.json`.

```bash
cargo build --profile release-perf -p ast-sgrep-cli --features neural-embed,rerank
ASGREP_NEURAL_EMBED=true ASGREP_RERANK=true ASGREP_RERANK_WEIGHT=20 ASGREP_RERANK_BATCH_SIZE=1 RAYON_NUM_THREADS=1 ASGREP_NEURAL_INTRA_THREADS=1 ASGREP_RERANK_INTRA_THREADS=1 ```
