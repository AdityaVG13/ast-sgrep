# Security policy

## Forbid-soundness (first-party `unsafe`)

Product crates inherit `[workspace.lints.rust] unsafe_code = "forbid"` and each
crate root carries `#![forbid(unsafe_code)]`. New workspace members must use
`[lints] workspace = true` so they inherit the ban.

The **only** intentional `unsafe` in this repository is
`crates/ast-sgrep-mmap`, a sealed wrapper around `memmap2::MmapOptions::map`.
IVF sidecars are published via write → fsync → rename; callers never mutate a
mapped inode in place. Do not add `#[allow(unsafe_code)]` in product crates.

Local / CI gate (also on `pull_request`):

```bash
bash scripts/verify-forbid-soundness
```

### `cargo audit` ≠ forbid-soundness

| Gate | What it checks |
|------|----------------|
| `scripts/verify-forbid-soundness` | First-party product code cannot use `unsafe` except the sealed mmap crate |
| `cargo audit` | Known advisories in **dependencies** (RustSec) |

Both are required. Passing audit does not mean forbid-soundness holds.

### `fuzz/` exclusion

The `fuzz/` tree is excluded from the workspace (`Cargo.toml` `exclude`).
Fuzz targets may need facilities that product code forbids. Bounded fuzz jobs
in CI still exercise parsers; they are not a license to weaken product crates.

## Environment trust

See [docs/env-trust.md](docs/env-trust.md) for embed URL allowlists,
`ASGREP_AST_GREP` / PATH exec policy, and binary-path integrity gates.

## Reporting

Open a GitHub issue with reproduction steps for security-sensitive defects.
Prefer fail-closed behavior: missing roots, empty indexes, and untrusted env
must surface as errors — never silent empty success.
