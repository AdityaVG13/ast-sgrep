# Classification tallies (top 30)

Run: `2026-08-10T235424Z-baseline` · Pass 2

| Classification | Count |
|---|---|
| `essential_domain` | 11 |
| `accidental_structure` | 5 |
| `extractable` | 14 |
| `dead_path` | 0 |
| **Total** | 30 |

## Resolve summary

| Resolve | Count |
|---|---|
| Keep | 11 |
| Cut / Cut (extract…) | 19 |

## Wave hints (Cut candidates only)

| Wave | Focus | Functions |
|---|---|---|
| 3 | guard_clause / early_return | resolveHost, resolveBinary, resolveCodemodeAddon, update_paths |
| 4 | extract_method | run_bench_suite, save_semantic_ivf_with_publication, measure_semantic_ivf_open_p99, index_all, parseSearchHit, index_content_at, read_node, run_codemode_batch, measure_index_update, regex_pass, delete_file_lines, run_bench_batch |
| 5 | lookup_table | argvFor, searchToolCall, literal_sql |
| 6 | boolean / decompose (optional Keep helpers) | ensureFresh (optional) |
| 7 | error-path extract (optional on Keep) | parseEnvelope (optional) |
