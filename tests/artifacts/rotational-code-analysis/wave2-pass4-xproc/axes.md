# Axes (≥2 vs pass 3)

| Axis | Pass 3 harden | Pass 4 harden |
|------|---------------|---------------|
| representation | policy-lattice | **interleaving** (writer stamp vs warm Searcher snapshot) |
| observer | attacker | **scheduler** (external watch/CLI epoch vs MCP/CM poll) |
| time | (implicit) | **reorder** (stamp bump before/without in-process invalidate) |
| scale | boundary | **process** (cross-process epoch; same-process stamp simulation) |

axes_changed_count: 4
prior_pass_axes: representation:policy-lattice | observer:attacker | scale:boundary
