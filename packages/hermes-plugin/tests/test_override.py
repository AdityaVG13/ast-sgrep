"""Failure-first tests for the search_files habit-lane override (Lane B).

The override takes over Hermes' core `search_files` tool for the narrow
eligible set (literal content search on a warm indexed root) and falls back
to the original handler everywhere else. The host gates registration behind
`plugins.entries.asgrep.allow_tool_override: true`; without it the override
must be inert.

These tests run standalone: the Hermes host modules the override probes are
faked in sys.modules with recording sentinels, so every pinned behavior
(guards, redaction, fallbacks, takeover rendering) is asserted against the
override module alone.
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
import threading
import types
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
PKG = HERE.parent
if str(PKG) not in sys.path:
    sys.path.insert(0, str(PKG))

BIN = Path(
    os.environ.get(
        "ASGREP_TEST_BIN",
        str(PKG.parents[1] / "target" / "release" / "asgrep"),
    )
)


# ---------------------------------------------------------------------------
# Fake host surface: exactly the symbols asgrep_hermes.override probes.
# ---------------------------------------------------------------------------

class _Recorder:
    def __init__(self):
        self.calls = []


def make_fake_host(cwd: str):
    """Build tools.registry / tools.file_tools / tools.file_operations_common
    fakes with host-faithful semantics for the pieces the wrapper mirrors."""
    rec = _Recorder()

    file_tools = types.ModuleType("tools.file_tools")
    file_tools.CWD = cwd
    file_tools._read_tracker = {}
    file_tools._read_tracker_lock = threading.Lock()
    file_tools.GUARDRAIL_REFUSAL_KEY = "guardrail_refusal"
    file_tools.redacted = []

    def normalize_search_pagination(offset, limit):
        return (max(0, offset), max(1, limit))

    def _bump_consecutive(task_data, key):
        if task_data.get("last_key") == key:
            task_data["consecutive"] = task_data.get("consecutive", 0) + 1
        else:
            task_data["last_key"] = key
            task_data["consecutive"] = 1
        return task_data["consecutive"]

    def get_nt_namespace_error(path, verb="Search"):
        return None

    def _resolve_path_for_task(path, task_id="default"):
        p = Path(path)
        if not p.is_absolute():
            p = Path(file_tools.CWD) / p
        return str(p.resolve())

    def get_read_block_error(path):
        return None

    def _check_not_found_cache(kind, path, task_id):
        return None

    def _record_not_found(kind, path, task_id, blob):
        rec.calls.append(("record_not_found", path))

    def _filter_read_blocked_search_results(result, task_id="default"):
        kept = [m for m in result.matches if "secret.env" not in m.path]
        dropped = len(result.matches) - len(kept)
        result.matches = kept
        return dropped

    def redact_sensitive_text(text, file_read=False, secret_file=False):
        file_tools.redacted.append(text)
        return "«redacted»" if "tok_" in text else text

    def _is_secret_file_arg(path):
        return "secret.env" in str(path)

    def _resolved_match_path(path, task_id="default"):
        return path

    for name, fn in list(locals().items()):
        if callable(fn) and getattr(fn, "__module__", None) is None:
            try:
                fn.__module__ = "tools.file_tools"
            except (AttributeError, TypeError):
                pass
    file_tools.normalize_search_pagination = normalize_search_pagination
    file_tools._bump_consecutive = _bump_consecutive
    file_tools.get_nt_namespace_error = get_nt_namespace_error
    file_tools._resolve_path_for_task = _resolve_path_for_task
    file_tools.get_read_block_error = get_read_block_error
    file_tools._check_not_found_cache = _check_not_found_cache
    file_tools._record_not_found = _record_not_found
    file_tools._filter_read_blocked_search_results = _filter_read_blocked_search_results
    file_tools.redact_sensitive_text = redact_sensitive_text
    file_tools._is_secret_file_arg = _is_secret_file_arg
    file_tools._resolved_match_path = _resolved_match_path

    common = types.ModuleType("tools.file_operations_common")

    class SearchMatch:
        def __init__(self, path, line_number, content, mtime=0.0):
            self.path = path
            self.line_number = line_number
            self.content = content
            self.mtime = mtime

    class SearchResult:
        def __init__(self, matches=None, files=None, counts=None, total_count=0,
                     truncated=False, limit_reason=None, warning=None, error=None):
            self.matches = matches or []
            self.files = files or []
            self.counts = counts or {}
            self.total_count = total_count
            self.truncated = truncated
            self.limit_reason = limit_reason
            self.warning = warning
            self.error = error

        def to_dict(self, densify=False):
            out = {"total_count": self.total_count}
            if self.matches:
                out["matches"] = [
                    {"path": m.path, "line": m.line_number, "content": m.content}
                    for m in self.matches
                ]
            if self.files:
                out["files"] = self.files
            if self.truncated:
                out["truncated"] = True
            if self.warning:
                out["warning"] = self.warning
            if self.error:
                out["error"] = self.error
            return out

    common.SearchMatch = SearchMatch
    common.SearchResult = SearchResult

    registry_mod = types.ModuleType("tools.registry")

    class _Registry:
        def __init__(self):
            self.entries = {}

        def get_entry(self, name):
            return self.entries.get(name)

        def register_original(self, name, schema, handler, check_fn=None):
            self.entries[name] = types.SimpleNamespace(
                schema=schema, handler=handler, check_fn=check_fn, emoji="🔎",
                description="")

    class _Original:
        """The builtin search_files handler: records and answers."""

        def __init__(self):
            self.calls = []

        def __call__(self, args, **kw):
            self.calls.append((dict(args), kw))
            return json.dumps({"total_count": 0, "via": "original"})

    registry_mod.tool_error = lambda msg, **kw: json.dumps({"error": msg, **kw})
    registry = _Registry()
    original = _Original()
    registry.register_original(
        "search_files",
        schema={"name": "search_files", "description": "stock", "parameters": {}},
        handler=original, check_fn=lambda: True)
    registry_mod.registry = registry

    return types.SimpleNamespace(
        rec=rec, file_tools=file_tools, common=common,
        registry_mod=registry_mod, registry=registry, original=original)


class OverrideContext:
    """PluginContext subset for override registration."""

    def __init__(self, allow_override=True):
        self.tools = {}
        self.allow_override = allow_override

    def register_tool(self, name, toolset, schema, handler, check_fn=None,
                      override=False, **kw):
        if override and not self.allow_override:
            class PluginToolOverrideError(Exception):
                pass
            raise PluginToolOverrideError(
                f"cannot override built-in tool {name!r}")
        self.tools[name] = {
            "toolset": toolset, "schema": schema, "handler": handler,
            "check_fn": check_fn, "override": override,
        }

    def get_config(self, key, default=None):
        return default


class FakePool:
    def __init__(self, has_worker=True, response=None, error=None):
        self.has_worker_value = has_worker
        self.response = response or {"ok": True, "value": {"hits": []}}
        self.error = error
        self.calls = []

    def has_worker(self, root):
        return self.has_worker_value

    def call(self, root, tool, args, timeout_s=None):
        self.calls.append((str(root), tool, dict(args)))
        if self.error is not None:
            raise self.error
        return self.response


def _hit(file, line, preview):
    return {"file": file, "lines": {"start": line, "end": line},
            "preview": preview, "ref": f"{file}#L{line}-L{line}"}


HITS = [
    _hit("src/a.rs", 3, "fn alpha_marker() {"),
    _hit("src/c.rs", 2, "    alpha_marker(); tok_xyz"),
]


class OverrideTestBase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        (self.root / "src").mkdir()
        (self.root / "src" / "a.rs").write_text("fn alpha_marker() {\n    alpha_marker();\n}\n")

        self.host = make_fake_host(cwd=str(self.root))
        self._installed = {}
        for modname, mod in (("tools", types.ModuleType("tools")),
                             ("tools.file_tools", self.host.file_tools),
                             ("tools.file_operations_common", self.host.common),
                             ("tools.registry", self.host.registry_mod)):
            self._installed[modname] = sys.modules.get(modname)
            sys.modules[modname] = mod
        sys.modules.pop("asgrep_hermes.override", None)

    def tearDown(self):
        for modname, prev in self._installed.items():
            if prev is None:
                sys.modules.pop(modname, None)
            else:
                sys.modules[modname] = prev
        sys.modules.pop("asgrep_hermes.override", None)
        self._tmp.cleanup()

    def load_override(self):
        import asgrep_hermes.override as ov  # noqa: PLC0415
        return ov

    def register(self, pool, ctx=None):
        ov = self.load_override()
        return ov.register_search_override(
            ctx or OverrideContext(),
            pool,
            root_fn=lambda: self.root,
            setting=lambda key, default=None: default,
        )

    def call_search(self, pool, args, ctx=None, undo=None):
        """Register (once), then invoke the overridden search_files handler."""
        if "search_files" not in ctx.tools:
            self.register(pool, ctx)
        handler = ctx.tools["search_files"]["handler"]
        return handler(dict(args), task_id="t1")


class ProbeFailClosedTest(OverrideTestBase):
    def test_missing_host_modules_leave_override_inert(self):
        for modname in ("tools.file_tools", "tools.file_operations_common",
                        "tools.registry"):
            sys.modules.pop(modname, None)
        ctx = OverrideContext()
        undo = self.register(FakePool(), ctx)
        self.assertIsNone(undo)
        self.assertNotIn("search_files", ctx.tools)

    def test_host_gate_refusal_is_silent(self):
        ctx = OverrideContext(allow_override=False)
        undo = self.register(FakePool(), ctx)
        self.assertIsNone(undo)
        self.assertNotIn("search_files", ctx.tools)


class RegistrationTest(OverrideTestBase):
    def test_override_registers_with_stock_schema_and_undo(self):
        ctx = OverrideContext()
        undo = self.register(FakePool(), ctx)
        self.assertIsNotNone(undo)
        spec = ctx.tools["search_files"]
        self.assertTrue(spec["override"])
        # Schema must be the live stock schema object, unmodified.
        self.assertIs(spec["schema"],
                      self.host.registry.entries["search_files"].schema)
        self.assertEqual(spec["schema"]["description"], "stock")
        undo()
        self.assertNotIn("search_files", ctx.tools)


class FallbackTest(OverrideTestBase):
    def setUp(self):
        super().setUp()
        self.ctx = OverrideContext()
        self.pool = FakePool()

    def run_search(self, **extra):
        args = {"pattern": "alpha_marker", "target": "content"}
        args.update(extra)
        return self.call_search(self.pool, args, ctx=self.ctx)

    def assert_fell_back(self, out):
        self.assertEqual(json.loads(out)["via"], "original")
        self.assertEqual(self.pool.calls, [], "asgrep must not be touched")

    def test_regex_metachar_pattern_falls_back(self):
        self.assert_fell_back(self.run_search(pattern="alpha.*marker"))

    def test_files_target_falls_back(self):
        self.assert_fell_back(self.run_search(target="files"))

    def test_context_lines_fall_back(self):
        self.assert_fell_back(self.run_search(context=2))

    def test_non_content_output_modes_fall_back(self):
        self.assert_fell_back(self.run_search(output_mode="count"))
        self.assert_fell_back(self.run_search(output_mode="files_only"))

    def test_file_glob_falls_back(self):
        self.assert_fell_back(self.run_search(file_glob="*.rs"))

    def test_nonzero_offset_falls_back(self):
        self.assert_fell_back(self.run_search(offset=10))

    def test_cold_worker_falls_back_without_spawning(self):
        pool = FakePool(has_worker=False)
        out = self.call_search(pool, {"pattern": "alpha_marker"}, ctx=self.ctx)
        self.assertEqual(json.loads(out)["via"], "original")
        self.assertEqual(pool.calls, [])

    def test_path_outside_indexed_root_falls_back(self):
        out = self.run_search(path=str(self.root.parent))
        self.assert_fell_back(out)

    def test_worker_error_falls_back_verbatim(self):
        from asgrep_hermes.serve_client import ServeError
        pool = FakePool(error=ServeError("worker exploded"))
        out = self.call_search(pool, {"pattern": "alpha_marker"}, ctx=self.ctx)
        self.assertEqual(json.loads(out)["via"], "original")


class TakeoverTest(OverrideTestBase):
    def setUp(self):
        super().setUp()
        self.ctx = OverrideContext()
        self.pool = FakePool(response={"ok": True, "value": {"hits": HITS}})

    def test_literal_takeover_renders_matches(self):
        out = json.loads(self.call_search(
            self.pool, {"pattern": "alpha_marker", "limit": 10}, ctx=self.ctx))
        self.assertNotIn("via", out)
        self.assertEqual(out["total_count"], 2)
        self.assertEqual(
            [(m["path"], m["line"]) for m in out["matches"]],
            [("src/a.rs", 3), ("src/c.rs", 2)])
        # The match with the token marker was redacted through the host hook.
        self.assertEqual(out["matches"][1]["content"], "«redacted»")
        # The original handler must never run on takeover.
        self.assertEqual(self.host.original.calls, [])
        # The worker saw the literal channel with a truncation probe.
        root, tool, args = self.pool.calls[0]
        self.assertEqual(tool, "search")
        self.assertEqual(args["query"], "literal:alpha_marker")
        self.assertEqual(args["limit"], 11)

    def test_zero_hits_is_a_final_answer(self):
        pool = FakePool(response={"ok": True, "value": {"hits": []}})
        out = json.loads(self.call_search(
            pool, {"pattern": "absent_literal"}, ctx=self.ctx))
        self.assertEqual(out["total_count"], 0)
        self.assertNotIn("matches", out)
        self.assertEqual(self.host.original.calls, [])

    def test_truncation_probe_sets_hint(self):
        pool = FakePool(response={
            "ok": True,
            "value": {"hits": HITS + [_hit("notes.md", 1, "alpha_marker here")]}})
        out = json.loads(self.call_search(
            pool, {"pattern": "alpha_marker", "limit": 2}, ctx=self.ctx))
        self.assertEqual(out["total_count"], 2)
        self.assertTrue(out.get("truncated"))
        self.assertIn("offset=2", out["_hint"])

    def test_read_blocked_matches_are_omitted_with_note(self):
        pool = FakePool(response={"ok": True, "value": {"hits": [
            _hit("src/a.rs", 3, "fn alpha_marker() {"),
            _hit("secret.env", 1, "tok_abc alpha_marker"),
        ]}})
        out = json.loads(self.call_search(
            pool, {"pattern": "alpha_marker"}, ctx=self.ctx))
        # Engine answered 2; the blocked match is dropped from `matches` and
        # the count/note delta is explained — the stock pipeline's shape.
        self.assertEqual(out["total_count"], 2)
        self.assertEqual([m["path"] for m in out["matches"]], ["src/a.rs"])
        self.assertIn("omitted", out["_omitted"])

    def test_repeat_guard_blocks_fourth_identical_search(self):
        args = {"pattern": "alpha_marker"}
        for i in range(3):
            out = json.loads(self.call_search(self.pool, args, ctx=self.ctx))
            self.assertNotIn("error", out)
        blocked = json.loads(self.call_search(self.pool, args, ctx=self.ctx))
        self.assertIn("BLOCKED", blocked["error"])
        self.assertTrue(blocked["guardrail_refusal"])
        # The guard fires before any worker interaction on the 4th call.
        self.assertEqual(len(self.pool.calls), 3)


class PrewarmTest(OverrideTestBase):
    def test_prewarm_pings_index_status_in_background(self):
        ov = self.load_override()
        pool = FakePool()
        thread = ov.prewarm(pool, root_fn=lambda: self.root, timeout_s=5)
        thread.join(timeout=10)
        self.assertFalse(thread.is_alive())
        self.assertEqual(
            [(t, a) for _r, t, a in pool.calls],
            [("index_status", {})])

    def test_prewarm_swallows_errors(self):
        ov = self.load_override()
        from asgrep_hermes.serve_client import ServeError
        pool = FakePool(error=ServeError("no worker"))
        thread = ov.prewarm(pool, root_fn=lambda: self.root, timeout_s=5)
        thread.join(timeout=10)
        self.assertFalse(thread.is_alive())  # must not raise


class PoolProbeTest(unittest.TestCase):
    def test_has_worker_false_on_fresh_pool(self):
        from asgrep_hermes.serve_client import ServePool
        pool = ServePool(str(BIN), timeout_s=30)
        try:
            self.assertFalse(pool.has_worker(Path.cwd()))
        finally:
            pool.close()


if __name__ == "__main__":
    unittest.main()
