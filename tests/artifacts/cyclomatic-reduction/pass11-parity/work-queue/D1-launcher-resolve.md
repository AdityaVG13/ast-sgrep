# Work packet D1 — Launcher resolve family

| Field | Value |
|---|---|
| id | D1 |
| priority | P2 |
| status | open / **Defer** |
| risk | medium |
| product_area | `packages/pi/launcher` |
| campaign_bill_file_ΣCC | **102** (must not rise) |
| baseline_file_ΣCC | 106 (pass-1 era) |

## Goal

Lower decision density in resolve\* helpers **only** if file (or package) ΣCC does not rise. Prefer permanent Keep residual over vanity extract.

## Exact targets (census pass 10)

| Function | CC | NLOC | File:line (approx) |
|---|---:|---:|---|
| `resolveHost` | 26 | 55 | `packages/pi/launcher/src/index.js:136` |
| `resolveCodemodeAddon` | 18 | 46 | `packages/pi/launcher/src/index.js:231` |
| `resolveBinary` | 17 | 29 | `packages/pi/launcher/src/index.js:198` |

Also read helpers already extracted in pass 3: `isPathFallbackError`, `isOptionalHostMiss`, and any pass-9-era assert helpers still in tree.

## History (do not repeat)

| Pass | Action | Bill |
|---|---|---|
| 3 | Guard clauses on resolve\* | fn CC −3/−5/−5; file −4 net |
| 9 | Pure extract `assertHostManifestMatches` + addon helpers | **+6 Refuse** — reverted / not kept as cut |
| 10 | Re-measure only | file ΣCC **102** |

## Classification

| Function | Class | Notes |
|---|---|---|
| resolveHost | accidental_structure + extractable residual | PATH / platform / checksum ladder; much is requisite error taxonomy |
| resolveCodemodeAddon | accidental_structure | Soft-null vs throw paths; overlap with host package resolution |
| resolveBinary | accidental_structure | PATH fallback domain; thin residual after guards |

## Allowed techniques (only)

1. **Shared collapse** of **duplicate** decision trees across the three resolve\* functions (same predicate / same error code map).
2. **Consolidate predicates** already proven bill-neutral (named helpers that **eliminate** a duplicated `if`, not just move it).
3. Dead-path removal **only** with test proof the branch is unreachable.

## Forbidden

- Pure extract that moves branches without eliminating decisions (measured **+6**).
- API / host-manifest contract redesign.
- Changing PATH fallback codes (`ASGREP_PLATFORM_PACKAGE_MISSING` | `ASGREP_EXECUTABLE_EMPTY` | `ASGREP_UNSUPPORTED_PLATFORM`).
- Soft-null codemode behavior changes.

## Procedure for implementer

1. Read all three functions end-to-end; table duplicate predicates (error code sets, env key order, checksum steps).
2. If **no** duplicate tree ≥2 sites → **stop**: mark packet `Keep residual`, do not edit.
3. If duplicate exists: extract **one** shared predicate/helper that **removes** decisions from ≥2 call sites.
4. Re-measure **before** commit claim:
   ```bash
   # from skill scripts dir or project-known measure path
   python /Users/aditya/AI/JeffreySkills/_custom/cyclomatic-reduction/scripts/measure_complexity.py \
     packages/pi/launcher/src --threshold 10
   ```
   Accept only if `total_cc` ≤ **102**.
5. Run verify suite (below).
6. If suite red **and** caused by your edit → revert. If red is F1/F2 inventory/keyword → not your problem.

## Verify (acceptance)

```bash
cd packages/pi/launcher
node --test test/npm-native-packages.test.mjs test/binary-env-alias.test.mjs
# expect 13 pass

# optional security floor (should stay green)
node --test test/package-security.test.mjs
```

Pins that must still hold (from pass-3 parity):

- PATH fallback only for the three platform/empty/unsupported codes above
- Codemode soft-null for unsupported platform / missing package
- Checksum message prefixes for executable vs NAPI addon

## Resolve default

**Defer** until a real shared-collapse is visible under diff review.  
Otherwise permanent **Keep residual** — not a failure of the campaign.

## Stop / escalate

- ΣCC would rise → Refuse + document like pass 9.
- Need public API change → Refuse (needs human auth outside skill).
