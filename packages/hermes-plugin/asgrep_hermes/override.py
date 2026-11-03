"""Habit-lane override for Hermes' stock ``search_files`` (Lane B).

The power lane (``asgrep_search`` & co.) requires the model to choose the
tool. This module takes over the tool models already reach for habitually:
for the narrow eligible set — literal content search on a warm indexed root —
the answer comes from the persistent ``asgrep serve`` worker; every other
call (regex patterns, file search, context lines, pagination, count modes)
falls through to the stock ripgrep handler, verbatim.

Fidelity contract (ripgrep-faithful by construction):

* Registration uses the host's sanctioned ``register_tool(override=True)``
  seam, which refuses to load unless the operator sets
  ``plugins.entries.asgrep.allow_tool_override: true`` — the lane ships
  dormant and the HOST enforces the trust decision, not this module.
* The stock schema is re-registered unmodified: the model-visible surface
  never changes; acceleration is an implementation detail.
* The takeover path mirrors ``tools.file_tools.search_tool`` by importing and
  calling the host's own pipeline pieces (repeat-search guard, NTLM/path
  resolution, read-block filtering, secret redaction), so the guardrails
  that live above the search engine all still run; pages that could be
  partial hand off to ripgrep whole (``_hint`` pagination is stock's own).
* Every host symbol is probed up front; if ANY is missing (host update moved
  a private name), the override stays inert — the stock tool is untouched.
* Any exception during takeover falls back to the stock handler; the override
  can slow a call down, never break one.

Takeover eligibility is deliberately narrow v1: target=content, a literal
pattern (no regex metacharacters), context=0, offset=0, output_mode=content,
no file_glob, the resolved path equal to the indexed session root, and a
worker the pool ALREADY holds for that root (the search path never spawns;
``prewarm`` supplies liveness at session start).
"""

from __future__ import annotations

import json
import logging
import threading
from pathlib import Path
from typing import Any, Callable, Optional

from .serve_client import ServeError

log = logging.getLogger(__name__)

# A pattern containing any of these is a regex, not a literal: fall back so
# the answer is exactly what rg would return (plus "\n" — multiline regex).
_REGEX_METACHARS = set("\\.^$*+?()[]{}|")

_HOST = None  # probed host namespace; None = override inert this process


def _probe_host():
    """Import every host symbol the takeover path mirrors, or None.

    All-or-nothing: a host update that renames one private helper must
    disable the override, not half-work.
    """
    try:
        from importlib import import_module

        registry_mod = import_module("tools.registry")
        file_tools = import_module("tools.file_tools")
        common = import_module("tools.file_operations_common")

        host = types_namespace(
            tool_error=getattr(registry_mod, "tool_error"),
            registry=getattr(registry_mod, "registry"),
            normalize_search_pagination=getattr(file_tools, "normalize_search_pagination"),
            read_tracker=getattr(file_tools, "_read_tracker"),
            read_tracker_lock=getattr(file_tools, "_read_tracker_lock"),
            bump_consecutive=getattr(file_tools, "_bump_consecutive"),
            guardrail_refusal_key=getattr(file_tools, "GUARDRAIL_REFUSAL_KEY"),
            get_nt_namespace_error=getattr(file_tools, "get_nt_namespace_error"),
            resolve_path_for_task=getattr(file_tools, "_resolve_path_for_task"),
            get_read_block_error=getattr(file_tools, "get_read_block_error"),
            check_not_found_cache=getattr(file_tools, "_check_not_found_cache"),
            record_not_found=getattr(file_tools, "_record_not_found"),
            filter_read_blocked=getattr(file_tools, "_filter_read_blocked_search_results"),
            redact_sensitive_text=getattr(file_tools, "redact_sensitive_text"),
            is_secret_file_arg=getattr(file_tools, "_is_secret_file_arg"),
            resolved_match_path=getattr(file_tools, "_resolved_match_path"),
            SearchMatch=getattr(common, "SearchMatch"),
            SearchResult=getattr(common, "SearchResult"),
        )
        for value in vars(host).values():
            if value is None:
                return None
        return host
    except Exception:  # noqa: BLE001 — any host drift must disable, not crash
        log.info("search_files override inert: host pipeline symbols unavailable")
        return None


def types_namespace(**kwargs):
    import types

    return types.SimpleNamespace(**kwargs)


def _is_host_gate_error(exc: BaseException) -> bool:
    if type(exc).__name__ == "PluginToolOverrideError":
        return True
    try:
        from hermes_cli.plugins import PluginToolOverrideError  # noqa: PLC0415

        return isinstance(exc, PluginToolOverrideError)
    except Exception:  # noqa: BLE001
        return False


def _eligible(host, pattern: str, target: Optional[str], file_glob, limit: int, offset: int,
              output_mode: str, context: int, resolved_root: Optional[str],
              session_root: Optional[Path], pool) -> bool:
    if target != "content":
        return False
    if not pattern or any(ch in _REGEX_METACHARS or ch == "\n" for ch in pattern):
        return False
    if context or offset or file_glob or output_mode != "content":
        return False
    if session_root is None or resolved_root != str(session_root):
        return False
    if not pool.has_worker(session_root):
        return False
    return True


