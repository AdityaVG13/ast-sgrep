"""Native Hermes registration for ast-sgrep.

Eager repo-search tools (`asgrep_search`, `asgrep_read`, `asgrep_status`)
over a persistent `asgrep serve` NDJSON worker: zero spawn cost per call, warm
index, ambient freshness (the worker builds/refreshes the index for its root
automatically — first call on an unindexed repo just works). Registration does
no indexing, no network, no filesystem writes; every tool is read-only.

Fail-closed: when the asgrep binary cannot be resolved, check_fn removes the
tools from the model's context entirely (no dead schemas). Tool-level errors
from the worker pass through verbatim — including stale-schema errors, which
carry their exact `asgrep reindex <root> --json` recovery command.
"""

from __future__ import annotations

import atexit
import json
import logging
from pathlib import Path

try:  # imported as a package module by the Hermes loader
    from .asgrep_hermes.eager import install_eager_tools
    from .asgrep_hermes.override import prewarm, register_search_override, should_prewarm
    from .asgrep_hermes.serve_client import ServeError, ServePool, resolve_binary
except ImportError:  # imported as a top-level module (tests, ad-hoc runs)
    import sys as _sys

    _here = str(Path(__file__).resolve().parent)
    if _here not in _sys.path:
        _sys.path.insert(0, _here)
    from asgrep_hermes.eager import install_eager_tools
    from asgrep_hermes.override import prewarm, register_search_override, should_prewarm
    from asgrep_hermes.serve_client import ServeError, ServePool, resolve_binary

log = logging.getLogger(__name__)

TOOLSET = "asgrep"

_SEARCH_SCHEMA = {
    "name": "asgrep_search",
    "description":
        "Reach for this first when you know a name — even a fragment — or "
        "want where it's defined, who calls it, or who imports it. Results "
        "rank definitions-first, and every hit carries a ref that "
        "asgrep_read expands into exact source — search chains into "
        "reading, no re-grepping. One grammar covers the ladder: "
        "parse_pattern (fused), defs:SYMBOL, callers:SYMBOL, "
        "imports:MODULE, word:TERM, pattern:'fn name($$$A)'. The index "
        "persists per repo and warms as you work. For plain-text, log, or "
        "config lookups, use your text search.",
    "parameters": {
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "maxLength": 4096,
                "description": "Fused keyword/AST query: plain words, symbols, "
                               "snake_case fragments ('parse_pattern', 'auth refresh'), "
                               "structural patterns with metavariables "
                               "('fn name($$$A)'), or a channel prefix — "
                               "defs:SYMBOL, callers:SYMBOL, imports:MODULE, "
                               "word:TERM, pattern:PATTERN",
            },
            "lang": {"type": "string", "description": "Language filter (rust, python, typescript, ...)"},
            "file_filter": {"type": "string", "description": "Glob filter on file paths, e.g. src/*.rs"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Max hits (default 20)"},
            "excerpt_lines": {
                "type": "integer", "minimum": 0, "maximum": 50,
                "description": "Include N context lines around each hit excerpt "
                               "(0 = capsule previews only)",
            },
            "semantic_only": {
                "type": "boolean",
                "description": "Force the semantic channel only (default: fused hybrid)",
            },
        },
        "required": ["query"],
        "additionalProperties": False,
    },
}

_READ_SCHEMA = {
    "name": "asgrep_read",
    "description":
        "Expand refs from asgrep_search into exact, bounded source — the "
        "second half of the workflow: search finds, read retrieves. Pass "
        "the ref verbatim ('src/a.rs#L12-L40', LSP 'src/a.rs:12', or a hit "
        "id); optional context lines and per-window budgets. Prefer over "
        "re-searching or shell cat: exact-line, budgeted, warm.",
    "parameters": {
        "type": "object",
        "properties": {
            "refs": {
                "type": "array",
                "items": {"type": "string"},
                "minItems": 1,
                "maxItems": 32,
                "description": "File refs or hit ids from asgrep_search: "
                               "'src/a.rs', 'src/a.rs#L12', 'src/a.rs#L12-L40' "
                               "(LSP-style 'src/a.rs:12' also accepted)",
            },
            "context_lines": {"type": "integer", "minimum": 0, "maximum": 50,
                              "description": "Extra context lines around each window"},
            "max_chars": {"type": "integer", "minimum": 1, "maximum": 20000,
                          "description": "Per-window character budget"},
        },
        "required": ["refs"],
        "additionalProperties": False,
    },
}

_STATUS_SCHEMA = {
    "name": "asgrep_status",
    "description":
        "Call once before heavy retrieval in a fresh repo: index readiness "
        "and counts. If cold, the first search builds it and the rest of "
        "the session rides it.",
    "parameters": {
        "type": "object",
        "properties": {},
        "additionalProperties": False,
    },
}


def _fail(message: str, recovery: str | None = None) -> str:
    out = {"ok": False, "error": message}
    if recovery:
        out["recovery"] = recovery
    return json.dumps(out, ensure_ascii=False)


