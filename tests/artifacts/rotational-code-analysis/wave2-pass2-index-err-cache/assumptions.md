# Assumptions

- Wave-2 pass-1 freeze/authorize at HEAD `06a6e94` remains the pre-patch baseline for this harden unit.
- Shape **A** (surface invalidate on Ok+Err) is sufficient; full core committed-dirty signaling deferred.
- `force_sidecar_rebuild_err` is test-only (thread_local); not a product failpoint API.
- Pi `runtime.ts` / rg freshness leftover remains out of scope.
