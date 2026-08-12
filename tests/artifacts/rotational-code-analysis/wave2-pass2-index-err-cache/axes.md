# Axes (≥2 vs pass 1 freeze)

| Axis | Pass 1 freeze | Pass 2 harden |
|------|---------------|---------------|
| representation | (freeze ledger) | **exception-graph** (Ok vs Err after bulk commit) |
| time | new-freeze | **degradation** (stale Searcher after mid-sidecar Err) |
| observer | operator-harden | **failure-handler** (invalidate on Err surface) |

axes_changed_count: 3
