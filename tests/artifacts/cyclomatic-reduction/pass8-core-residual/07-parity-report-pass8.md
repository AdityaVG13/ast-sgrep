# 07 — Parity report (Pass 8 core residual)

## Compile

```text
cargo check -p ast-sgrep-core
# Finished `dev` profile … (ok)
```

## Targeted tests (joint-allowed; no workspace suite)

```text
cargo test -p ast-sgrep-core --test regex_budget --test semantic_ivf_roundtrip --test parity
# parity: 3 passed
# regex_budget: 1 passed
# semantic_ivf_roundtrip: 8 passed, 1 ignored

cargo test -p ast-sgrep-core --test search_correctness_epics --test code_prose_fields --test e2e_smoke
# code_prose_fields: 5 passed
# e2e_smoke: 5 passed, 1 ignored
# search_correctness_epics: 10 passed (includes iva9_5_literal_lang_filter_not_starved_by_path_limit)
```

## Differential notes

- `content_matches_literal`: pure rewrite of existing `if let Some(needle_lower)` arms; same call order to `has_literal_match` / `to_lowercase`.
- `write_ivf_temporary`: body moved verbatim from the former closure; cleanup-on-err still at publication site.
- No public API changes.

## Escape

Level-4 uncurated property differential not re-run as a separate harness; existing integration suite covers literal lang filter, IVF roundtrip/corruption, and search epics. Documented as suite-backed parity for this residual wave.
