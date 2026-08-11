# Pass 7 — Error/Result path consolidation

Techniques: `extract_method` on failure ladders; accidental decision elimination on error-path predicates.
Scope: `packages/pi/extension/src/runtime.ts` only (no Rust).

## Transforms

### 1. `parseEnvelope` (primary) — CC 31 → **17**

Classification: essential protocol field checks **Keep**; accidental error-ladder density **Cut**.

| Change | Technique | Effect |
|---|---|---|
| Extract `throwNonzeroProcessFailure` | extract_method | Moves nonzero-exit try/parse/OPERATIONAL vs PROCESS_FAILED ladder out of success-path protocol checks |
| Limit check: `stdout + stderr > limit` only | combine_predicates | Byte lengths non-negative ⇒ single sum covers either-side overflow (was 3-way OR) |
| Object shape via `record(value)` | consolidate / reuse | Replaces `!value \|\| typeof !== "object" \|\| Array.isArray` (3 decisions → 1) |
| Protocol field chain stays inline | Ashby Keep | tool / schema_version / ok boolean / ok:false message varieties |

Rejected mid-pass:
- `throwOperationalFailure` + `rethrowExecFailure` alone (no decision elimination) → file ΣCC **+2** (function-base dump) — dropped until paired with real cuts.
- Standalone `isStructuredFailedEnvelope` helper → Σ +1 base; inlined into `throwNonzeroProcessFailure` using `record(JSON.parse(...))`.

### 2. `run` — CC 12 → **6**

Extract `rethrowExecFailure` (RuntimeError rethrow / CANCELLED / TIMEOUT / EXEC_FAILED ladder). Parent keeps argv + abort guards + exec happy path.

### 3. `rebuildIncompatibleIndex` — CC 11 → **7**

Extract `throwIndexRebuildFailed` (recoveryPath / priorIndexPreserved classification → INDEX_REBUILD_FAILED). Swap/rename success path stays in method.

## Explicit Keep / Refuse

| Item | Resolve | Why |
|---|---|---|
| tool / schema / ok protocol checks in `parseEnvelope` | **Keep** | Requisite wire-contract variety (Ashby); different error codes by field |
| Nonzero vs zero exit error-code split (PROCESS_FAILED vs MALFORMED/TOOL_MISMATCH) | **Keep** | Distinct agent-facing codes; do not unify into one parse path |
| `indexHealth` (16) | **Keep** | Domain health varieties (pass 6 residual) |
| `parseEditParams` (13) | **Defer** | Flat validation guards; not nested Result ladder — module residual pass 8/9 |
| Pure extract without decision elimination | **Refuse** | ΣCC rose +1..+2 (Kolmogorov dump) |

## Public API

Unchanged. Helpers are module-private. `AstSgrepRuntime.run` / `rebuildIncompatibleIndex` / envelope error codes and details preserved (parity via runtime tests).
