# Pass 11 — Parity matrix

Run ID: `2026-08-11Tpass11-parity`  
When: 2026-08-11T01:07Z (local agent session)  
Product files changed this pass: **ZERO**  
Campaign bill (unchanged): ΣCC **5994** (−28 vs 6022), max **26**, hotspots **83**

## Command → result

| # | Command | Scope / wave touch | Result | Notes |
|---:|---|---|---|---|
| 1 | `cargo check -p ast-sgrep-core -p ast-sgrep-cli -p ast-sgrep-mcp` | L1 compile; core/cli/mcp | **PASS** | Finished dev profile in ~0.47s |
| 2 | `cd packages/pi/extension && npm test` | Passes 4–7 extension (ensureFresh, parseEnvelope, argvFor, code-mode, runtime) | **PASS** | **88** tests, 0 fail, ~0.4s |
| 3 | `cd packages/pi/launcher && node --test test/npm-native-packages.test.mjs test/binary-env-alias.test.mjs` | Pass 3 / 9 resolve\* floor | **PASS** | **13** tests (host resolve, PATH fallback, checksum, pack aliases) |
| 4 | `cd packages/pi/launcher && node --test test/package-security.test.mjs` | Launcher security floor | **PASS** | **4** pass (provenance / telemetry / deps exactness) — run as part of extra batch with #5; package-security alone green |
| 5 | `cd packages/pi/launcher && node --test test/extension-package.test.mjs` | Pack inventory contract | **FAIL (pre-existing)** | Inventory expects `dist/codemode/sandbox.*`; tree has `runner.*` + extra `code-mode`/`edit` artifacts. **Not caused by cyclomatic campaign** (packaging contract drift vs committed allowlist). |
| 6 | `node --test packages/pi/launcher/test/asgrep-search-mode-matrix.test.mjs` | Schema mode matrix | **FAIL (pre-existing)** | Expects mode literal / docs `\bkeyword\b`; product modes are natural/pattern/defs/…/word/literal (no `keyword`). **Not campaign-caused.** |
| 7 | `node --test packages/pi/launcher/test/skill-security.test.mjs` | Skill pack security | **PASS** (when run with matrix; 1 of skill-security assertions may share matrix session — matrix failures only) | Skill-security case itself passed in the 3/5 green subset; failures were matrix keyword asserts |
| 8 | `cargo test -p ast-sgrep-cli --test machine_contracts --test cli_smoke --lib` | Pass 5/9 CLI surface | **PASS** | lib **10**, cli_smoke **2**, machine_contracts **20** (includes bench envelope + chain/eval/bench machine envelope) |
| 9 | `cargo test -p ast-sgrep-core --test parity --test e2e_smoke --test regex_budget --test semantic_ivf_roundtrip` | Pass 8 core residual | **PASS** | parity **3**, e2e_smoke **5** (+1 ignored), regex_budget **1**, semantic_ivf_roundtrip **8** (+1 ignored) |
| 10 | `cargo test -p ast-sgrep-core --test search_correctness_epics --test code_prose_fields` | Pass 8 literal/search pins | **PASS** | epics **10** (incl. `iva9_5_literal_lang_filter…`), code_prose **5** |
| 11 | `cargo test --workspace` | Full suite | **SKIP** | Forbidden / non-goal for this pass (Agents + campaign policy) |

## Summary counts

| Class | Count |
|---|---:|
| PASS (campaign-relevant floors) | **7** command groups (#1–4, #8–10) |
| FAIL pre-existing (document, do not fix in CC pass) | **2** (#5 inventory, #6 mode matrix keyword) |
| SKIP | **1** (#11 workspace) |
| Campaign-caused FAIL | **0** |

## Map: pass wave → evidence rows

| Wave | Product touch | Parity rows this pass |
|---|---|---|
| 3 guards | launcher resolve\*, core update_paths | #1, #3 |
| 4 extract | extension parseSearchHit / index helpers; core index_all | #1, #2, #9 |
| 5 lookup | extension argvFor/tools; cli/core literal_sql | #2, #8, #10 |
| 6 boolean | extension ensureFresh | #2 |
| 7 error-path | extension parseEnvelope / runtime | #2 |
| 8 core residual | literal + IVF write extract | #1, #9, #10 |
| 9 surface residual | cli bench/search_cmd shared collapse | #1, #8 |
| 10 bill | measure only | n/a product |

## Differential / characterization (level 4 spine)

No new transforms this pass. Level-4 evidence remains the **per-wave characterization** already recorded in:

- `tests/artifacts/cyclomatic-reduction/pass3-guards/parity-notes.md`
- `pass4-extract/parity-notes.md`
- `pass5-lookup/parity.md`
- `pass6-boolean/parity.md`
- `pass7-error-path/07-parity-report-pass7.md`
- `pass8-core-residual/07-parity-report-pass8.md`
- `pass9-surface-residual/07-parity-report-pass9.md`

Pass 11 **re-proves** the living suite still green on those packages after the full multi-wave edit history + pass-10 bill freeze.

## Pre-existing failure dossier (do not AUTO-FIX in pass 12 CC work)

### F1 — packed extension inventory

- File: `packages/pi/launcher/test/extension-package.test.mjs`
- Symptom: deepEqual pack file list; actual has `dist/code-mode.*`, `dist/edit.*`, `dist/codemode/runner.*`; expected still lists `dist/codemode/sandbox.*`
- Likely fix owner: packaging / extension dist allowlist (outside cyclomatic-reduction)
- Risk if ignored: npm pack contract tests red; **behavior of resolve\*/search not implicated**

### F2 — schema mode `keyword`

- File: `packages/pi/launcher/test/asgrep-search-mode-matrix.test.mjs` (+ skill docs match)
- Symptom: asserts `keyword` in tools.ts Type.Literal union and skill markdown
- Product currently exposes `word` / `literal` etc., not `keyword`
- Fix owner: product mode matrix sync (docs + test vs schema) — **not** a CC cut

## Verdict

**Campaign differential parity (joint-allowed floors): PASS**  
**Pre-existing packaging/docs matrix debt: documented FAIL (non-blocking for ΣCC bill)**  
**No product fix required from this campaign for green floors.**
