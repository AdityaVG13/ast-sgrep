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
import subprocess
import threading
from contextlib import suppress
from pathlib import Path
from typing import Any, Optional


class ServeError(Exception):
    """Transport-level failure talking to an `asgrep serve` worker."""

    def __init__(self, message: str, *, recovery: Optional[str] = None,
                 restartable: bool = False) -> None:
        super().__init__(message)
        self.message = message
        self.recovery = recovery
        self.restartable = restartable


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

    def __init__(self, binary: str, root: Path, timeout_s: float) -> None:
        self.root = root
        self.timeout_s = timeout_s
        self.proc = subprocess.Popen(
            [binary, "serve", "--root", str(root)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            cwd=str(root),
        )
        self._lines: queue.Queue = queue.Queue()
        self._seq = 0
        self._call_lock = threading.Lock()
        stream = self.proc.stdout
        assert stream is not None  # PIPE was requested
        self._reader = threading.Thread(target=self._pump, args=(stream,), daemon=True)
        self._reader.start()

    def _pump(self, stream) -> None:
        try:
            for line in stream:
                self._lines.put(line)
        finally:
            self._lines.put(None)  # EOF sentinel

    def call(self, tool: str, args: dict, timeout_s: Optional[float] = None) -> dict:
        with self._call_lock:
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
            try:
                line = self._lines.get(timeout=timeout_s or self.timeout_s)
            except queue.Empty:
                self.close()
                raise ServeError(
                    "asgrep serve call timed out",
                    recovery="Raise the plugin timeout_s, or warm the index: "
                             "`asgrep index <root> --json`",
                    restartable=True,
                ) from None
            if line is None:
                raise ServeError(
                    "asgrep serve worker exited",
                    recovery="The call is retried on a fresh worker.",
                    restartable=True,
                )
            response = json.loads(line)
            kind = response.get("type")
            if kind == "result":
                if response.get("id") != rid:
                    raise ServeError(
                        "serve response id mismatch (protocol drift)",
                        restartable=True,
                    )
                return {
                    "ok": bool(response.get("ok")),
                    "value": response.get("value"),
                    "error": response.get("error"),
                }
            if kind == "error":
                return {"ok": False, "error": response.get("error", "serve error")}
            raise ServeError(
                f"unexpected serve response type {kind!r}",
                restartable=True,
            )

    def kill(self) -> None:
        with suppress(Exception):
            self.proc.kill()

    def wait(self, timeout: float = 10) -> None:
        with suppress(Exception):
            self.proc.wait(timeout=timeout)

    def close(self) -> None:
        # Graceful: closing stdin ends the serve read loop; kill is the backstop.
        with suppress(Exception):
            if self.proc.stdin is not None:
                self.proc.stdin.close()
        with suppress(Exception):
            if self.proc.stdout is not None:
                self.proc.stdout.close()
        with suppress(Exception):
            self.proc.wait(timeout=5)
        with suppress(Exception):
            self.proc.kill()


class ServePool:
    """Root-keyed pool of sticky workers with one transparent restart."""

    def __init__(self, binary: str, timeout_s: float = 120.0) -> None:
        self.binary = binary
        self.timeout_s = timeout_s
        self._lock = threading.Lock()
        self._procs: dict[Path, _Worker] = {}

    def call(self, root: Path, tool: str, args: dict,
             timeout_s: Optional[float] = None) -> dict[str, Any]:
        root = Path(root).resolve()
        for attempt in (0, 1):
            with self._lock:
                worker = self._procs.get(root)
                if worker is None or worker.proc.poll() is not None:
                    if worker is not None:
                        worker.close()
                    try:
                        worker = _Worker(self.binary, root, self.timeout_s)
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
                if not exc.restartable or attempt:
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
