"""End-to-end tests for the native Hermes plugin, run against the REAL asgrep
release binary. No network, no hermes-agent import: plugin.py is exercised
through a stub context that mirrors the PluginContext surface the plugin uses.

Failure-first: this suite was written and run RED before the plugin package
existed; every test names the behavior it pins.
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
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


def make_corpus(root: Path) -> None:
    (root / "src").mkdir(parents=True, exist_ok=True)
    (root / "src" / "a.rs").write_text(
        "fn alpha_marker() {\n    alpha_marker();\n}\n"
    )
    (root / "src" / "b.py").write_text(
        "def beta_helper():\n    return 'flurbify_me'\n"
    )
    # Lane discriminators: gamma_widget (def + prose mention) and the
    # multi-word "alpha marker" query against a.rs. The keyword lane has
    # no contiguous literal for either; the fused engine resolves symbols.
    (root / "src" / "c.rs").write_text(
        "fn gamma_widget() {\n    alpha_marker();\n}\n"
    )
    (root / "notes.md").write_text(
        "Prose note: gamma_widget is only mentioned here in text.\n"
    )


class StubContext:
    """The PluginContext subset the plugin touches."""

    def __init__(self, config=None, cwd=None):
        # Prewarm spawns a real serve worker on the corpus root; unit tests
        # assert exact spawn counts, so it is opt-in here (plugin default on).
        self.config = {"prewarm": False}
        self.config.update(config or {})
        self.cwd = str(cwd or Path.cwd())
        self.tools = {}
        self.unload_hooks = []

    def get_config(self, key, default=None):
        return self.config.get(key, default)

    def session_cwd(self):
        return self.cwd

    def register_tool(self, name, toolset, schema, handler, check_fn=None, **kw):
        self.tools[name] = {
            "toolset": toolset,
            "schema": schema,
            "handler": handler,
            "check_fn": check_fn,
        }

    def on_unload(self, fn):
        self.unload_hooks.append(fn)


def load_plugin():
    import plugin as plugin_module  # noqa: PLC0415

    return plugin_module


class RegistrationTest(unittest.TestCase):
    def test_three_readonly_tools_in_asgrep_toolset(self):
        ctx = StubContext(cwd=Path.cwd())
        load_plugin().register(ctx)
        self.assertEqual(
            set(ctx.tools), {"asgrep_search", "asgrep_read", "asgrep_status"}
        )
        for name, spec in ctx.tools.items():
            self.assertEqual(spec["toolset"], "asgrep", name)
            self.assertTrue(callable(spec["handler"]), name)
        # Schemas follow the host convention (dict with `parameters`);
        # required args pinned either way.
        def schema_of(name):
            raw = ctx.tools[name]["schema"]
            return raw if isinstance(raw, dict) else json.loads(raw)

        schema = schema_of("asgrep_search")["parameters"]
        self.assertEqual(schema["required"], ["query"])
        self.assertIn("refs", schema_of("asgrep_read")["parameters"]["required"])

    def test_check_fn_gates_on_binary_presence(self):
        ctx = StubContext(cwd=Path.cwd())
        load_plugin().register(ctx)
        self.assertTrue(ctx.tools["asgrep_search"]["check_fn"]())
        bad = StubContext(config={"binary": "/nonexistent/asgrep-nope"}, cwd=Path.cwd())
        load_plugin().register(bad)
        self.assertFalse(bad.tools["asgrep_search"]["check_fn"]())


class ToolHandlerTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name) / "corpus"
        make_corpus(self.root)
        import asgrep_hermes.serve_client as sc  # noqa: PLC0415

        self.spawned = []
        base = sc._Worker
        spawned = self.spawned

        class CountingWorker(base):
            def __init__(self, *a, **kw):
                spawned.append(self)
                super().__init__(*a, **kw)

        self._real_worker = sc._Worker
        sc._Worker = CountingWorker
        self.ctx = StubContext(cwd=self.root)
        load_plugin().register(ctx=self.ctx)

    def tearDown(self):
        for fn in self.ctx.unload_hooks:
            fn()
        import asgrep_hermes.serve_client as sc  # noqa: PLC0415

        sc._Worker = self._real_worker
        self._tmp.cleanup()

    def call(self, name, params):
        raw = self.ctx.tools[name]["handler"](params)
        return json.loads(raw)

    def test_search_read_status_over_one_persistent_worker(self):
        # No index exists: ambient freshness must build it inside the worker.
        search = self.call("asgrep_search", {"query": "alpha_marker"})
        self.assertTrue(search["ok"], search)
        self.assertIn("a.rs", json.dumps(search["value"]))
        read = self.call("asgrep_read", {"refs": ["src/a.rs#L1-L1"]})
        self.assertTrue(read["ok"], read)
        self.assertIn("alpha_marker", json.dumps(read["value"]))
        status = self.call("asgrep_status", {})
        self.assertTrue(status["ok"], status)
        # Persistence: all three calls rode ONE serve child for this root.
        self.assertEqual(len(self.spawned), 1, "expected a single sticky worker")

    def test_missing_binary_fails_closed_with_recovery(self):
        bad = StubContext(
            config={"binary": "/nonexistent/asgrep-nope"}, cwd=self.root
        )
        load_plugin().register(bad)
        raw = bad.tools["asgrep_search"]["handler"]({"query": "x"})
        out = json.loads(raw)
        self.assertFalse(out["ok"])
        self.assertIn("asgrep", out["error"])
        self.assertTrue(
            any(k in out for k in ("recovery", "hint")), "must teach a fix"
        )

    def test_worker_death_restarts_transparently(self):
        first = self.call("asgrep_search", {"query": "beta_helper"})
        self.assertTrue(first["ok"], first)
        worker = self.spawned[0]
        worker.kill()
        worker.wait(timeout=10)
        second = self.call("asgrep_search", {"query": "beta_helper"})
        self.assertTrue(second["ok"], second)
        self.assertEqual(len(self.spawned), 2, "death must respawn exactly once")

    def test_unknown_tool_gets_catalog_pointer(self):
        from asgrep_hermes.serve_client import ServeError, ServePool  # noqa: PLC0415

        pool = ServePool(str(BIN), timeout_s=60)
        try:
            out = pool.call(self.root, "flurbify", {})
            self.assertFalse(out["ok"], out)
            self.assertIn("catalog", json.dumps(out))
        finally:
            pool.close()

    def test_unload_closes_workers(self):
        self.call("asgrep_status", {})
        self.assertEqual(len(self.spawned), 1)
        for fn in self.ctx.unload_hooks:
            fn()
        self.assertIsNotNone(
            self.spawned[0].proc.poll(), "unload must close every worker"
        )


class EagerWrapperTest(unittest.TestCase):
    def test_wrapper_flags_only_our_tools_and_undoes(self):
        saved = {k: sys.modules.get(k) for k in ("tools", "tools.tool_search", "tools.registry", "toolsets")}
        try:
            calls = []

            def original_deferrable(name, *a, **kw):
                calls.append(name)
                return True

            ts = types.ModuleType("tools.tool_search")
            ts.is_deferrable_tool_name = original_deferrable
            tools_pkg = types.ModuleType("tools")
            tools_pkg.tool_search = ts
            registry = types.SimpleNamespace(
                get_tool_names_for_toolset=lambda t: [],
                _generation=1,
            )
            reg_mod = types.ModuleType("tools.registry")
            reg_mod.registry = registry
            toolsets_mod = types.ModuleType("toolsets")
            toolsets_mod._registry_generation = lambda: ("g", 1)
            sys.modules["tools"] = tools_pkg
            sys.modules["tools.tool_search"] = ts
            sys.modules["tools.registry"] = reg_mod
            sys.modules["toolsets"] = toolsets_mod

            from asgrep_hermes.eager import install_eager_tools  # noqa: PLC0415

            undo = install_eager_tools(
                ["asgrep_search", "asgrep_read"], lambda: True
            )
            self.assertIsNotNone(undo)
            self.assertFalse(ts.is_deferrable_tool_name("asgrep_search"))
            self.assertTrue(ts.is_deferrable_tool_name("read_file"))
            self.assertIn(
                "asgrep_read",
                registry.get_tool_names_for_toolset("coding"),
            )
            undo()
            self.assertIs(ts.is_deferrable_tool_name, original_deferrable)
            self.assertEqual(registry.get_tool_names_for_toolset("coding"), [])
        finally:
            for key, mod in saved.items():
                if mod is None:
                    sys.modules.pop(key, None)
                else:
                    sys.modules[key] = mod


if __name__ == "__main__":
    unittest.main(verbosity=2)


class PoolRetryContractTest(unittest.TestCase):
    """retry=False is the takeover's budget guarantee: one attempt, then the
    caller (stock rg) supplies the retry. Default retry=True stays for the
    power lane, where no fallback exists."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        from asgrep_hermes.serve_client import ServePool  # noqa: PLC0415

        self.serve_pool_cls = ServePool

    def tearDown(self):
        self._tmp.cleanup()

    def test_retry_false_is_single_attempt(self):
        import types as _types  # noqa: PLC0415

        from asgrep_hermes.serve_client import ServeError  # noqa: PLC0415

        class StubWorker:
            def __init__(self):
                self.calls = 0
                self.proc = _types.SimpleNamespace(poll=lambda: None)

            def call(self, *a, **k):
                self.calls += 1
                raise ServeError("boom", restartable=True)

            def close(self):
                pass

        pool = self.serve_pool_cls("asgrep", timeout_s=5)
        stub = StubWorker()
        pool._procs[self.root.resolve()] = stub  # pyright: ignore[reportArgumentType]
        try:
            with self.assertRaises(ServeError):
                pool.call(self.root, "search", {}, timeout_s=5, retry=False)
            self.assertEqual(stub.calls, 1, "retry=False must not re-attempt")
        finally:
            pool.close()

    def test_default_retry_attempts_twice(self):
        # Attempt 0: injected stub raises restartable ServeError -> pool drops
        # it and re-attempts on a real child; the child is a garbage-line
        # script, so attempt 1 must end in the malformed-line ServeError (not
        # an escaping ValueError).
        import stat  # noqa: PLC0415
        import types as _types  # noqa: PLC0415

        from asgrep_hermes.serve_client import ServeError  # noqa: PLC0415

        script = self.root / "fake_worker.sh"
        script.write_text("#!/bin/sh\necho 'garbage not json'\nsleep 30\n")
        script.chmod(script.stat().st_mode | stat.S_IEXEC)
        pool = self.serve_pool_cls(str(script), timeout_s=30)

        class StubWorker:
            def __init__(self):
                self.calls = 0
                self.proc = _types.SimpleNamespace(poll=lambda: None)

            def call(self, *a, **k):
                self.calls += 1
                raise ServeError("boom", restartable=True)

            def close(self):
                pass

        stub = StubWorker()
        pool._procs[self.root.resolve()] = stub  # pyright: ignore[reportArgumentType]
        try:
            with self.assertRaises(ServeError) as caught:
                pool.call(self.root, "search", {}, timeout_s=30)
            self.assertEqual(stub.calls, 1)
            self.assertIn("malformed", str(caught.exception),
                          "attempt 1 must surface the child's protocol failure")
        finally:
            pool.close()


