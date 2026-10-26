"""Warm-worker preservation on call timeout (2026-10-26 TensorAST incident).

Contract: a call timeout against a LIVE, BUSY worker must not destroy that
worker's in-flight work (ambient freshness commits only at the end, so a kill
restarts the same multi-minute pass forever). The caller gets a non-restartable
"warming" ServeError instead, the worker keeps running, and the next call on
the same worker succeeds by draining the stale response and matching its own
id. Only after ``warm_grace_s`` of continuous busyness does the timeout revert
to kill+restart semantics (a wedged worker must not poison the pool forever).

Red proof: on 2026-10-26 code, ``_Worker.call`` closes the worker on every
timeout (serve_client.py queue.Empty branch) and the pool restarts it —
``warming`` is never raised, and two workers spawn for the first scenario.
"""

from __future__ import annotations

import json
import stat
import sys
import tempfile
import time
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
PKG = HERE.parent
if str(PKG) not in sys.path:
    sys.path.insert(0, str(PKG))

from asgrep_hermes import serve_client as sc  # noqa: E402
from asgrep_hermes.serve_client import ServeError, ServePool  # noqa: E402

# Speaks the serve NDJSON protocol: answers each request with a result whose
# value echoes the query, delaying ONLY the first request by DELAY seconds.
# Responses stay in request order (single reader loop), matching the real
# worker's in-order contract.
def _slow_shim(delay_s: float) -> str:
    return (
        "#!/usr/bin/env python3\n"
        "import json, sys, time\n"
        f"DELAY = {delay_s!r}\n"
        "first = True\n"
        "for line in sys.stdin:\n"
        "    req = json.loads(line)\n"
        "    if first:\n"
        "        time.sleep(DELAY)\n"
        "        first = False\n"
        "    sys.stdout.write(json.dumps({\"type\": \"result\", \"id\": req[\"id\"], \"ok\": True,\n"
        "        \"value\": {\"echo\": (req.get(\"args\") or {}).get(\"query\", \"\")}}) + \"\\n\")\n"
        "    sys.stdout.flush()\n"
    )

# Reads nothing, answers nothing: a wedged worker.
_WEDGED = "import time\ntime.sleep(600)\n"


def _script(root: Path, name: str, body: str) -> Path:
    path = root / name
    path.write_text(body)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)
    return path


class _SpawnCountingTestCase(unittest.TestCase):
    """Counts every _Worker spawn by patching the module attribute, exactly
    like test_plugin.ToolHandlerTest; pool logic itself is never reimplemented
    here."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.spawned = []
        real_worker = sc._Worker
        spawned = self.spawned

        class CountingWorker(real_worker):
            def __init__(self, *a, **kw):
                super().__init__(*a, **kw)
                spawned.append(self)

        sc._Worker = CountingWorker
        self._real_worker = real_worker

    def tearDown(self):
        sc._Worker = self._real_worker
        self._tmp.cleanup()


class WarmTimeoutPreservesWorkerTest(_SpawnCountingTestCase):
    def test_busy_timeout_warms_then_next_call_rides_same_worker(self):
        shim = _script(self.root, "slow_worker.py", _slow_shim(3.0))
        pool = ServePool(str(shim), timeout_s=1.0)
        pool.warm_grace_s = 60.0
        try:
            with self.assertRaises(ServeError) as caught:
                pool.call(self.root, "search", {"query": "q1"})
            exc = caught.exception
            self.assertTrue(
                getattr(exc, "warming", False),
                f"timeout on a busy worker must be warming, got: {exc}",
            )
            self.assertFalse(
                exc.restartable, "warming must not restart the worker"
            )
            self.assertIn("retry", (exc.recovery or "").lower(), exc.recovery)
            self.assertEqual(len(self.spawned), 1,
                             "warming must not kill the busy worker")
            self.assertIsNone(self.spawned[0].proc.poll(),
                              "worker must stay alive mid-warm")

            # The retry: same worker, drains q1's stale response, matches its
            # own id, and returns q2's own answer.
            out = pool.call(self.root, "search", {"query": "q2"}, timeout_s=30.0)
            self.assertTrue(out["ok"], out)
            self.assertEqual(out["value"]["echo"], "q2",
                             "id-matched drain must not leak a stale answer")
            self.assertEqual(len(self.spawned), 1,
                             "the successful retry must ride the SAME worker")
            self.assertIsNone(self.spawned[0].proc.poll())
        finally:
            pool.close()


class WedgedWorkerEscapesAfterGraceTest(_SpawnCountingTestCase):
    def test_worker_busy_beyond_grace_is_killed_and_replaced(self):
        shim = _script(self.root, "wedged_worker.py",
                       f"#!/usr/bin/env python3\n{_WEDGED}")
        pool = ServePool(str(shim), timeout_s=0.5)
        pool.warm_grace_s = 1.5
        try:
            with self.assertRaises(ServeError) as first:
                pool.call(self.root, "search", {"query": "q1"})
            self.assertTrue(getattr(first.exception, "warming", False))
            self.assertEqual(len(self.spawned), 1, "inside grace: keep alive")

            time.sleep(1.6)  # cross warm_grace_s on the same busy worker
            with self.assertRaises(ServeError):
                pool.call(self.root, "search", {"query": "q2"})
            self.assertGreaterEqual(
                len(self.spawned), 2,
                "beyond grace the busy worker must be killed and replaced",
            )
            self.assertIsNotNone(
                self.spawned[0].proc.poll(), "the stale worker must be dead"
            )
        finally:
            pool.close()


if __name__ == "__main__":
    unittest.main(verbosity=2)
