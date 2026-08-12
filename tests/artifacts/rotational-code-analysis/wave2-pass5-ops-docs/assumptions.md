# Assumptions — Wave 2 Pass 5

1. Wave-2 freeze SHA retained (`62ee4b45…`); dirty tree during harden is B-DIRTY-FREEZE process note, not a product defect.
2. FastUnsafe doctor issue sets `healthy:false` / exit 2 — intentional CI/ops visibility, not a silent warn-only.
3. C1 fix is **docs aligned to code** (lexical fallback), not changing hybrid stop semantics.
4. CM/NAPI root already jailed in pass 3 — docs must not still claim free root; host duty remains Session/`ASGREP_ROOT` choice.
5. ESC-3 honesty is post-mutation deadline only; pre-start deadline remains "before start".
6. zerostack / tokenzero engines unavailable on this host (B-ZS-ENGINES) — install note only, not product.
7. RCH requires `RCH_CANONICAL_PROJECT_ROOT=/Users/aditya` because `/Users/aditya/AI/ast-sgrep` symlink escapes default `canonical_root=/Users/aditya/AI`.