class MalformedWorkerLineTest(unittest.TestCase):
    """A worker that answers with a non-JSON line is transport drift, not an
    escaping exception: serve_client's own contract says nothing raises past
    the tool-handler boundary."""

    def test_malformed_line_raises_serve_error(self):
        import stat  # noqa: PLC0415
        import tempfile as _tf  # noqa: PLC0415

        from asgrep_hermes.serve_client import ServeError, ServePool  # noqa: PLC0415

        with _tf.TemporaryDirectory() as tmp:
            root = Path(tmp)
            script = root / "fake_worker.sh"
            script.write_text("#!/bin/sh\necho 'garbage not json'\nsleep 30\n")
            script.chmod(script.stat().st_mode | stat.S_IEXEC)
            pool = ServePool(str(script), timeout_s=30)
            try:
                with self.assertRaises(ServeError):
                    pool.call(root, "search", {}, timeout_s=30)
            finally:
                pool.close()


class DispatchBackstopTest(unittest.TestCase):
    """No exception of any kind may cross the tool-handler boundary; the
    model must always get a structured fail envelope with a recovery hint."""

    def test_unexpected_exception_returns_envelope(self):
        import tempfile as _tf  # noqa: PLC0415

        import asgrep_hermes.serve_client as sc  # noqa: PLC0415

        with _tf.TemporaryDirectory() as tmp:
            ctx = StubContext(cwd=Path(tmp))
            load_plugin().register(ctx)
            original = sc.ServePool.call

            def kaboom(self, *a, **k):
                raise RuntimeError("unexpected plugin failure")

            sc.ServePool.call = kaboom
            try:
                raw = ctx.tools["asgrep_search"]["handler"]({"query": "x"})
                out = json.loads(raw)
                self.assertFalse(out["ok"], out)
                self.assertTrue(any(k in out for k in ("recovery", "hint")),
                                "must still teach a fix")
            finally:
                sc.ServePool.call = original
                for fn in ctx.unload_hooks:
                    fn()


