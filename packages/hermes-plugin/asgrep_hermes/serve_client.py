"""Persistent `asgrep serve` worker management for the Hermes plugin.

One sticky NDJSON child per repository root. The child's lifetime is bound to
this process: it exits when its stdin pipe closes (host death included), and
on_unload/atexit close it on normal plugin unload. Everything structured —
transport failures carry a recovery hint and never raise past the tool
handler boundary.
"""

from __future__ import annotations

import json
import os
import queue
import shutil
import signal
import subprocess
import threading
import time
from contextlib import suppress
from pathlib import Path
from typing import Any, Optional


class ServeError(Exception):
    """Transport-level failure talking to an `asgrep serve` worker."""

    def __init__(self, message: str, *, recovery: Optional[str] = None,
                 restartable: bool = False, warming: bool = False,
                 elapsed_s: Optional[float] = None) -> None:
        super().__init__(message)
        self.message = message
        self.recovery = recovery
        self.restartable = restartable
        # Warming: the worker is alive and busy (typically ambient index
        # freshness). NOT restartable — killing it would destroy in-flight
        # work that commits only at the end.
        self.warming = warming
        self.elapsed_s = elapsed_s


def resolve_binary(configured: str = "asgrep") -> Optional[str]:
    """Resolve the asgrep executable. Path-like values must exist and be
    executable; bare names resolve via PATH, then the user-bin convention
    ``~/.local/bin/<name>`` (GUI hosts launch with minimal launchd PATHs that
    omit it). None = unavailable (fail closed)."""
    configured = configured or "asgrep"
    candidate = Path(configured)
    if candidate.is_absolute() or "/" in configured or os.sep in configured:
        return str(candidate) if candidate.is_file() and os.access(candidate, os.X_OK) else None
    found = shutil.which(configured)
    if found:
        return found
    fallback = Path.home() / ".local" / "bin" / configured
    return str(fallback) if fallback.is_file() and os.access(fallback, os.X_OK) else None


