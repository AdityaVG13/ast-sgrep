# Pass 3 — parity notes

## Launcher

```bash
cd packages/pi/launcher
node --test test/binary-env-alias.test.mjs test/npm-native-packages.test.mjs
# 13 pass

node --test test/package-security.test.mjs
# 4 pass
```

Ad-hoc: unsupported / missing-package error codes and codemode soft-null verified via one-shot node smoke.

## Rust

```bash
cargo check -p ast-sgrep-core   # ok
cargo test -p ast-sgrep-core --test p1_correctness_batch  # 6 pass
```

`cargo test -p ast-sgrep-core pipeline_parts` **not green** due to pre-existing lib-test `SearchHit.resolution` / `SearchResponse` field drift in fusion/search fixtures — unrelated to `update_paths`.

## Behavior pins preserved

- PATH fallback only for `ASGREP_PLATFORM_PACKAGE_MISSING` | `ASGREP_EXECUTABLE_EMPTY` | `ASGREP_UNSUPPORTED_PLATFORM`
- Codemode soft-null for unsupported platform / missing package
- Checksum message prefixes for executable vs NAPI addon
- Watch: empty relative / directory → continue without `files_skipped++`
