# Cost and saturation model — Loop 14

Observer: capacity-planner. Scale: request→fleet (one MCP/CM/CLI process under amplify). Time: load.

## Dominant costs

| Resource | Dominant path | Asymptotic / concrete | Bound / control | Residual |
|----------|---------------|----------------------|-----------------|----------|
| Disk→heap (index) | `read_text_capped` | O(file) ≤ 64 MiB/file | `MAX_INDEX_FILE_BYTES` | CONSISTENT |
| Disk→heap (pattern native) | `pattern.rs` `fs::read` + rayon | O(Σ files) unbounded per file | **none** (vs index cap) | GAP `R-PATTERN-UNBOUNDED-READ` |
| HTTP embed | `embed_via_api` / `embed_via_ollama` | 1 RTT/text; body via `into_json` | allowlist + redirects(0); **no read/overall timeout; no body cap** | GAP `R-EMBED-HTTP-TIMEOUT-BODY` |
| IVF build | `build_from_flat` k-means | O(n·dim·iters); peak ≈ 2× flat f32 | threshold ≥2k; k≤256 | CONSISTENT cost; OOM→Err residual |
| IVF query | mmap vectors + cluster heap | resident ≈ clusters only | `read_clusters_bounded` | CONSISTENT |
| MCP index | single-flight + soft 600s | serial mutator | `index_lock` + `INDEX_REPO_DEADLINE` | CONSISTENT (deadline post-mutate known ESC-3) |
| MCP/CM read | line scan | ≤64 MiB scan; max_chars | `MAX_SCAN_BYTES` + `MAX_READ_CHARS` | CONSISTENT |
| Query / results | search / agent | chars ≤4096; agent limit ≤100; hard ≤1000 | `limits.rs` + MCP schema | CONSISTENT |
| Stdio RPC | MCP / CM batch | line ≤1 MiB | `MAX_STDIN_LINE_BYTES` | CONSISTENT |
| Code Mode wall | `runCodemode` | soft timeout default 30s | Promise.race; **no abort of orphan** | GAP `R-CM-SOFT-TIMEOUT-ORPHAN` |
| Lockfile corpus | walk index | `.lock` skipped; `package-lock.json` as json ≤64 MiB | extension allowlist + index cap | CONSISTENT cost noise |

## Unbounded / weakly bounded amplifiable paths

1. **Embed HTTP hang or huge JSON** (allowlisted host) → indexer / MCP `index_repo` stalls past soft deadline checks (boundary-only); process may OOM on `into_json`. Pass9 books wrongly assumed ureq timeouts exist.
2. **`pattern:` native walk** loads full files in parallel without `MAX_INDEX_FILE_BYTES` — fleet OOM under huge `.json`/`.md` trees.
3. **Code Mode soft timeout** returns Err while AsyncFunction keeps calling shared NAPI `Session` (Mutex-serialized) — capacity bleed, not silent wrong hits.

## Backpressure / admission inventory

| Surface | Admission | Deadline | Load shed | Cleanup under overload |
|---------|-----------|----------|-----------|------------------------|
| MCP `index_repo` | `index_lock` single-flight | 600s soft (pre + post) | refuse start if wait exhausted | invalidate Searcher/registries always |
| MCP search | stdio serial | none (query-bound) | N/A | warm Searcher reuse |
| MCP `code_read` | max 20 ids; scan/chars | none | scan limit Err | TOCTOU reopen checks |
| CM NAPI | Mutex per Session | runner soft timeout | soft only | orphan may continue under lock |
| CLI watch | serial debounce | recv timeout | coalesce paths | deferred sidecar rebuild |
| Embed cloud | env allowlist | **missing** | none | Err string to caller |

## Gate

> Each externally amplifiable resource has a bound, control, or explicit residual risk; micro-optimizations are not conflated with failures.

**Met** — bounds named CONSISTENT; missing timeout/body and pattern read named GAP residuals (availability/cost), not micro-opt theater. No high/critical **correctness-under-load** (wrong answers) dual-evidence finding with a small fix this pass → ZERO-CHANGE.