class CloseDeadlockTest(unittest.TestCase):
    """Regression: a child that ignores stdin-close (wedged worker) must not
    hold close() hostage for its lifetime — stdout is closed only after
    kill+reap, so the pump thread's blocked readline can never deadlock it."""

    def test_wedged_child_closes_within_backstop(self):
        import stat  # noqa: PLC0415
        import tempfile as _tf  # noqa: PLC0415
        import time as _time  # noqa: PLC0415

        import asgrep_hermes.serve_client as sc  # noqa: PLC0415

        with _tf.TemporaryDirectory() as tmp:
            root = Path(tmp)
            script = root / "wedged_worker.sh"
            script.write_text("#!/bin/sh\nsleep 60\n")
            script.chmod(script.stat().st_mode | stat.S_IEXEC)
            worker = sc._Worker(str(script), root, 5)
            try:
                started = _time.monotonic()
                worker.close()
                elapsed = _time.monotonic() - started
            finally:
                worker.kill()
            self.assertLess(elapsed, 30,
                            "close() must reap a wedged child, not wait out "
                            "its 60s sleep")


class Round9ParityTest(ToolHandlerTest):
    """Round-9 surface parity: channels, excerpt control, LSP refs."""

    def test_channel_prefixes_route_through_search(self):
        # defs:/callers: are first-class query channels (CLI rounds 5-6):
        # the model never needs a separate tool for symbol lookup.
        defs = self.call("asgrep_search", {"query": "defs:alpha_marker"})
        self.assertTrue(defs["ok"], defs)
        self.assertIn("a.rs", json.dumps(defs["value"]))
        callers = self.call("asgrep_search", {"query": "callers:alpha_marker"})
        self.assertTrue(callers["ok"], callers)

    def test_search_forwards_excerpt_lines_and_semantic_only(self):
        capsule = self.call("asgrep_search", {"query": "alpha_marker"})
        self.assertTrue(capsule["ok"], capsule)
        wide = self.call("asgrep_search", {"query": "alpha_marker", "excerpt_lines": 3})
        self.assertTrue(wide["ok"], wide)
        self.assertGreater(
            len(json.dumps(wide["value"])), len(json.dumps(capsule["value"])),
            "excerpt_lines must widen the returned bodies",
        )
        sem = self.call("asgrep_search", {"query": "alpha_marker", "semantic_only": True})
        self.assertTrue(sem["ok"], sem)
        # All calls rode the same sticky worker.
        self.assertEqual(len(self.spawned), 1)


    def test_search_dispatches_the_fused_engine(self):
        # The schema promises fused keyword/AST search, so the dispatch
        # target must be the codemode `search` tool (fused engine), never
        # `find` (the keyword/grep lane). Discriminator: a multi-word query
        # like "alpha marker" has no contiguous literal match anywhere, so
        # the keyword lane answers EMPTY, while the fused engine resolves
        # it to alpha_marker's def + caller + embed channels.
        res = self.call("asgrep_search", {"query": "alpha marker"})
        self.assertTrue(res["ok"], res)
        hits = res["value"]["hits"]
        self.assertTrue(hits, "fused engine must answer multi-word queries")
        top = hits[0]
        self.assertEqual(top.get("kind"), "def")
        self.assertEqual(top.get("symbol"), "alpha_marker")
        self.assertTrue(str(top.get("file", "")).endswith("a.rs"))

    def test_read_accepts_lsp_colon_refs(self):
        # read is fail-closed on a missing index (search builds ambient, read
        # never mutates) — warm with one search first, as a model would.
        warm = self.call("asgrep_search", {"query": "alpha_marker"})
        self.assertTrue(warm["ok"], warm)
        colon = self.call("asgrep_read", {"refs": ["src/a.rs:1"]})
        self.assertTrue(colon["ok"], colon)
        self.assertIn("alpha_marker", json.dumps(colon["value"]))

    def test_read_dedupes_repeated_refs(self):
        # br-puf failure-first (live 2026-10-28): models repeat refs (search
        # echo + copy) and every duplicate forwarded to serve returned an
        # identical window — paying window budget twice for zero
        # information. The handler dedupes order-preserving before
        # forwarding; the window count equals the unique ref count.
        warm = self.call("asgrep_search", {"query": "alpha_marker"})
        self.assertTrue(warm["ok"], warm)
        res = self.call(
            "asgrep_read",
            {"refs": ["src/a.rs#L1-L1", "src/a.rs:1", "src/a.rs#L1-L1"]},
        )
        self.assertTrue(res["ok"], res)
        windows = res["value"]["windows"]
        self.assertEqual(
            len(windows), 2, "duplicate refs must collapse: " + json.dumps(windows)
        )

    def test_search_limit_clamped_to_schema_max(self):
        # br-m2b failure-first (live 2026-10-27): the bridge does not enforce
        # schema maximums — a limit=500 call returned a 224KB envelope. The
        # handler owns the contract it teaches: clamp to 1..=200 before
        # forwarding. The envelope's limit echo (trustworthy since br-ksx)
        # must report the CLAMPED value, never the oversized ask.
        big = self.call("asgrep_search", {"query": "alpha_marker", "limit": 500})
        self.assertTrue(big["ok"], big)
        self.assertEqual(big["value"].get("limit"), 200, big["value"].get("limit"))
        floor = self.call("asgrep_search", {"query": "alpha_marker", "limit": 0})
        self.assertTrue(floor["ok"], floor)
        self.assertEqual(floor["value"].get("limit"), 1, floor["value"].get("limit"))


