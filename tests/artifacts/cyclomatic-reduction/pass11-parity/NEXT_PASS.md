# NEXT_PASS.md

Run ID: `2026-08-11Tpass11-parity`  
Completed: **Pass 11 — Differential parity hardening + residual queue + scorecard narrative**  
Next: **Pass 12 — Absolute convergence scan (expect ZERO product change)**

## Pass 11 outcome

| Item | Result |
|---|---|
| Product files changed | **0** |
| Campaign ΣCC (frozen) | **5994** (−28 vs baseline) |
| Joint-allowed parity floors | **PASS** (see `parity-matrix.md`) |
| Pre-existing red | F1 pack inventory, F2 mode `keyword` — **not** campaign-caused |
| Residual packets D1–D3 | Hardened for independent agent |
| Scorecard narrative | `scorecard-final.md` |

## Pass 12 goals (absolute convergence)

**Default posture: ZERO product edits.**

1. **Convergence scan only**
   - Re-read `bill-summary.json` (pass 10) + `scorecard-final.md` + residual INDEX
   - Confirm no uncommitted campaign product edits required for parity
   - Optionally re-run the **same** targeted floors if branch drifted; do not invent workspace suite
2. **Optional tooling** (if present on skill PATH)
   - `validate_cut_branches.py` on final report
   - `score_scorecard.py` / `render_scorecard.py` — record real numbers; never invent ≥900
3. **Freeze residual**
   - Keep ledger stays Keep
   - D1–D3 remain Defer unless an agent **already** has a measured bill-negative shared-collapse plan **before** editing
   - Do **not** open micro-beads per hotspot
4. **Campaign RESULT**
   - Emit final `CUT_BRANCHES_RESULT: partial` (ceiling not cleared) **unless** authorized scope is redefined to “Keep/Defer only”
   - Only claim `complete` if every scoped hotspot is ≤ceiling **or** `blocked_with_reason`/`Keep` with evidence **and** user/authorized scope allows that definition

## Explicit non-goals for pass 12

- Funded transform wave by default
- Pure extract of resolveHost / run_process / regex_pass
- Fixing F1/F2 packaging/docs matrix (separate product work)
- `cargo test --workspace`
- Cutting Keep rows for vanity max-CC

## When pass 12 may touch product (exception only)

If and only if:

1. A D1–D3 packet shows an **obvious** duplicate decision tree, **and**
2. Pre-measure + post-measure prove −ΣCC on the packet gate, **and**
3. Pass 11 verify commands stay green,

…then a **minimal** shared-collapse is allowed.  
**Expectation: zero such cuts remain fundable** after passes 8–9 refuses.

## Artifacts to update in pass 12

- `NEXT_PASS.md` → `none` or residual permanent Keep note
- Final RESULT block
- Mirror under `tests/artifacts/cyclomatic-reduction/pass12-convergence/`
- Update `.cyclomatic-reduction/LATEST`

## Handoff paths

| Need | Path |
|---|---|
| Parity matrix | `parity-matrix.md` |
| Residual queue | `residual-queue-INDEX.md`, `work-queue/D1..D3` |
| Scorecard | `scorecard-final.md` |
| Bill numbers | `../2026-08-11Tpass10-bill/bill-summary.json` |
| Mirror | `tests/artifacts/cyclomatic-reduction/pass11-parity/` |
