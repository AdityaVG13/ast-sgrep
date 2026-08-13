# Assumptions — Wave 2 Pass 1

1. Wave-1 audit seal (`pass12-convergence`) remains valid as **books complete-with-residuals**; product residual R-* still open.
2. User authorize HARDEN for product fixes on PR #27 is explicit; empty authorize would stay audit.
3. Re-freeze to `62ee4b4595ad2433bd16b0ac14747dada612b4d6` supersedes wave-1 freeze identity `fb932aac852f5496c0a7035cc5a0b508e05111cb` for subsequent harden rotations; wave-1 snapshot inventory retained under V-STATE-IGNORE (not content-authoritative for new HEAD).
4. Dirty beads DB/WAL and Pi extension leftovers do not block freeze; Pi leftovers are out of scope for this mission.
5. `fszero-codemode` missing ⇒ CodeMode/zs fs unavailable; shell is the authorized evidence path.
