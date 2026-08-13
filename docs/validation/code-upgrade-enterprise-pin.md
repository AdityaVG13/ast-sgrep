# code-upgrade-enterprise PIN

Campaign start on `refactor/de-monolithize-isomorphic` (PR 29).

- Artifact root: `.code-upgrade-enterprise/` (gitignored).
- Mode: `dedicated-session`.
- This pass: Surface/PIN + books skeleton only.
- Primary verify gate (campaign): `rch exec -- cargo test --workspace --no-fail-fast` with `$HOME/.local/bin` first on `PATH`.
- Inherited baseline (not re-measured at PIN): 488 passed / 0 failed / 4 ignored on spark-1672 (demonolith Phase 3), HEAD `da6ade5`.
- CI also defines `cargo fmt --check` and `cargo clippy --workspace --release --all-targets -- -D warnings` (clippy historically red on main; re-measure before claims).

Green suite alone is not upgrade proof.
