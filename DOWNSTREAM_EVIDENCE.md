# DOWNSTREAM_EVIDENCE — PR #25 (`cursor/anti-bloat-cleanup-da21`)

Hard evidence for beads closed on this branch tip.
Commands assume cwd `/workspace/.worktrees/pr25`.
**Note:** `.beads/` was not modified (per task instruction).

---

| Bead | Evidence |
|------|----------|
| `ast-sgrep-gajb` | `AGENTS.md` § “Benchmark and published-number claims”: no bare quotes without baselines row or `UNREPRODUCIBLE` tag; harness path; negative ledger; no dual canonical figures. `docs/RELEASING.md` § “Honesty checklist”: no unreproducible README GATE; single fingerprint; negative ledger; optional harness job URL. |
| `ast-sgrep-micr` | `docs/getting-started.md` CLI reference now lists every shipped subcommand (`index`, `status`, `reindex`, `bench`, `watch`, `semantic`, `chain`, `capabilities`, `version`, `robot-docs`, `doctor`, `eval`, default search) and global flags including `--neural-embed`, `--ann-probes`, `--rerank*`, `--excerpt-lines`, `--format` aliases. Graph prefixes table includes `literal:`/`regex:`/`word:`; links `QUERY_GRAMMAR.md` + `capabilities --json`. |
| `ast-sgrep-vjpb` | `benchmarks/results/baselines.md` “Canonical fingerprint rows”: `rg-hybrid-default-d3eab74` = **0.290**; `rg-neural-rerank-d3eab74` = **0.605**; `self-hybrid-d3eab74` = **0.712**; historical ~0.75/`0.746` marked **SUPERSEDED**. `losses.md` / `bakeoff.md` / `head-to-head.md` label 0.605 as neural+rerank only. README drops dual ~0.75 current claim. |
| `ast-sgrep-jr5i` | `docs/semantic-search.md`: 256 is storage width; BLAKE3 sign period 32 honesty note. `docs/how-it-works.md`: forbid-list oracle, not 0% false-caller rate. `docs/comparison.md` + `getting-started.md`: remove ~0.3ms typical product claim; point at baselines / fixture-only context. |

## Validation

Docs-only on this tip; no Cargo behavioral change required for these beads.
