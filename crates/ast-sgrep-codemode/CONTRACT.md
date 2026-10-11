# Code Mode core contract

This crate exposes the tool catalog, warm `CodeModeSession`, batch/plan runner,
and host adapters described in [docs/codemode.md](../../docs/codemode.md).
It depends on the retrieval core, not the MCP server.

- The host chooses the session root; per-call root overrides must remain within
  it. An index override is privileged host configuration, not guest authority.
- Calls preserve core ranking and tie-breaking. No cross-machine latency or
  arbitrary-pattern completeness guarantee is made.
- Chain uses the core's post-seeding attempt budget, independent of output limit.
  `max_work` is a validated integer in 1..=1,000,000 (default 8,192), not a SQL,
  byte or time limit. Cutoffs disclose reasons/work; discovered node count becomes
  a lower bound. Complete replies retain their legacy shape. See the chain
  bounds section in `docs/codemode.md` for units, prefix bias and exclusions.
- Tool/argument failures return errors; batch results retain per-call failures.
  Sessions and batches enforce their existing call-count and payload budgets.
- Read/edit paths preserve OS filename identity, including literal POSIX
  backslashes; only actual directory separators are normalized for refs.
  Canonical paths that cannot be represented as UTF-8 are rejected, not decoded
  into replacement-character aliases.
- Read/edit source paths must be regular files. Disk loads use the core's
  capped reader, rejecting oversized metadata before decoding and bounding
  reads even if a regular file grows. Static FIFOs are rejected before open;
  this is not a race-free guarantee against concurrent path replacement.
- Edit batches validate before writing, but write/reindex failures are not a
  multi-file rollback transaction. External filesystem writers are not locked.
- Read windows exclude the index's EOF cursor row and preserve line positions,
  including leading blank lines. An exhausted character budget reports
  truncation rather than an empty file.
- Cancellation is cooperative: indexing polls its token; an already-running
  read/search can finish. Hosts must propagate deadlines into session calls.
- No hand-written unsafe boundary is introduced; workspace safety lints apply.
  Optional features and dependency choices are declared in `Cargo.toml`.

Relevant tests live in `tests/codemode/`. The native consumer's
`ffi_surface` target checks read-window fidelity and admission behavior; its
`ffi_bilateral` target compares native and core contracts. See the
[native contract](../ast-sgrep-codemode-napi/CONTRACT.md) for targeted commands.