def register_search_override(ctx, pool, *, root_fn: Callable[[], Optional[Path]],
                             setting: Callable[[str, Any], Any]):
    """Install the search_files takeover if the host allows it.

    Returns an undo callable, or None when the lane is inert (host symbols
    missing, no stock entry, or the operator has not granted
    ``allow_tool_override``).
    """
    global _HOST
    if _HOST is None:
        _HOST = _probe_host()
    host = _HOST
    if host is None:
        return None
    entry = host.registry.get_entry("search_files")
    if entry is None or not callable(getattr(entry, "handler", None)):
        return None
    original_handler = entry.handler

    def fallback(args, **kw):
        return original_handler(args, **kw)

    def search_files(args, **kw):
        task_id = kw.get("task_id") or "default"
        try:
            out = _takeover(host, pool, root_fn, args, task_id)
            if out is not None:
                return out
        except Exception:  # noqa: BLE001 — the override may slow, never break
            log.exception("search_files takeover failed; falling back to ripgrep")
        return fallback(args, **kw)

    handle = None
    try:
        handle = ctx.register_tool(
            name="search_files",
            toolset="file",
            schema=entry.schema,
            handler=search_files,
            check_fn=getattr(entry, "check_fn", None),
            emoji=getattr(entry, "emoji", "🔎"),
            override=True,
        )
    except Exception as exc:  # noqa: BLE001
        if _is_host_gate_error(exc):
            log.info(
                "search_files override dormant: set "
                "plugins.entries.asgrep.allow_tool_override: true to enable"
            )
        else:
            log.exception("search_files override registration failed")
        return None

    def undo() -> None:
        dispose = getattr(handle, "dispose", None)
        if callable(dispose):
            try:
                dispose()
            except Exception:  # noqa: BLE001
                pass
        tools = getattr(ctx, "tools", None)
        if isinstance(tools, dict):
            tools.pop("search_files", None)

    return undo


