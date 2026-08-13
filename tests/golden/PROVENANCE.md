# Golden provenance

Goldens live under `tests/golden/` (workspace) or crate-local fixture paths
passed to `assert_golden_at` / `assert_golden_json_at`.

| Field | Rule |
|---|---|
| Command | The test that froze the file (crate + test name). |
| Date | ISO date of the freeze. |
| Scrub | `Scrubber` preset: `none`, `standard`, `machine_contract`, `search_dump(root)`, `doctor`, `status`. |
| Notes | Why this freeze is stable. |

Update with `ASGREP_UPDATE_GOLDENS=1` only. Never `UPDATE_GOLDENS` or `INSTA_UPDATE`.
Compare is the default (env unset). Mismatches write `{golden}.actual` (gitignored).

## Existing crate-local freezes

These predate this helper and stay next to `machine_contracts`:

| File | Command | Scrub | Notes |
|---|---|---|---|
| `tests/cli/fixtures/capabilities.json` | `ast-sgrep-cli` `capabilities_and_version_match_goldens` | test assigns `version` → `<version>` then `assert_golden_json_at` | Machine capabilities envelope. |
| `tests/cli/fixtures/envelopes.json` | same test, `version` sub-object | ad-hoc | Still `assert_eq!` until a later child. |
| `tests/cli/fixtures/machine_shapes.json` | `index_reindex_status_and_doctor_have_stable_shapes` | key-set only | Shape keys, not a full dump. |
