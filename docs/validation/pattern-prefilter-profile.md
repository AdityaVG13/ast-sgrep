# Native pattern search prefilter

Behavioral coverage lives in `crates/ast-sgrep-core/tests/pattern_prefilter.rs`:
literal needles skip non-candidate files, metavariable-only patterns disable the
prefilter without losing matches, and declaration keywords are not treated as
cross-language required literals.

Historical work-span / Brent numbers from a one-off `release-perf` host run are
not reproduced in-tree (no fixture harness). Prefer the behavioral tests above
over profile theater when gating PRs.
