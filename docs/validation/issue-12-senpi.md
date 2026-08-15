# Issue 12: senpi graph-mode validation

Validation source: [`code-yeongyu/senpi`](https://github.com/code-yeongyu/senpi) at commit `8e489041fd9fc7c2a937ea59f85c6a7f99650eca`.

The original Issue 12 report recorded 3,486 files, 144,959 caller edges, and 10,327 imports. The upstream monorepo has continued to grow. Reindexing the pinned snapshot on 2026-07-26 produced:

| Metric | Value |
|---|---:|
| Indexed files | 3,746 |
| Skipped files | 196 |
| Symbols | 22,861 |
| Caller edges | 184,409 |
| Imports | 12,898 |

Run the external-corpus graph oracle with:

```bash
ASGREP_REAL_PI_FIXTURE=/Users/aditya/ast-sgrep-senpi-fixture \
  cargo test --locked -p ast-sgrep-core --release --test e2e_smoke \
  archived_pi_fixture_graph_modes_match_indexed_keys -- \
  --ignored --nocapture
```

The oracle verifies all of the following against the freshly built index in one process:

- `defs:refreshToken` returns definition evidence.
- `callers:refreshToken` returns caller evidence and has the same count as `callers:refreshtoken`.
- `chain refreshToken` returns graph evidence.
- Three source-spelled callees that also have definitions return equal mixed-case and lowercase caller counts.
- The three most frequent stored module paths return equal source-spelled and lowercase import counts.
- The status totals meet the full-monorepo scale: at least 3,000 files, 100,000 caller edges, and 10,000 imports.

The test is ignored by default because the external repository is intentionally not vendored. Set `ASGREP_REAL_PI_FIXTURE` and explicitly include ignored tests to repeat this validation. The weekly and manually dispatched [Large graph E2E workflow](../../.github/workflows/graph-scale.yml) checks out the pinned corpus, runs this exact test, fails if the corpus is absent or incomplete, and retains the test log as a CI artifact.
