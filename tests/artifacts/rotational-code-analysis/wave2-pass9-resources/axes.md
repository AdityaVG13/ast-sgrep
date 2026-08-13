# Axes — Wave 2 Pass 9 vs Pass 8 / prior wave-2

| Axis | Pass 8 (prior) | Pass 9 (this) |
|------|----------------|---------------|
| representation | policy-lattice (identity→resource) | **cost-model** (CPU/mem/I/O/HTTP/queue bounds) |
| observer | attacker+tenant | **capacity-planner** (saturation, admission, deadlines) |
| scale | identity→resource | **request→fleet** (single MCP/CM process under amplify) |
| time | (implicit identity) | **load** (overload, hang, OOM, soft timeout) |

**≥2 axes changed vs pass 8:** representation, observer, scale, time (4).

**V-SAME-GAZE avoided:** do not re-litigate `sandbox_root` / Option A (pass 3), schema refuse (pass 7), watch symlink (pass 8), generation fallthrough (pass 6), writer_generation (pass 4).
**Out of scope this pass:** Pi leftover `runtime.ts`; `R-PI-EDIT-SYMLINK-LEXICAL` (different loop).
