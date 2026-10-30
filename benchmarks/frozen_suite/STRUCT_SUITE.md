# Frozen structural-intent suite (2026-10-29)

Conformance coverage for the structural half of the engine (callers /
defs / imports channels), which the conceptual suites (CoSQA, RepoQA)
never exercise. Generated deterministically from the indexed RepoQA
corpora via `struct_gen.py` (selection rules in the file header);
frozen at sha256
`f6a258b320d47ec48ed4d605e9ccb880bb00c725edd219a3454a1a4b2140a29d`
(596 queries: 298 callers, 180 defs, 118 imports).

Run against a built binary from the bench dir:

    python3 struct_arm.py            # default binary + out-struct
    python3 struct_arm.py --out out-struct-cap1

Baseline (budget-probe binary, 2026-10-29): callers MRR@10 .9813,
defs 1.0000, imports .9397, ALL .9787. Scoring is file-level
(first returned hit whose file is in gold), matching RepoQA convention.
