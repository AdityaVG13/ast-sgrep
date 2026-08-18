#!/usr/bin/env python3
"""Rebuild tiny legacy SQLite corpora for ghiw.4 migration tests.

Does not require cargo. Run from the repository root:

    python3 tests/fixtures/migration/build_legacy.py

Then `cargo test -p ast-sgrep-core --test semantic_chunk_migration committed_schema`.
Do not treat these files as published-number goldens.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path


def write(path: Path, version: int) -> None:
    if path.exists():
        path.unlink()
    conn = sqlite3.connect(path)
    conn.execute(f"PRAGMA user_version = {version}")
    conn.execute(
        "CREATE TABLE files(id INTEGER PRIMARY KEY, path TEXT, language TEXT, "
        "mtime_secs INTEGER, mtime_nanos INTEGER, content_hash TEXT)"
    )
    conn.commit()
    conn.close()
    print(f"{path.name} {path.stat().st_size} bytes user_version={version}")


def main() -> None:
    root = Path(__file__).resolve().parent
    write(root / "schema5_empty.sqlite", 5)
    write(root / "schema99_unsupported.sqlite", 99)


if __name__ == "__main__":
    main()
