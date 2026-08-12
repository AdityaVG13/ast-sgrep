# Axes — Wave 2 Pass 6 vs Passes 2–5

| Axis | Passes 2–5 (prior) | Pass 6 (this) |
|------|--------------------|---------------|
| representation | exception-graph / policy-lattice / interleaving / lifecycle-runbook | **state-store-model** (SQLite bulk tx · sidecars · active.json · flat legacy) |
| observer | failure-handler / attacker / scheduler / operator | **data-integrity** (one coherent corpus after crash/partial activation) |
| time | degradation / reorder / (ops docs) | **commit+recovery** (corrupt activation · refuse stale fallthrough) |
| evidence | MCP/CM invalidate · root jail · writer_generation · doctor/docs | **store path resolution + generation_swap pin** |

**≥2 axes changed:** representation, observer, time.
**V-SAME-GAZE avoided:** not re-expanding Searcher invalidate-on-Err, CM root jail, writer_generation stamp mechanics, or FastUnsafe doctor/docs.
