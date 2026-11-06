#!/usr/bin/env python3
"""Regenerate tests/cli/constants_contract_generated.rs (F37).

The generated test pins the current values of the tuning constants that the
behavioral suites previously hand-transcribed via `assert_eq!(CONST, literal)`
pins (those broke on every intentional retune). Values here are
machine-transcribed from the `pub const` definitions in `crates/*/src/*.rs`
(the source of truth) — never hand-copied.

Workflow:
  - Intentional retune: change the `pub const` in `crates/`, rerun this
    script from the repo root, review the diff, run the generated target:
      python3 scripts/gen_constants_contract.py
      cargo test -p ast-sgrep-cli --test constants_contract_generated
  - Behavioral suites (`tests/core/oracle_limits.rs`, `tests/codemode/*`,
    `tests/lang/numerical_chunk_ranker.rs`, `tests/cli/oracle_foundry_units.rs`)
    probe boundaries symbolically (`MAX + 1`, `MAX / 2`) and must NOT pin
    values; only this file does.

The generator parses a small expression grammar (int/float literals with `_`,
`+ - *`, parens, Hahn-Banach-free identifier and `a::b::NAME` references).
Keep new constant definitions to that grammar or extend `eval_expr` below.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

CONST_RE = re.compile(
    r"^\s*(?:pub\s+)?const\s+([A-Za-z_][A-Za-z0-9_]*)\s*:\s*"
    r"([A-Za-z0-9_\[\]&; ]+?)\s*=\s*(.+?)\s*;\s*$"
)

TOKEN_RE = re.compile(
    r"\s*(?:(\d[\d_]*(?:\.\d[\d_]*)?)|([A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*)|(\*\*|[+\-*/()]))"
)


def tokenize(expr: str):
    pos, tokens = 0, []
    while pos < len(expr):
        m = TOKEN_RE.match(expr, pos)
        if not m or m.end() == pos:
            raise ValueError(f"unparseable expression: {expr!r}")
        pos = m.end()
        num, ident, op = m.groups()
        if num is not None:
            tokens.append(("num", num))
        elif ident is not None:
            tokens.append(("ident", ident))
        else:
            tokens.append(("op", op))
    return tokens


def eval_expr(expr: str, env: dict[str, float | int]):
    """Evaluate a const initializer over already-parsed values."""
    tokens = tokenize(expr)
    pos = 0

    def peek():
        return tokens[pos] if pos < len(tokens) else (None, None)

    def parse_add():
        nonlocal pos
        lhs = parse_mul()
        while peek() == ("op", "+") or peek() == ("op", "-"):
            op = tokens[pos][1]
            pos += 1
            rhs = parse_mul()
            lhs = lhs + rhs if op == "+" else lhs - rhs
        return lhs

    def parse_mul():
        nonlocal pos
        lhs = parse_atom()
        while peek() == ("op", "*") or peek() == ("op", "/"):
            op = tokens[pos][1]
            pos += 1
            rhs = parse_atom()
            lhs = lhs * rhs if op == "*" else lhs / rhs
        return lhs

    def parse_atom():
        nonlocal pos
        kind, text = peek()
        if kind == "op" and text == "(":
            pos += 1
            val = parse_add()
            if peek() != ("op", ")"):
                raise ValueError(f"missing ')' in {expr!r}")
            pos += 1
            return val
        if kind == "op" and text == "-":
            pos += 1
            return -parse_atom()
        if kind == "num":
            pos += 1
            clean = text.replace("_", "")
            return float(clean) if "." in clean else int(clean)
        if kind == "ident":
            pos += 1
            name = text.split("::")[-1]
            if name not in env:
                raise ValueError(f"unknown const {text!r} in {expr!r}")
            return env[name]
        raise ValueError(f"unexpected token in {expr!r}")

    val = parse_add()
    if pos != len(tokens):
        raise ValueError(f"trailing tokens in {expr!r}")
    return val


# (source file, names pinned by the generated test). Private consts in the
# same files are also parsed so initializers can reference them.
SOURCES: list[tuple[str, list[str]]] = [
    (
        "crates/ast-sgrep-core/src/limits.rs",
        [
            "MAX_OUTPUT_RESULTS",
            "DEFAULT_AGENT_LIMIT",
            "MAX_SEARCH_HIT_EXCERPT_BYTES",
            "MAX_EXCERPT_LINES",
            "MAX_FILE_FILTER_CHARS",
            "MAX_QUERY_CHARS",
            "MAX_REGEX_PATTERN_CHARS",
            "MAX_STDIN_LINE_BYTES",
        ],
    ),
    (
        "crates/ast-sgrep-core/src/semantic_ann.rs",
        ["DEFAULT_ANN_THRESHOLD", "DEFAULT_ADAPTIVE_PROBE_PERCENT"],
    ),
    ("crates/ast-sgrep-core/src/lexicon.rs", ["MIN_SUPPORT", "MAX_PER_TERM"]),
    (
        "crates/ast-sgrep-codemode/src/batch.rs",
        [
            "MAX_BATCH_RESPONSE_BYTES",
            "MAX_BATCH_VALUE_BYTES",
            "MAX_BATCH_ERROR_BYTES",
            "MAX_BATCH_ID_BYTES",
            "MAX_BATCH_TOOL_BYTES",
        ],
    ),
    ("crates/ast-sgrep-codemode/src/session.rs", ["MAX_CALL_RESPONSE_BYTES"]),
    ("crates/ast-sgrep-embed/src/math.rs", ["MIN_SIMILARITY"]),
    (
        "crates/ast-sgrep-cli/src/supervisor.rs",
        ["MIN_CPU_LIMIT", "MAX_CPU_LIMIT", "DEFAULT_CPU_LIMIT"],
    ),
]

USES = [
    (
        "ast_sgrep_core",
        [
            "MAX_OUTPUT_RESULTS",
            "DEFAULT_AGENT_LIMIT",
            "MAX_SEARCH_HIT_EXCERPT_BYTES",
            "MAX_EXCERPT_LINES",
            "MAX_FILE_FILTER_CHARS",
            "MAX_QUERY_CHARS",
            "MAX_REGEX_PATTERN_CHARS",
            "MAX_STDIN_LINE_BYTES",
        ],
    ),
    (
        "ast_sgrep_core::semantic_ann",
        ["DEFAULT_ANN_THRESHOLD", "DEFAULT_ADAPTIVE_PROBE_PERCENT"],
    ),
    ("ast_sgrep_core::lexicon", ["MIN_SUPPORT", "MAX_PER_TERM"]),
    (
        "ast_sgrep_codemode",
        [
            "MAX_BATCH_RESPONSE_BYTES",
            "MAX_BATCH_VALUE_BYTES",
            "MAX_BATCH_ERROR_BYTES",
            "MAX_BATCH_ID_BYTES",
            "MAX_BATCH_TOOL_BYTES",
            "MAX_CALL_RESPONSE_BYTES",
        ],
    ),
    ("ast_sgrep_embed", ["MIN_SIMILARITY"]),
    ("ast_sgrep_cli::supervisor", ["MIN_CPU_LIMIT", "MAX_CPU_LIMIT", "DEFAULT_CPU_LIMIT"]),
]


def parse_sources() -> tuple[dict[str, float | int], dict[str, str]]:
    values: dict[str, float | int] = {}
    types: dict[str, str] = {}
    for rel, _pinned in SOURCES:
        path = ROOT / rel
        for lineno, line in enumerate(path.read_text().splitlines(), 1):
            m = CONST_RE.match(line)
            if not m:
                continue
            name, rust_type, expr = m.group(1), m.group(2).strip(), m.group(3)
            # Only numeric initializers are transcribable; string/slice consts
            # in the same files are skipped unless explicitly pinned.
            try:
                values[name] = eval_expr(expr, values)
            except ValueError as exc:
                if any(name in pinned for _, pinned in SOURCES):
                    raise ValueError(f"{rel}:{lineno}: {exc}") from exc
                continue
            types[name] = rust_type
    return values, types


def fmt_literal(name: str, value: float | int, rust_type: str) -> str:
    if isinstance(value, float):
        if rust_type == "f32":
            return f"{value!r}f32"
        return repr(value)
    if value < 0:
        return f"(-{abs(int(value)):_})"
    return f"{int(value):_}"


def pin(name: str, values, types) -> str:
    if name not in values:
        raise KeyError(f"pinned const {name} not found in parsed sources")
    return f"    assert_eq!({name}, {fmt_literal(name, values[name], types[name])});"


def main() -> int:
    values, types = parse_sources()
    missing = [
        name for _, pinned in SOURCES for name in pinned if name not in values
    ]
    if missing:
        print(f"missing consts: {missing}", file=sys.stderr)
        return 1

    reserve = values.get("BATCH_ENVELOPE_RESERVE_BYTES")
    if reserve is None:
        print("BATCH_ENVELOPE_RESERVE_BYTES not parsed", file=sys.stderr)
        return 1

    out: list[str] = []
    out.append("//! GENERATED by `scripts/gen_constants_contract.py` — do not hand-edit.")
    out.append("//!")
    out.append("//! Machine-transcribed value pins for every tuning constant the")
    out.append("//! behavioral suites used to pin by hand (`assert_eq!(CONST, literal)`,")
    out.append("//! F37). The values below are read from the `pub const` definitions")
    out.append("//! in `crates/*/src/*.rs` (the source of truth), never hand-copied:")
    out.append("//! an intentional retune only requires rerunning the generator")
    out.append("//! (`python3 scripts/gen_constants_contract.py`) and reviewing the diff.")
    out.append("//! Behavioral tests probe boundaries symbolically and must not pin values.")
    out.append("")
    for crate_path, names in USES:
        out.append(f"use {crate_path}::{{{', '.join(names)}}};")
    out.append("")

    def test(fn_name: str, intent: str, names: list[str], extra: list[str] | None = None):
        out.append(f"/// {intent}")
        out.append("#[test]")
        out.append(f"fn {fn_name}() {{")
        out.extend(pin(n, values, types) for n in names)
        out.extend(extra or [])
        out.append("}")
        out.append("")

    test(
        "core_output_and_query_limits",
        "INTENT: generated value pins for the output/query clamp ceilings and",
        [
            "MAX_OUTPUT_RESULTS",
            "DEFAULT_AGENT_LIMIT",
            "MAX_SEARCH_HIT_EXCERPT_BYTES",
            "MAX_EXCERPT_LINES",
            "MAX_FILE_FILTER_CHARS",
            "MAX_QUERY_CHARS",
            "MAX_REGEX_PATTERN_CHARS",
            "MAX_STDIN_LINE_BYTES",
        ],
        ["    assert!(DEFAULT_AGENT_LIMIT < MAX_OUTPUT_RESULTS);"],
    )
    test(
        "core_ann_defaults",
        "INTENT: generated value pins for the ANN threshold defaults.",
        ["DEFAULT_ANN_THRESHOLD", "DEFAULT_ADAPTIVE_PROBE_PERCENT"],
    )
    test(
        "core_lexicon_gates",
        "INTENT: generated value pins for the lexicon support gate and per-term cap.",
        ["MIN_SUPPORT", "MAX_PER_TERM"],
    )
    test(
        "codemode_batch_byte_caps",
        "INTENT: generated value pins for the batch byte caps plus the envelope-reserve arithmetic.",
        [
            "MAX_BATCH_RESPONSE_BYTES",
            "MAX_BATCH_VALUE_BYTES",
            "MAX_BATCH_ERROR_BYTES",
            "MAX_BATCH_ID_BYTES",
            "MAX_BATCH_TOOL_BYTES",
        ],
        [
            "    // Reserve arithmetic, value read from the private",
            "    // BATCH_ENVELOPE_RESERVE_BYTES definition in batch.rs.",
            f"    assert_eq!(MAX_BATCH_RESPONSE_BYTES - MAX_BATCH_VALUE_BYTES, {int(reserve):_});",
        ],
    )
    test(
        "codemode_call_response_alias",
        "INTENT: generated value pin for the per-call response cap plus its aliasing to the stdin line cap.",
        ["MAX_CALL_RESPONSE_BYTES"],
        ["    assert_eq!(MAX_CALL_RESPONSE_BYTES, ast_sgrep_core::MAX_STDIN_LINE_BYTES);"],
    )
    test(
        "embed_similarity_gate",
        "INTENT: generated value pin for the chunk-ranker similarity gate.",
        ["MIN_SIMILARITY"],
    )
    test(
        "cli_cpu_bounds",
        "INTENT: generated value pins for the supervisor CPU-limit bounds plus min<=default<=max ordering.",
        ["MIN_CPU_LIMIT", "MAX_CPU_LIMIT", "DEFAULT_CPU_LIMIT"],
        [
            "    assert!(MIN_CPU_LIMIT <= DEFAULT_CPU_LIMIT && DEFAULT_CPU_LIMIT <= MAX_CPU_LIMIT);"
        ],
    )

    # Drop the unused helper.
    target = ROOT / "tests/cli/constants_contract_generated.rs"
    target.write_text("\n".join(out))
    print(f"wrote {target} ({len(out)} lines)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