class _Worker:
    """One sticky NDJSON child; a single in-flight request at a time.

    Responses are read by a daemon thread into a queue; the single-writer
    protocol (`asgrep serve` answers one request per line, in order) makes
    id checking a cheap integrity assertion rather than routing logic.
    """

    def __init__(self, binary: str, root: Path, timeout_s: float,
                 warm_grace_s: float = 900.0) -> None:
        self.root = root
        self.timeout_s = timeout_s
        # How long a continuously-busy worker is kept alive past call
        # timeouts before the pool falls back to kill+restart. Ambient
        # freshness measured 400s on a 63k-file tree (2026-10-26); the
        # default accommodates several times that.
        self.warm_grace_s = warm_grace_s
        self.proc = subprocess.Popen(
            [binary, "serve", "--root", str(root)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            cwd=str(root),
            # Own session: kill() must be able to take the whole group — a
            # worker that forks helpers must not leave orphans holding the
            # stdout pipe (they would pin the pump thread's readline, and
            # with it close()/atexit).
            start_new_session=True,
        )
        self._lines: queue.Queue = queue.Queue()
        self._seq = 0
        self._call_lock = threading.Lock()
        # Requests written but not yet answered. The single-writer protocol
        # answers one request per line, in order; a caller that returns
        # warming leaves its request outstanding, so later calls must drain
        # those stale responses (id-keyed) before their own arrives.
        self._awaited: set[str] = set()
        self._busy_since: Optional[float] = None
        stream = self.proc.stdout
        assert stream is not None  # PIPE was requested
        self._reader = threading.Thread(target=self._pump, args=(stream,), daemon=True)
        self._reader.start()

    def _forget(self, rid: str) -> None:
        self._awaited.discard(rid)
        if not self._awaited:
            self._busy_since = None

    def _pump(self, stream) -> None:
        try:
            for line in stream:
                self._lines.put(line)
        finally:
            self._lines.put(None)  # EOF sentinel

    def call(self, tool: str, args: dict, timeout_s: Optional[float] = None) -> dict:
        with self._call_lock:
            timeout = timeout_s or self.timeout_s
            self._seq += 1
            rid = f"h{self._seq}"
            request = json.dumps(
                {"type": "call", "id": rid, "tool": tool, "args": args},
                separators=(",", ":"),
            )
            try:
                assert self.proc.stdin is not None
                self.proc.stdin.write(request.encode() + b"\n")
                self.proc.stdin.flush()
            except (BrokenPipeError, OSError) as exc:
                raise ServeError(
                    f"asgrep serve worker is gone: {exc}",
                    recovery="The call is retried on a fresh worker.",
                    restartable=True,
                ) from exc
            if not self._awaited:
                self._busy_since = time.monotonic()
            self._awaited.add(rid)
            deadline = time.monotonic() + timeout
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    busy_elapsed = time.monotonic() - (self._busy_since or time.monotonic())
                    if busy_elapsed < self.warm_grace_s:
                        # Live worker, busy past this call's timeout — almost
                        # always ambient index freshness, whose work commits
                        # only at the end. Killing here would restart the
                        # whole pass on every timeout (2026-10-26 TensorAST
                        # incident: a 400s rebuild, never able to finish
                        # under a 120s timeout). Keep the worker; the caller
                        # retries and rides its progress.
                        raise ServeError(
                            f"asgrep serve call still running: worker busy for "
                            f"{busy_elapsed:.0f}s (warming — in-flight index "
                            f"work is preserved)",
                            recovery="Retry the same call shortly; progress is "
                                     "kept on the live worker. It restarts "
                                     "automatically only if busy beyond the "
                                     "grace window.",
                            restartable=False,
                            warming=True,
                            elapsed_s=busy_elapsed,
                        )
                    self.close()
                    raise ServeError(
                        "asgrep serve call timed out",
                        recovery="Raise the plugin timeout_s, or warm the index: "
                                 "`asgrep index <root> --json`",
                        restartable=True,
                    ) from None
                try:
                    line = self._lines.get(timeout=remaining)
                except queue.Empty:
                    continue  # re-check the deadline (warm vs hard branch)
                if line is None:
                    raise ServeError(
                        "asgrep serve worker exited",
                        recovery="The call is retried on a fresh worker.",
                        restartable=True,
                    )
                try:
                    response = json.loads(line)
                except ValueError:
                    # Non-JSON stdout is protocol drift: transport-level, restart
                    # semantics — never an exception past the handler boundary.
                    raise ServeError(
                        "serve worker emitted a malformed line (protocol drift)",
                        recovery="The call is retried on a fresh worker.",
                        restartable=True,
                    ) from None
                kind = response.get("type")
                if kind not in ("result", "error"):
                    raise ServeError(
                        f"unexpected serve response type {kind!r}",
                        restartable=True,
                    )
                rid_line = response.get("id")
                if rid_line == rid:
                    self._forget(rid)
                    if kind == "error":
                        return {"ok": False, "error": response.get("error", "serve error")}
                    return {
                        "ok": bool(response.get("ok")),
                        "value": response.get("value"),
                        "error": response.get("error"),
                    }
                if rid_line in self._awaited:
                    # Stale answer for an earlier request whose caller already
                    # returned warming: drop it and keep waiting for ours.
                    self._forget(rid_line)
                    continue
                raise ServeError(
                    f"unexpected serve response id {rid_line!r} (protocol drift)",
                    restartable=True,
                )

    def kill(self) -> None:
        # Group kill first: a forked grandchild still holding the stdout
        # pipe would otherwise outlive the direct child and pin the pump.
        with suppress(Exception):
            os.killpg(os.getpgid(self.proc.pid), signal.SIGKILL)
        with suppress(Exception):
            self.proc.kill()

    def wait(self, timeout: float = 10) -> None:
        with suppress(Exception):
            self.proc.wait(timeout=timeout)

    def close(self) -> None:
        self._awaited.clear()
        self._busy_since = None
        # Graceful: closing stdin ends the serve read loop. The kill/reap
        # backstop runs BEFORE closing stdout — the pump thread's blocked
        # readline holds the stream lock, so closing stdout while anything
        # still holds the pipe's write end (direct child or a forked
        # grandchild; hence the group kill) can deadlock close() until that
        # process exits on its own — a wedged child would hang
        # pool.close()/atexit for its full lifetime.
        with suppress(Exception):
            if self.proc.stdin is not None:
                self.proc.stdin.close()
        with suppress(Exception):
            self.proc.wait(timeout=5)
        self.kill()
        with suppress(Exception):
            self.proc.wait(timeout=5)
        with suppress(Exception):
            if self.proc.stdout is not None:
                self.proc.stdout.close()


class ServePool:
    """Root-keyed pool of sticky workers with one transparent restart."""

    def __init__(self, binary: str, timeout_s: float = 120.0,
                 warm_grace_s: float = 900.0) -> None:
        self.binary = binary
        self.timeout_s = timeout_s
        # Read at spawn time, so operators (and tests) may retune it on a
        # live pool (same seam as `pool.binary`).
        self.warm_grace_s = warm_grace_s
        self._lock = threading.Lock()
        self._procs: dict[Path, _Worker] = {}

    def call(self, root: Path, tool: str, args: dict,
             timeout_s: Optional[float] = None, *,
             retry: bool = True) -> dict[str, Any]:
        """Call ``tool`` on the worker for ``root``.

        ``retry=False`` disables the one transparent restart: the caller
        supplies its own fallback (the takeover hands off to stock rg inside
        a fixed budget — a re-attempt here would spend that budget twice).
        """
        root = Path(root).resolve()
        for attempt in (0, 1):
            with self._lock:
                worker = self._procs.get(root)
                if worker is None or worker.proc.poll() is not None:
                    if worker is not None:
                        worker.close()
                    try:
                        worker = _Worker(self.binary, root, self.timeout_s,
                                         self.warm_grace_s)
                    except OSError as exc:
                        raise ServeError(
                            f"cannot start asgrep serve ({exc})",
                            recovery=f"Check the configured binary `{self.binary}`; "
                                     "`asgrep doctor --robot-triage` diagnoses installs.",
                        ) from exc
                    self._procs[root] = worker
            try:
                return worker.call(tool, args, timeout_s)
            except ServeError as exc:
                if not exc.restartable or attempt or not retry:
                    raise
                with self._lock:
                    dead = self._procs.pop(root, None)
                if dead is not None:
                    dead.close()
        raise AssertionError("unreachable")  # pragma: no cover

    def has_worker(self, root: Path) -> bool:
        """True when a live worker is already held for ``root`` (no spawn)."""
        root = Path(root).resolve()
        with self._lock:
            worker = self._procs.get(root)
            return worker is not None and worker.proc.poll() is None

    def close(self) -> None:
        with self._lock:
            workers = list(self._procs.values())
            self._procs.clear()
        for worker in workers:
            worker.close()
