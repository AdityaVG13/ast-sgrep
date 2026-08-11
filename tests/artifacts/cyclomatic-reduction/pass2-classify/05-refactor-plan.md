# 05 — Refactor plan (passes 3–7)

Run: `2026-08-10T235424Z-baseline`
Mode: classify complete for top 30; **no product edits in pass 2**.
North Star: lower **ΣCC** with exact behavior; Ashby Keep for domain; Kolmogorov Bill on extracts.

## Constraints

- Stay on `perf/software-optimization` until orchestrator says otherwise.
- No public API / wire-schema / ranking-order changes.
- After each wave: re-measure scope; ΣCC must not rise (or justified displacement).
- Prefer joint-allowed targeted tests over full-suite sweeps.

## Tallies (top 30)

- essential_domain (Keep): **11**
- accidental_structure (Cut): **5**
- extractable (Cut extract): **14**
- dead_path: **0**

---

## Pass 3 — Guard-clause wave (bounded)

**Goal:** Flatten nested try/catch and skip ladders without removing domain checks.

| Priority | Function | File | CC | Technique | Notes |
|---|---|---|---|---|---|
| P0 | `resolveHost` | `packages/pi/launcher/src/index.js` | 29 | guard_clause | Early fail after small readJson helper; keep HOSTS table |
| P0 | `resolveBinary` | same | 22 | early_return | `isPathFallbackError`; share checksum helper |
| P0 | `resolveCodemodeAddon` | same | 23 | guard_clause | Soft-null codes + early returns |
| P1 | `update_paths` | `crates/ast-sgrep-core/src/index.rs` | 18 | guard_clause | Extract `should_skip_watch_path` |

**Out of wave 3:** essential Keep list (fusion, IVF parse, pattern DSL, URL allowlist, ensureFresh).

**Parity focus:** launcher unit tests / binary resolution fixtures; watch update path tests.

**Exit:** re-lizard launcher + index.rs; cards updated with after-CC; ΣCC ≤ 6022.

---

## Pass 4 — Extract-method wave (product first)

Bounded product batch (max ~6 functions per session):

| Order | Function | CC | Extract targets |
|---|---|---|---|
| 1 | `index_all` | 23 | `commit_prepared_files`, `post_index_hooks` |
| 2 | `index_content_at` | 20 | `try_structure_skip_refresh` |
| 3 | `delete_file_lines` | 18 | ordered multi-table delete helper |
| 4 | `run_codemode_batch` | 19 | `load_batch_raw`, `apply_cli_batch_defaults` |
| 5 | `parseSearchHit` | 21 | `isValidHitShape` |
| 6 | `read_node` | 20 | `scan_line_window` only (keep TOCTOU inline) |

**Later pass-4 session (bench / lower urgency):**

| Function | CC | Note |
|---|---|---|
| `run_bench_suite` | 29 | extract case + report |
| `run_bench_batch` | 16 | extract per-query |
| `measure_semantic_ivf_open_p99` | 24 | sample loops |
| `measure_index_update` | 18 | reuse `time_loop` |
| `save_semantic_ivf_with_publication` | 25 | write temp body |
| `regex_pass` | 18 | worker join helper |

---

## Pass 5 — Lookup-table wave

| Function | CC | Table shape |
|---|---|---|
| `argvFor` | 22 | tool → argv builder |
| `searchToolCall` | 17 | SearchMode → [tool, args] |
| `literal_sql` | 18 | (case_insensitive, has_lang) → SQL template |

Preserve default/exhaustiveness behavior and SQL escaping.

---

## Pass 6 — Boolean / decompose (optional Keep helpers)

| Function | CC | Action |
|---|---|---|
| `ensureFresh` | 23 | Named predicates + `runIndex(force)` only if still hotspot after 3–5; **do not remove health varieties** |

---

## Pass 7 — Error-path extracts

| Function | CC | Action |
|---|---|---|
| `parseEnvelope` | 31 | Optional extract of failed-envelope path; **Keep** field validation chain |
| launcher residuals | — | Any leftover catch ladders after pass 3 |

---

## Explicit Keep (do not Cut for metric)

| Function | Why |
|---|---|
| `read_header`, `read_clusters_bounded` | IVF binary format |
| `apply_weighted_rrf` | ranking determinism |
| `classify_native`, `cached_pattern_signatures` | pattern DSL / signature identity |
| `refresh_lines_only` | index integrity |
| `embed_pass_lazy_ivf` | already guard-shaped channel gates |
| `embed_url_is_allowed` | SSRF policy |
| `readLineWindow` | safe streaming read product |
| `parseEnvelope` (core fields) | machine protocol |

---

## Deferred policy

- Ranks 31–91: classify after first transform waves.
- Bench ΣCC carve-out: still open; until decided, bench Cuts count toward Bill.

## Success criteria per wave

1. Analysis technique applied as named.
2. Targeted/joint-allowed tests green.
3. Differential parity where public outputs exist.
4. Re-measure: function CC down; **scope ΣCC not up**.
5. Ledger + `NEXT_PASS.md` updated.
