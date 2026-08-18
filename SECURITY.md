# Security policy

## Forbid-soundness (first-party `unsafe`)

Ordinary product crates inherit `[workspace.lints.rust] unsafe_code = "forbid"`
and each crate root carries `#![forbid(unsafe_code)]`. New workspace members
must use `[lints] workspace = true` so they inherit the ban.

There are exactly two sealed exceptions:

- `crates/ast-sgrep-mmap` contains the repository's only hand-written `unsafe`,
  wrapping `memmap2::MmapOptions::map`. IVF sidecars are published via write →
  fsync → rename; callers never mutate a mapped inode in place.
- `crates/ast-sgrep-codemode-napi` permits unsafe only because `napi-derive`
  generates FFI glue for Node's C ABI. Its first-party source contains no
  hand-written unsafe block.

Do not add `#[allow(unsafe_code)]` or another crate-level exception outside
these two reviewed boundaries.

Local / manual CI gate:

```bash
bash scripts/verify-forbid-soundness
```

### `cargo audit` ≠ forbid-soundness

| Gate | What it checks |
|------|----------------|
| `scripts/verify-forbid-soundness` | First-party code cannot use hand-written `unsafe` except the sealed mmap crate; the generated N-API FFI exception stays explicit |
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
Prefer fail-closed behavior: missing roots, untrusted env, and empty indexes
when `--no-auto-index` is set must surface as errors — never silent empty
success. Search indexes an empty checkout first unless that flag is set.
