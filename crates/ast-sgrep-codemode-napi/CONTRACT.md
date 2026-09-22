# Native Code Mode boundary

`ast-sgrep-codemode-napi` exposes the core Code Mode session to Node via N-API.
Pi loads this addon through `packages/pi/extension/src/codemode/native.ts`.

- Accepted `Session.call` and `Session.batch` requests return Promises and run on
  libuv workers. Admission errors throw; task failures reject the Promise.
- Calls on one session serialize through a mutex; independent sessions stay isolated.
- Pre-aborted signals reject without calling a tool or incrementing `callCount`.
  Later aborts cancel queued mutex waiters and interrupt indexing at its existing
  cancellation checkpoints. An already-running read/search may finish.
- Cancellation does not leave the session locked; subsequent calls remain usable.
- Batch admission, payload, output, and root-confinement limits come from
  `ast-sgrep-codemode`. A tool failure stays attached to its batch entry.
- `callNow` is restricted to the existing bounded lookup/cache contract.
- Results preserve the core ranking and tie-breaking; no latency guarantee is made.
- Hand-written source uses safe Rust. Only napi-derive generated FFI glue is an
  unsafe exception. No new feature flags are introduced at this boundary.

Verification: after exported-signature changes, run
`cargo test -p ast-sgrep-codemode-napi --test ffi_surface --test ffi_bilateral`
to check Rust callers and admission contracts without a Node environment.
Build only this crate with `cargo build -p ast-sgrep-codemode-napi`,
copy its host library to a `.node` artifact, then set `ASGREP_CODEMODE_NAPI_PATH`
to that artifact and run `npm run test:native --workspace pi-ast-sgrep`. The real
Node suite in `tests/pi/extension/native-inprocess.test.ts` checks Promise
behavior, cancellation before/during work, batch cancellation, and recovery.
An explicitly supplied addon must be the exact binding exercised; tests cannot
silently skip or substitute a compatible fallback.
