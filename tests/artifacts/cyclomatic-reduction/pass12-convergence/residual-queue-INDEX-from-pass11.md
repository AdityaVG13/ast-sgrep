# Residual queue INDEX — Pass 11

queue_adapter: markdown  
packets_dir: `work-queue/`  
run_id: `2026-08-11Tpass11-parity`  
prior_bill: `2026-08-11Tpass10-bill`  
policy: **Do not flood beads** (repo already has 50+ gauntlet/perf/MCT issues). Markdown packets are canonical for cyclomatic residual.

## Campaign freeze numbers (do not invent)

| Metric | Value | Provenance |
|---|---:|---|
| ΣCC | **5994** | pass10 `bill-summary.json` |
| Max CC | **26** | same |
| Hotspots CC>10 | **83** | same |
| ΔΣCC vs baseline | **−28** | 6022 → 5994 |

## How to claim (no beads)

1. Pick a row with `status=open` and no open deps.
2. Open **only** that packet file + the listed source paths.
3. Implement **only** Allowed techniques; re-measure before claiming cut.
4. Run Verify commands; if ΣCC rises → **Refuse**, revert, mark residual Keep.
5. Update this INDEX status when done.

Independence test: second agent, packet + repo only, no author chat.

## Queue table

| id | pri | kind | risk | title | packet | depends | status | external_id |
|----|-----|------|------|-------|--------|---------|--------|-------------|
| D1 | 2 | cc-cut | med | Launcher resolve\* shared-collapse only | [work-queue/D1-launcher-resolve.md](work-queue/D1-launcher-resolve.md) | — | **open / Defer** | — |
| D2 | 2 | cc-cut | med | CLI surface residual (bench/process/chain) | [work-queue/D2-cli-surface.md](work-queue/D2-cli-surface.md) | — | **open / Defer** | — |
| D3 | 2 | cc-cut | high | Core search/store residual | [work-queue/D3-core-search-store.md](work-queue/D3-core-search-store.md) | — | **open / Defer** | — |
| K* | — | keep | — | Essential domain ledger (do not cut for score) | see Keep section | — | **frozen Keep** | — |
| F1 | 3 | packaging | low | extension pack inventory drift | documented in `parity-matrix.md` | — | **out of CC scope** | — |
| F2 | 3 | product-sync | low | mode matrix `keyword` drift | documented in `parity-matrix.md` | — | **out of CC scope** | — |

## Keep ledger (frozen — pass 12 must not cut for score)

From pass-10 residual top-20 + essential table. Full list: prior run `residual-hotspots.md`.

| Function | CC | Why Keep |
|---|---:|---|
| `read_header` | 25 | IVF on-disk format parser |
| `readLineWindow` | 25 | Line-window protocol |
| `read_clusters_bounded` | 22 | ANN cluster I/O bounds |
| `apply_weighted_rrf` | 21 | Fusion ranking domain |
| `classify_native` | 20 | Lang pattern taxonomy |
| `cached_pattern_signatures` | 19 | Signature algebra |
| `isValidHitShape` | 18 | Hit-shape validator residual |
| `save_semantic_ivf_with_publication` residual | 17 | IVF validation |
| `embed_url_is_allowed` | 17 | Security allowlist |
| `parseEnvelope` residual | 17 | Protocol envelope (31→17 done) |
| KindRule / detect_language / keyword_symbol_kind | 11–15 | Domain match arms |

## Fundable Defer only (D1–D3)

| Cluster | Head functions | Gate |
|---|---|---|
| D1 | resolveHost 26, resolveCodemodeAddon 18, resolveBinary 17 | File ΣCC ≤ 102; shared-collapse only |
| D2 | run_bench_suite 24, run_bench_batch 16, run_process 16, run_chain/watch 14 | Package ΣCC non-increase; no ladder extract |
| D3 | embed_pass_lazy_ivf 20, refresh_lines_only 19, regex_pass 18, literal_sql 15 | Core total_cc non-increase; no walk/regex dump |

## Pass 11 hygiene done

- [x] Packets expanded with paths, refuse history, exact verify cmds, stop conditions
- [x] INDEX independence-ready
- [x] Pre-existing test failures filed as F1/F2 (not CC packets)
- [x] Pass 12 default: **ZERO product change** convergence scan unless a D-packet is funded with a real bill-negative technique

## Pass 12 instruction (absolute)

**Default: ZERO product edits.** Re-read bill + this INDEX; optionally re-run scorecard/validate scripts; emit final campaign RESULT. Only open a transform if an agent discovers a **proven** shared-collapse with pre-measure / post-measure −ΣCC. Expect residual 83 hotspots to remain as Keep/Defer — campaign stays `partial` unless authorized scope ceiling is redefined.
