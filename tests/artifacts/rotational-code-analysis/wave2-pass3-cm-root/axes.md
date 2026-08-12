# Axes (≥2 vs pass 2)

| Axis | Pass 2 harden | Pass 3 harden |
|------|---------------|---------------|
| representation | exception-graph | **policy-lattice** (MCP jail vs CM free → unified contained-in-root) |
| observer | failure-handler | **attacker** (foreign `root` + pinned `index_path` prune) |
| scale | (implicit module) | **boundary** (Session/NAPI tool-root edge) |

axes_changed_count: 3
prior_pass_axes: representation:exception-graph | time:degradation | observer:failure-handler
