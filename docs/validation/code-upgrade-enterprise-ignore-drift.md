# C-IGNORE-DRIFT — ignored-test count reconciliation (WS-03)

Campaign books `20260813-043427` · branch `refactor/de-monolithize-isomorphic` · measured HEAD `ebac2ad` (post clippy `955d04a`, docs `ebac2ad`).

## Verdict

**Preserve** — suite ignored count is **4**; inventory of `#[ignore]` alone under-counts by one because the fourth ignore is a **doctest fence**, not an attribute.

## Command (measured)

```bash
PATH=$HOME/.local/bin:$PATH rch exec -- cargo test --workspace --no-fail-fast
```

- Host: RCH worker **spark-1672**
- Remote exit: **0**
- Wall: **283101 ms** (~283 s)
- Aggregated from 78 `test result:` lines: **488 passed / 0 failed / 4 ignored**

Matches prior modal baseline `488/0/4` in `demonolith-baselines.md` (no product change required).

## Ignore sites

### `#[ignore = ...]` attributes (3)

| file:line | test | reason |
| --- | --- | --- |
| `tests/core/e2e_smoke.rs:217` | `archived_pi_fixture_graph_modes_match_indexed_keys` | requires `ASGREP_REAL_PI_FIXTURE` archive |
| `tests/core/semantic_ivf_roundtrip.rs:321` | `adaptive_ivf_tradeoff_at_2048_and_10000_vectors` | release-mode ANN recall/latency tradeoff |
| `tests/core/store_delete.rs:158` | `re_upsert_many_files_is_linear` | timing quarantine; not a CI correctness gate |

### Fourth suite ignore (doctest, not `#[ignore]`)

| site | how ignored | suite line |
| --- | --- | --- |
| `crates/ast-sgrep-codemode/src/lib.rs:29` | rustdoc fence ` ```ignore ` in Quick start example | `test crates/ast-sgrep-codemode/src/lib.rs - (line 29) ... ignored` |

## Why inventory said 3

Grep for `#\[ignore\]` / `#\[ignore =` finds only the three integration-test attributes. Cargo still reports **4** ignored because doctests with ` ```ignore ` contribute to the workspace ignored total. That is the entire “drift”; docs/baseline `tests_ignored=4` were already correct.

## Attack (delta of this note)

- No tests enabled or disabled to force a pretty count.
- Numbers taken only from this suite log aggregation (not re-copied from Phase 3 without re-run).
- Artifact sync after RCH warned on local rsync `getcwd`; remote command exit **0** and result lines were captured in `/tmp/ast-sgrep-ws03-suite.log` before local sync failed.
