# Bench history (keep-gate SSoT)

Committed per-scenario `*.latest.json` files are the **only** keep-gate prior.
Local `.bench-history.json` (gitignored) is a scratch aggregate, not truth.

| File | Role |
|---|---|
| `thresholds.json` | Primary −3%, geomean −5%, `cv_pct > 5` quarantine |
| `<label>.latest.json` | Last **kept** (or placeholder) sample for that suite/query key |
| `<label>.run.json` | This-run candidate (gitignored); copy to `.latest.json` only after a keep |

Label keys match `asgrep bench` history labels (`suite:<fixture>:<suite>`,
`query:<text>`, `batch:<filename>`). Filename sanitizes `:` `/` to `-`.

## Keep rules

1. Compare current mean vs committed prior. Regression above
   `primary_regression_pct` (3) fails. Suite geomean regression above 5 fails.
2. `cv_pct > 5` → ineligible / quarantine (not a keep).
3. Placeholder or missing prior → **establish baseline**, not a win keep.
4. Record `host`, `git_sha`, `profile` on every decision.
5. Competitor latency (ast-grep, rg, UNREPRODUCIBLE ledgers) is **not** keep
   and **not** correctness. See `docs/progress/perf-negative-results.md`.
6. Claiming a **win** keep also requires a HotPath / profile sample (checklist;
   not every micro commit).

Overwrite `.latest.json` only with `ASGREP_BENCH_HISTORY_COMMIT=1` after the
gate passes on a labeled host. CI never sets that.

Disable the gate with `ASGREP_BENCH_RATCHET=0`. History compare is default-on.