def register(ctx):
    def setting(key, default):
        getter = getattr(ctx, "get_config", None)
        return getter(key, default) if callable(getter) else default

    def _float_setting(key, default):
        value = setting(key, default)
        try:
            return float(value)  # pyright: ignore[reportArgumentType]
        except (TypeError, ValueError):
            return float(default)

    # A live worker busy past a call's timeout is PRESERVED (warming) until
    # this grace elapses: ambient freshness commits only at the end, so a
    # premature kill restarts the whole pass on every retry (2026-10-26
    # TensorAST incident). Default 900s covers several times the measured
    # 400s worst-case freshness on a 63k-file tree.
    pool = ServePool("asgrep", timeout_s=_float_setting("timeout_s", 120),
                     warm_grace_s=_float_setting("warm_grace_s", 900))

    def binary() -> str | None:
        return resolve_binary(str(setting("binary", "asgrep")))

    def timeout_s() -> float:
        try:
            return float(setting("timeout_s", 120))
        except (TypeError, ValueError):
            return 120.0

    def root() -> Path | None:
        getter = getattr(ctx, "session_cwd", None)
        cwd = getter() if callable(getter) else None
        path = Path(cwd or Path.cwd()).resolve()
        return path if path.is_dir() else None

    def dispatch(tool: str, args: dict) -> str:
        here = root()
        if here is None:
            return _fail("session root is not a directory", "Open the session inside a repository.")
        executable = binary()
        if executable is None:
            return _fail(
                f"asgrep binary not found (`{setting('binary', 'asgrep')}`)",
                "Install ast-sgrep (`asgrep install --help`) or set the plugin "
                "`binary` config to an absolute path.",
            )
        pool.binary = executable
        try:
            out = pool.call(here, tool, args, timeout_s())
        except ServeError as exc:
            return _fail(exc.message, exc.recovery)
        except Exception:
            # Handler-boundary backstop: the model always gets a structured
            # envelope, never a stack trace through the tool bridge.
            log.exception("asgrep dispatch failed unexpectedly")
            return _fail("unexpected asgrep plugin failure",
                         "The failure was logged; retry the call, and if it "
                         "repeats, set the plugin `binary` to a known-good "
                         "asgrep or report the log line.")
        if out.get("ok"):
            return json.dumps({"ok": True, "value": out.get("value")}, ensure_ascii=False)
        error = out.get("error")
        return _fail(error if isinstance(error, str) else "asgrep tool failed",
                     "The error above comes from asgrep itself and names the exact fix.")

    def search(params, **kwargs):
        args = dict(params or {})
        forwarded = {k: args[k] for k in (
            "query", "lang", "file_filter", "limit", "excerpt_lines", "semantic_only",
        ) if k in args}
        if "query" not in forwarded:
            return _fail("query is required")
        return dispatch("search", forwarded)

    def read(params, **kwargs):
        args = dict(params or {})
        forwarded = {k: args[k] for k in ("refs", "context_lines", "max_chars") if k in args}
        if "refs" not in forwarded:
            return _fail("refs is required", "Pass file refs or hit ids from asgrep_search.")
        return dispatch("read", forwarded)

    def status(params, **kwargs):
        return dispatch("index_status", {})

    def check() -> bool:
        return binary() is not None

    ctx.register_tool(name="asgrep_search", toolset=TOOLSET, schema=_SEARCH_SCHEMA,
                      handler=search, check_fn=check)
    ctx.register_tool(name="asgrep_read", toolset=TOOLSET, schema=_READ_SCHEMA,
                      handler=read, check_fn=check)
    ctx.register_tool(name="asgrep_status", toolset=TOOLSET, schema=_STATUS_SCHEMA,
                      handler=status, check_fn=check)

    undo_eager = None
    if bool(setting("eager", True)):
        with __import__("contextlib").suppress(Exception):
            undo_eager = install_eager_tools(
                ["asgrep_search", "asgrep_read", "asgrep_status"],
                lambda: bool(setting("eager", True)),
            )

    # Lane B — habit lane: take over the stock search_files for eligible
    # literal searches (host-gated by plugins.entries.asgrep.allow_tool_override;
    # inert unless the operator grants it). Prewarm then supplies the live
    # worker + ambient index build so the first eligible call is warm.
    undo_override = None
    with __import__("contextlib").suppress(Exception):
        undo_override = register_search_override(
            ctx, pool, root_fn=root, setting=setting)
    if bool(setting("prewarm", True)):
        executable = binary()
        here = root()
        # Speculation is scoped to repo-like roots (.git / existing .asgrep):
        # a session opened in $HOME must not ambient-index the home tree.
        if executable is not None and here is not None and should_prewarm(here):
            with __import__("contextlib").suppress(Exception):
                pool.binary = executable
                try:
                    warm_s = float(setting("prewarm_timeout_s", 600))
                except (TypeError, ValueError):
                    warm_s = 600.0
                prewarm(pool, root_fn=root, timeout_s=warm_s)

    def unload():
        pool.close()
        if undo_eager is not None:
            undo_eager()
        if undo_override is not None:
            undo_override()

    atexit.register(pool.close)
    on_unload = getattr(ctx, "on_unload", None)
    if callable(on_unload):
        on_unload(unload)
