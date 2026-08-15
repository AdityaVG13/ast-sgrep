# Graph precision fixture

This fixed, intentionally non-compiling Rust corpus exercises call-edge
resolution without external services. `local_target` is file-local unique,
`other_target` is repository unique, `shared_target` is name-only because two
definitions exist, and the checked-in SCIP projection upgrades `remote_target`
to `scip_exact`.

Every extracted call is listed in `benchmarks/gold/graph_precision.json`, so
unlisted predictions are genuine false positives rather than unlabeled data.
Regenerate results with `./benchmarks/run_eval.sh`; do not promote its numbers
without a reviewed clean-worktree fingerprint row in `results/baselines.md`.
