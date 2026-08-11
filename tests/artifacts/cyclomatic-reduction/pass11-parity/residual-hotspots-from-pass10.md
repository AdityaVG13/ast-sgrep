# Residual hotspots — Pass 10 bill (CC > 10)

Full-scope re-measure: **83** functions with CC>10 (baseline 91).  
Labels from campaign ledger (passes 2–9) + Ashby Keep policy.  
**Top 20** are the residual queue head; Keep ledger for essential domain follows.

## Top 20 remaining (CC>10)

| # | CC | Function | File:line | Label | Rationale |
|---|---:|---|---|---|---|
| 1 | 26 | `resolveHost` | `packages/pi/launcher/src/index.js:136` | **Defer** | Pass-3 guards done; pure extract +6 Refuse; need shared-collapse only if funded |
| 2 | 25 | `read_header` | `crates/ast-sgrep-core/src/semantic_ivf.rs:412` | **Keep** | IVF format parser — requisite variety |
| 3 | 25 | `readLineWindow` | `packages/pi/extension/src/code-mode.ts:285` | **Keep** | Line-window protocol shape |
| 4 | 24 | `run_bench_suite` | `crates/ast-sgrep-cli/src/bench.rs:278` | **Defer** | Case-loop identity; pure extract Refuse pass 9 |
| 5 | 24 | `measure_semantic_ivf_open_p99` | `crates/ast-sgrep-core/src/bench_suite.rs:198` | **Defer** | Bench harness; extract only if Σ-funded |
| 6 | 22 | `read_clusters_bounded` | `crates/ast-sgrep-core/src/semantic_ann.rs:104` | **Keep** | ANN cluster I/O bounds |
| 7 | 21 | `apply_weighted_rrf` | `crates/ast-sgrep-core/src/fusion.rs:207` | **Keep** | Fusion ranking domain |
| 8 | 20 | `embed_pass_lazy_ivf` | `crates/ast-sgrep-core/src/search/passes/embed.rs:113` | **Defer** | Embed pass residual; shared helpers if duplicate gates found |
| 9 | 20 | `classify_native` | `crates/ast-sgrep-lang/src/pattern.rs:138` | **Keep** | Language pattern taxonomy |
| 10 | 19 | `refresh_lines_only` | `crates/ast-sgrep-core/src/store/sqlite.rs:657` | **Defer** | Store path; refuse vanity extract |
| 11 | 19 | `cached_pattern_signatures` | `crates/ast-sgrep-lang/src/signature.rs:15` | **Keep** | Signature domain |
| 12 | 18 | `measure_index_update` | `crates/ast-sgrep-core/src/pipeline_parts.rs:231` | **Defer** | Pipeline measure harness |
| 13 | 18 | `regex_pass` | `crates/ast-sgrep-core/src/search/passes/regex.rs:26` | **Defer** | Pass-8 multi-helper extract Refuse (+4); only shared collapse |
| 14 | 18 | `isValidHitShape` | `packages/pi/extension/src/code-mode.ts:174` | **Keep** | Hit-shape validator (pass-4 extract residual) |
| 15 | 18 | `resolveCodemodeAddon` | `packages/pi/launcher/src/index.js:231` | **Defer** | Same as resolveHost family |
| 16 | 17 | `save_semantic_ivf_with_publication` | `crates/ast-sgrep-core/src/semantic_ivf.rs:179` | **Keep**-lean | Validation residual after pass-8 write extract |
| 17 | 17 | `embed_url_is_allowed` | `crates/ast-sgrep-embed/src/embedder.rs:27` | **Keep** | Security allowlist — do not scatter |
| 18 | 17 | `parseEnvelope` | `packages/pi/extension/src/runtime.ts:431` | **Keep** | Protocol envelope residual (31→17) |
| 19 | 17 | `resolveBinary` | `packages/pi/launcher/src/index.js:198` | **Defer** | PATH fallback domain; thin residual |
| 20 | 16 | `run_bench_batch` | `crates/ast-sgrep-cli/src/bench.rs:460` | **Defer** | Batch top-10 packing |

## Essential Keep ledger (campaign — do not cut for score)

| Function | CC | Why Keep |
|---|---:|---|
| `read_header` / `write_header` / IVF map_and_parse residual | 25 / 11 / 11 | On-disk IVF format fidelity |
| `readLineWindow` | 25 | Window/protocol branches |
| `classify_native` | 20 | Lang taxonomy |
| `cached_pattern_signatures` / `required_pattern_literal` | 19 / 12 | Signature algebra |
| `apply_kind_rule` / extract name helpers | 15 / 11 | KindRule match arms = domain |
| `apply_weighted_rrf` / signal margins | 21 / 12 | Ranking variety |
| `read_clusters_bounded` / ANN load residual | 22 / 11 | ANN structure |
| `embed_url_is_allowed` | 17 | Security |
| `isValidHitShape` / `parseEnvelope` / `indexHealth` | 18 / 17 / 16 | Protocol / status shapes |
| `detect_language` / keyword_symbol_kind | 12 / 12 | Lang surface |
| Test oracles (`bead_vwga_*`, `graph_oracle_*`) | 13 / 12 | Test harness density — out of product Cut focus |

## Fundable Defer clusters (for pass 11+ / work-queue — not one bead per hotspot)

### D1 — Launcher resolve family
- `resolveHost` 26, `resolveCodemodeAddon` 18, `resolveBinary` 17  
- **Only** if a technique eliminates duplicate decision trees without +ΣCC (pass-9 pure extract +6 Refuse).

### D2 — CLI surface residual
- `run_bench_suite` 24, `run_bench_batch` 16, `run_process` 16, `run_chain`/`run_watch` 14, agent/eval 13–12  
- Prefer shared collapse over ladder extracts (`run_process` +3 Refuse).

### D3 — Core search / store residual
- `regex_pass` 18, `literal_sql` 15, `literal_trigram` 11, `embed_pass_lazy_ivf` 20, `update_paths` 15, sqlite refresh/upsert cluster  
- Pass-8 already refused walk/regex multi-helper dumps; only bill-negative shared collapses.

## Stats

- Residual CC>10: **83**
- Top-20 Keep count: **9** (incl. Keep-lean)
- Top-20 Defer count: **11**
- Under-hard-ceiling wins this campaign (examples): `ensureFresh` 10, `run_bench` 9, `run_search` 10, `index_all` 8, `parseSearchHit` 4, `delete_file_lines` 2, `write_ivf_temporary` 8