class SearchSchemaTeachingTest(unittest.TestCase):
    """br-ao2: the schema must teach pattern forms that ANSWER."""

    def test_query_description_uses_the_answering_pattern_form(self):
        ctx = StubContext(cwd=Path.cwd())
        load_plugin().register(ctx)
        raw = ctx.tools["asgrep_search"]["schema"]
        schema = raw if isinstance(raw, dict) else json.loads(raw)
        desc = schema["parameters"]["properties"]["query"]["description"]
        # The metavar form answers on every corpus (live: 16 hits on the
        # polyglot repo); the concrete `fn name($$$A)` example fail-closed
        # on polyglot repos (rust bodyless concrete answers trait sigs only).
        self.assertIn("'fn $NAME($$$)'", desc)
        self.assertNotIn("fn name($$$A)", desc)


class BinaryResolutionTest(unittest.TestCase):
    """Hosts launch with minimal launchd PATHs that omit ~/.local/bin; a
    bare-name lookup must still find the canonical user-bin target, or the
    fail-closed check_fn silently removes every tool from context (the
    2026-10-24 gateway incident: plugin enabled, tools never surfaced)."""

    def _resolve_with(self, home: Path, path: str):
        from asgrep_hermes import serve_client

        saved = {k: os.environ.get(k) for k in ("HOME", "PATH")}
        os.environ["HOME"] = str(home)
        os.environ["PATH"] = path
        try:
            return serve_client.resolve_binary("asgrep")
        finally:
            for key, val in saved.items():
                if val is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = val

    def test_minimal_path_falls_back_to_local_bin(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            bindir = home / ".local" / "bin"
            bindir.mkdir(parents=True)
            exe = bindir / "asgrep"
            exe.write_text("#!/bin/sh\nexit 0\n")
            exe.chmod(0o755)
            # The desktop gateway's actual PATH: no ~/.local/bin on it.
            self.assertEqual(
                self._resolve_with(home, "/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin"),
                str(exe),
            )

    def test_still_fail_closed_when_nowhere_to_be_found(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(
                self._resolve_with(Path(tmp), "/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin")
            )
