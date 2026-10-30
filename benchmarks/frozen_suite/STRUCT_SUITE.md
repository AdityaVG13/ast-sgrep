# Frozen structural-intent suite (2026-10-29)

Conformance coverage for the structural half of the engine (callers /
defs / imports channels), which the conceptual suites (CoSQA, RepoQA)
never exercise. Generated deterministically from the indexed RepoQA
corpora via `struct_gen.py` (selection rules in the file header);
frozen at sha256
`231aec65efc1876cd8a3499997fa2365d0330356157176db22d6350ae64605bc`
(596 queries: 298 callers, 180 defs, 118 imports).

Run against a built binary from the bench dir:

    python3 struct_arm.py            # default binary + out-struct
    python3 struct_arm.py --out out-struct-cap1

Baseline (cap1 binary, 2026-10-29 rev 2): callers MRR@10 .9846,
defs 1.0000, imports .9617, ALL .9847. Rev 2 fixes: callers lane skips
empty callees; imports gold credits qualified shapes (`pkg::X`) as hits
for `imports:X`, matching engine semantics (exact-key gold under-credited
and fake-penalized the engine's correct qualified matches). Scoring is file-level
(first returned hit whose file is in gold), matching RepoQA convention.