def _takeover(host, pool, root_fn, args, task_id):
    """Mirror search_tool for the eligible subset; None = fall back."""
    target_map = {"grep": "content", "find": "files"}
    raw_target = args.get("target", "content")
    target = target_map.get(raw_target, raw_target)
    pattern = args.get("pattern", "")
    path = args.get("path", ".")
    if path is None or (isinstance(path, str) and not path.strip()):
        path = "."
    file_glob = args.get("file_glob")
    limit = args.get("limit", 50)
    offset = args.get("offset", 0)
    output_mode = args.get("output_mode", "content")
    context = args.get("context", 0)
    order = args.get("order", "discovery")

    session_root = root_fn()
    if session_root is not None:
        # Canonical on both sides: _resolve_path_for_task resolves symlinks
        # (macOS /var -> /private/var), so compare against the resolved root.
        session_root = Path(session_root).resolve()
    resolved_root = None
    nt_err = host.get_nt_namespace_error(path, verb="Search")
    if nt_err:
        return host.tool_error(nt_err)
    try:
        resolved_root = str(host.resolve_path_for_task(path, task_id))
    except (OSError, ValueError, RuntimeError) as exc:
        if isinstance(exc, RuntimeError) and not host.get_read_block_error(path):
            raise
        resolved_root = path

    if not _eligible(host, pattern, target, file_glob, limit, offset, output_mode,
                     context, resolved_root, session_root, pool):
        return None

    offset, limit = host.normalize_search_pagination(offset, limit)

    # Repeat-search guard: identical plumbing to search_tool so the anti-loop
    # guardrail survives the engine swap (search_key includes the same fields).
    # The entry is snapshotted before bumping: every FALLBACK path restores it,
    # so the stock handler's own bump is the only one that survives a handoff
    # -- exactly one bump per call, whichever engine answers (copy/restore is
    # layout-agnostic, so a host-side tracker format change cannot wedge it).
    search_key = ("search", pattern, target, str(path), file_glob or "", limit, offset, order)
    with host.read_tracker_lock:
        task_data = host.read_tracker.setdefault(
            task_id, {"last_key": None, "consecutive": 0, "read_history": set()})
        saved_task_data = dict(task_data)
        count = host.bump_consecutive(task_data, search_key)

    def _restore_guard() -> None:
        with host.read_tracker_lock:
            current = host.read_tracker.get(task_id)
            if current is not None:
                current.clear()
                current.update(saved_task_data)

    if count >= 4:
        return host.tool_error(
            f"BLOCKED: You have run this exact search {count} times in a row. "
            "The results have NOT changed. You already have this information. "
            "STOP re-searching and proceed with your task.",
            pattern=pattern,
            already_searched=count,
            **{host.guardrail_refusal_key: True})

    block_error = host.get_read_block_error(resolved_root)
    if block_error:
        return host.tool_error(block_error)
    cached_nf = host.check_not_found_cache("search", resolved_root, task_id)
    if cached_nf is not None:
        return cached_nf

    # --- the only leaf that differs from search_tool: the engine ---
    # Takeover serves ONLY answers that are indistinguishable from what rg
    # would have shown on this call:
    #   * the complete hit set fits strictly inside one page (hit_count
    #     < limit) -- the worker clamps the limit+1 probe at its own cap,
    #     so a FULL page can never be proven complete, and a capped page
    #     would be asgrep's ranked subset vs rg's walk-order subset with
    #     offset pagination crossing engines mid-stream;
    #   * no hit's preview was width-elided ("...") -- stock shows up to 500
    #     chars of a line, the capsule preview ~120 with the match itself
    #     possibly hidden in the elided middle.
    # Anything else hands off to the stock handler with the guard restored.
    # 60s matches the stock rg pipeline's own per-run budget: if the worker
    # cannot answer within what ripgrep would have been given, fall back and
    # let rg rescan at its budget instead of stacking both latencies.
    try:
        out = pool.call(session_root, "search",
                        {"query": f"literal:{pattern}", "limit": limit + 1},
                        timeout_s=60.0, retry=False)
        if not out.get("ok"):
            _restore_guard()
            return None
        hits = (out.get("value") or {}).get("hits") or []
        if len(hits) >= limit:
            # A full page (or more) cannot be proven complete -- the worker
            # clamps the limit+1 probe at its own cap, and a ranked top-N is
            # not rg's walk-order first-N. rg owns the pagination sequence.
            _restore_guard()
            return None
        if any(str(h.get("preview", "")).endswith("…") for h in hits):
            # A long line was elided; stock would show up to 500 chars.
            _restore_guard()
            return None
        matches = [
            host.SearchMatch(
                path=str(h.get("file", "")),
                line_number=int((h.get("lines") or {}).get("start", 0)),
                content=str(h.get("preview", "")),
            )
            for h in hits
        ]
    except Exception:
        log.exception("asgrep takeover failed; falling back to stock search")
        _restore_guard()
        return None
    result = host.SearchResult(matches=matches, total_count=len(matches),
                               truncated=False)

    omitted = host.filter_read_blocked(result, task_id)
    for m in getattr(result, "matches", None) or ():
        if m.content:
            m.content = host.redact_sensitive_text(
                m.content, file_read=True,
                secret_file=host.is_secret_file_arg(
                    host.resolved_match_path(m.path, task_id)))
    result_dict = result.to_dict(densify=True)

    if omitted:
        result_dict["_omitted"] = (
            f"{omitted} result(s) omitted because they target credential, "
            "token, cache, or secret-bearing environment files.")
    _search_err = result_dict.get("error") or ""
    if isinstance(_search_err, str) and _search_err.startswith("Path not found:"):
        host.record_not_found("search", resolved_root, task_id,
                              json.dumps(result_dict, ensure_ascii=False))
    if count >= 3:
        result_dict["_warning"] = (
            f"You have run this exact search {count} times consecutively. "
            "The results have not changed. Use the information you already have.")
    return json.dumps(result_dict, ensure_ascii=False)


def should_prewarm(root: Path, home: Optional[Path] = None) -> bool:
    """Prewarm is speculation: only repo-like roots earn it.

    A session opened in ``$HOME`` (the desktop default) must not kick off a
    background ambient index of the whole home tree — the worker caps
    per-file bytes, not tree size. A root is repo-like when it carries a
    ``.git`` (a checkout someone opened deliberately) or an existing
    ``.asgrep`` index (asgrep was used here before). The home tree and its
    ancestors are excluded outright: asgrep's own index lives at
    ``$HOME/.asgrep``, so the index marker alone would otherwise make $HOME
    repo-like and reinstate exactly the ambient home scan this rule forbids.
    The power lane is unaffected: explicit ``asgrep_*`` calls work on any
    root on demand; only the speculative warm-up is scoped. Without a warm
    worker the habit lane also stays inert on such roots — stock rg answers
    there.
    """
    root = Path(root).resolve()
    home = (Path(home) if home is not None else Path.home()).resolve()
    if root == home or root in home.parents:
        return False
    return (root / ".git").exists() or (root / ".asgrep").is_dir()


def prewarm(pool, *, root_fn: Callable[[], Optional[Path]],
            timeout_s: float = 600.0) -> threading.Thread:
    """Start the serve worker + ambient index build off the search path.

    The habit lane only takes over when the pool already holds a live worker
    for the session root; prewarm supplies that liveness at session start so
    the FIRST eligible search_files call is warm. Errors are swallowed — a
    failed prewarm just leaves the lane cold (stock ripgrep answers).
    """
    def _run():
        try:
            root = root_fn()
            if root is None:
                return
            pool.call(Path(root), "index_status", {}, timeout_s)
        except ServeError:
            log.debug("prewarm: serve worker unavailable", exc_info=True)
        except Exception:  # noqa: BLE001
            log.exception("prewarm failed")

    thread = threading.Thread(target=_run, name="asgrep-prewarm", daemon=True)
    thread.start()
    return thread
