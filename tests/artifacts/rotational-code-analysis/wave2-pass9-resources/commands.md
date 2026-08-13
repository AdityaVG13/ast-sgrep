# Commands — Wave 2 Pass 9 (evidence)

Read-only / source dual-evidence (ZERO-CHANGE; no RCH product verify).

```bash
# Freeze
git rev-parse HEAD
# → 872cc82a73d387f97f391497b3b642238fbdae23 (dirty; pass8 watch fix committed as 872cc82)

# Embed HTTP agent: redirects(0) only — no .timeout / timeout_read
rg -n "fn embed_http_agent|redirects|timeout" crates/ast-sgrep-embed/src/embedder.rs

# ureq 2.12 defaults (registry): timeout_read=None, timeout=None; timeout_connect=Some(30s)
# path: ~/.cargo/registry/src/*/ureq-2.12.1/src/agent.rs (~L256–259)

# Index file cap vs pattern native unbounded read
rg -n "MAX_INDEX_FILE_BYTES|read_text_capped|fs::read" \
  crates/ast-sgrep-core/src/io_bounds.rs \
  crates/ast-sgrep-core/src/pattern.rs \
  crates/ast-sgrep-core/src/index.rs

# MCP admission / scan / query bounds
rg -n "MAX_SCAN_BYTES|INDEX_REPO_DEADLINE|MAX_STDIN_LINE_BYTES|MAX_QUERY_CHARS|index_lock|MAX_AGENT_LIMIT" \
  crates/ast-sgrep-mcp/src/lib.rs \
  crates/ast-sgrep-core/src/limits.rs

# Lockfile / extension policy
rg -n "INDEXABLE_EXTENSIONS|should_skip_file" crates/ast-sgrep-core/src/gitignore.rs

# IVF build peak + mmap open
rg -n "build_from_flat|to_vec|read_clusters_bounded|mmap" \
  crates/ast-sgrep-core/src/semantic_ann.rs \
  crates/ast-sgrep-core/src/semantic_ivf.rs

# Code Mode soft timeout (Promise.race; no AbortController on timer)
rg -n "Promise.race|setTimeout|timeoutMs" packages/pi/extension/src/codemode/runner.ts
```
