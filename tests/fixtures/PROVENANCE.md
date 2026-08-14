# Test fixture provenance

Living registry for checked-in test artifacts. Golden compare/update SOP:
[`docs/validation/golden-files.md`](../../docs/validation/golden-files.md) and
[`tests/golden/PROVENANCE.md`](../golden/PROVENANCE.md).

**Not fixtures:** [`benchmarks/results/baselines.md`](../../benchmarks/results/baselines.md)
is an honesty ledger, not a CI golden. Temp indexes under `**/.asgrep/` are
gitignored. Do not commit `*.actual`.

Each row: purpose, how to regenerate, last-updated discipline, scrub notes.

## Ranking / sample

| Artifact | Purpose | Generator | Discipline | Scrub |
|---|---|---|---|---|
| `tests/fixtures/sample/` | Shared indexed corpus (`process_request`, `auth_refresh`, …) | hand-authored | Edit source; re-run ranking/CLI goldens | n/a |
| `tests/fixtures/ranking/cases.json` | must_include bag (`DISC-ranking-soft-oracle`) | hand-authored | Not a gold rank vector / MRR | n/a |

## CLI / plugin / protocol goldens

Regenerate with `ASGREP_UPDATE_GOLDENS=1` and targeted tests. Never in CI.
See `tests/golden/PROVENANCE.md` for per-file command + scrub.

| Artifact | Purpose |
|---|---|
| `tests/cli/fixtures/*.json`, `robot_guide.md` | Machine envelopes, search dumps, teaching, handbook |
| `tests/plugins/fixtures/*_sample.json` | Formatter dumps |
| `tests/mcp/fixtures/`, `tests/codemode/fixtures/` | MCP initialize/tools/list; catalog adapters |

## Lang extraction

| Artifact | Purpose | Generator | Discipline | Scrub |
|---|---|---|---|---|
| `tests/lang/fixtures/extract/*` | Immutable parse inputs (13 langs) | hand-authored | Reformatting requires dump refresh | n/a |
| `tests/lang/fixtures/extract_dumps/{lang}.json` | Full extraction dumps (nz7i.4) | `cargo test -p ast-sgrep-lang --test extraction_goldens` with `ASGREP_UPDATE_GOLDENS=1` | Extra symbols / kind-name drift fail dump compare; presence/forbid tuples stay in `assert_language_conformance` (`DISC-extraction-presence-only`) | sort only (`canonicalize_extraction`) |

Grammar pin (Cargo.lock, freeze date 2026-08-13): tree-sitter 0.26.10; rust 0.24.2;
typescript 0.23.2; javascript 0.25.0; python 0.25.0; go 0.25.0; java 0.23.5;
c-sharp 0.23.5; ruby 0.23.1; swift 0.7.3; c 0.24.2; cpp 0.23.4; kotlin-ng 1.1.0;
php 0.24.2.

Presence tuples graduate to dumps by calling `canonicalize_extraction` on the
conformance result and `assert_golden_json_at`. Do not reimplement scrub/compare.

## IVF frames (VERSION=2, magic `ASIVF\0`)

| Artifact | Purpose | Generator | Discipline | Scrub |
|---|---|---|---|---|
| `tests/fixtures/ivf/good_v2.ivf` | Tiny dim=4 / 4-chunk valid sidecar | `ASGREP_UPDATE_GOLDENS=1 cargo test -p ast-sgrep-core --test semantic_ivf_roundtrip committed_v2_frame` | Format break → new DISC + fixture | none |
| `tests/fixtures/ivf/bad_magic.ivf` | Reject: first byte flipped | same | fail-closed, no panic | none |
| `tests/fixtures/ivf/truncated.ivf` | Reject: last 4 bytes dropped | same | fail-closed, no panic | none |

Fingerprint: `compute_ann_fingerprint(4, 4, 4, Some("fixture"), 0)`. Vectors:
`i * 0.25` for `i in 0..16`. Adaptive ANN recall is `DISC-ivf-adaptive-threshold`.

## Schema migration DBs

Current `SCHEMA_VERSION` is **9** (not 7). Recreate:

```bash
python3 tests/fixtures/migration/build_legacy.py
```

Tests copy the file to a temp path before open so the committed bytes stay
immutable.

| Artifact | Purpose | user_version |
|---|---|---|
| `tests/fixtures/migration/v5_empty.sqlite` | Pre-v7 semantic-layout + later FTS/lexicon migrations | 5 |
| `tests/fixtures/migration/v99_unsupported.sqlite` | Newer-than-supported fail-closed | 99 |

In-process layout wipes remain in `tests/core/semantic_chunk_migration.rs`.
Keep these DBs tiny; do not check in full sample indexes.
