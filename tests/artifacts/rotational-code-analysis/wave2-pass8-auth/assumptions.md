# Assumptions — Wave 2 Pass 8

1. CLI is operator-trusted: no tool-`root` jail (host argv/`cwd` is the authority).
2. MCP / Code Mode / NAPI jail tool `root` under configured workspace (pass 3 Option A) -- not reopened.
3. `ASGREP_INDEX_PATH` / `--index-path` is a privileged sink by product design (labeled pass 5 / `docs/env-trust.md`).
4. Full-tree index uses `WalkDir::follow_links(false)`; watch must not be a weaker gate.
5. Pi `runtime.ts` / `index.ts` rg+freshness dirty tree stays out of scope unless a jail bypass lives there; Pi `planEdit` lexical containment is a separate residual.
6. zerostack unavailable this pass.
