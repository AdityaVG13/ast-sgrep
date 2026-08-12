# tests/

All project tests live here. Production crate sources must not contain
ingrained `mod tests` bodies.

| Path | What |
|---|---|
| `tests/<crate>/` | Cargo integration tests. Each crate's `Cargo.toml` points here with `[[test]] path = ...`. |
| `tests/unit/<crate>/` | Unit tests for private items. Included from the module under test with `#[cfg(test)] #[path]`. |
| `tests/pi/` | Node/TypeScript tests for Pi extension and launcher. |
| `tests/fixtures/` | Shared corpora used by integration tests. |

`#[cfg(test)]` branches inside production functions are fault-injection
hooks, not test suites. They stay next to the code they perturb.
