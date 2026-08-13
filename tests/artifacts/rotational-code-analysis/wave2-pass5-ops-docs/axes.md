# Axes — Wave 2 Pass 5 vs Pass 4

| Axis | Pass 4 (xproc) | Pass 5 (ops-docs) |
|------|----------------|-------------------|
| observer | scheduler (writer stamp / peer poll) | **operator** (doctor/status surfaces) |
| representation | interleaving / process stamp | **lifecycle-runbook** (durability, pin, deadline, cascade stop table) |
| evidence | core+mcp+cm tests | **docs+source** (cascade/env/mcp/codemode + doctor issue + ESC-3 string) |
| time | reorder (generation epoch) | (unchanged; not primary) |
| scale | process | (unchanged; not primary) |

**≥2 axes changed:** observer, representation, evidence.
**V-SAME-GAZE avoided:** not re-expanding writer_generation / root jail / Searcher cache.
