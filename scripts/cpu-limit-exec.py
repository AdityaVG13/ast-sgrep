#!/usr/bin/env python3
"""cpu-limit-exec.py — duty-cycle CPU limiter via process-group STOP/CONT.

Usage: cpu-limit-exec.py [--limit 0-80] [--cycle-ms MS] -- cmd [args...]

Architecture:
  Parent forks. Child creates a new session, SIGSTOPs itself, then
  execs the command when CONT'd. Parent detects the stop via
  waitpid(WUNTRACED), sends CONT, then enters a duty-cycle loop.

  Duty cycle (default 10 ms):
    work_ms  = max(1, floor(cycle_ms * limit% / 100))
    on_time  = work_ms / 1000                (child runs)
    off_time = (cycle_ms - work_ms) / 1000   (child stopped)
  This matches the production Rust supervisor millisecond quantization.

  Signals (INT/TERM/HUP/QUIT) are handled via a pending-signum flag.
  On signal: CONT child if stopped, forward signal, grace-wait 5 s,
  then SIGKILL the process group.  Always exits 128+signal.

  SIGTSTP (Ctrl-Z): stops child process group, then self-SIGSTOP
  (uncatchable).  After SIGCONT, the wrapper resumes its duty cycle
  with the child still stopped until the next controlled ON phase.
  TSTP is never forwarded to the child.

Contracts:
  - Default 80 %, hard cap 80 %.
  - Cycle period 10 ms (on + off).
  - Child is in its own session (setsid); STOP/CONT via killpg.
  - stdin/stdout/stderr are inherited; exit code is propagated.
  - On termination signal: always exit 128+signal.
  - After direct child exits: TERM process group, grace wait, KILL
    survivors in a loop until ESRCH.
    - SIGKILL safety net: a detached reaper grandchild watches the
    supervisor pid and killpg's the payload session if the supervisor
    dies without reaping (payloads can never be orphaned under PPID 1).
    - A payload with its own duty limiter receives multiplicative capacity;
    rustc-capped is for compiler/build payloads, not production asgrep.
"""

import argparse
import errno
import os
import signal
import sys
import time

_pending_signal = 0
_child_exit_status = None  # saved when _sleep_check reaps the child
_child_pgid = 0             # set in parent after fork; used by TSTP handler

# -- orphan reaper -----------------------------------------------------------
# If this wrapper is SIGKILL'd (or dies without reaping), the payload is
# reparented to launchd (PPID 1) with no limiter running. A detached reaper
# grandchild watches the supervisor pid and kills the payload's process
# group the moment the supervisor disappears. This closes the reap hole
# that left orphaned CPU saturators burning under PPID 1.

_REAPER_POLL_S = 0.05       # 50 ms supervisor-liveness poll
_REAPER_GRACE_S = 0.5       # TERM-to-KILL grace


def _spawn_orphan_reaper(supervisor_pid, payload_pgid):
    """Fork a detached reaper that killpg's *payload_pgid* when the
    supervisor dies. Best-effort: fork failure just skips the safety net.

    Called in the parent after the payload fork; the payload calls
    setsid() making its pid == its pgid, which is what we pass in.
    """
    try:
        if os.fork() > 0:
            return
    except OSError:
        return
    # Grandchild: fork again and exit so the reaper is never our zombie,
    # then detach into its own session.
    try:
        if os.fork() > 0:
            os._exit(0)
    except OSError:
        os._exit(0)
    try:
        os.setsid()
    except OSError:
        pass
    deadline = time.monotonic() + 86400  # 24 h hard ceiling
    # Wait for the payload to setsid() so its pgid exists before we probe it
    # (reaper may run before the child reaches setsid). 5 s is generous.
    setup_deadline = time.monotonic() + 5
    while time.monotonic() < setup_deadline:
        try:
            os.killpg(payload_pgid, 0)
            break
        except OSError:
            try:
                os.kill(supervisor_pid, 0)
            except OSError:
                os._exit(0)  # supervisor died before payload setup
            time.sleep(_REAPER_POLL_S)
    while time.monotonic() < deadline:
        try:
            os.kill(supervisor_pid, 0)
        except OSError:
            break  # supervisor gone -> reap the payload group
        try:
            os.killpg(payload_pgid, 0)
        except OSError:
            os._exit(0)  # payload group already empty, nothing to reap
        time.sleep(_REAPER_POLL_S)
    try:
        os.killpg(payload_pgid, signal.SIGTERM)
    except OSError:
        os._exit(0)
    time.sleep(_REAPER_GRACE_S)
    try:
        os.killpg(payload_pgid, signal.SIGKILL)
    except OSError:
        pass
    os._exit(0)


# -- safe signal handlers --------------------------------------------------

def _on_signal(signum, _frame):
    global _pending_signal
    _pending_signal = signum


def _on_tstp(signum, _frame):
    """Stop child process group, then self-stop (uncatchable).

    After SIGCONT the kernel resumes us right here; we return to the
    duty cycle with the child still stopped until the next controlled
    ON phase.  TSTP is never forwarded to the child.
    """
    global _child_pgid
    if _child_pgid:
        try:
            os.killpg(_child_pgid, signal.SIGSTOP)
        except (ProcessLookupError, OSError):
            pass
    # Self-STOP — uncatchable at the kernel level.
    os.kill(os.getpid(), signal.SIGSTOP)
    # Resumed by SIGCONT; fall through back to the duty cycle.


def _install_signal_handlers():
    for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT):
        signal.signal(sig, _on_signal)
    signal.signal(signal.SIGTSTP, _on_tstp)


# -- helpers ---------------------------------------------------------------

def _exit_code_from_status(status):
    """Convert a waitpid status to a process exit code."""
    if os.WIFEXITED(status):
        return os.WEXITSTATUS(status)
    if os.WIFSIGNALED(status):
        return 128 + os.WTERMSIG(status)
    return 1


def _sleep_check(seconds, child_pid):
    """Sleep *seconds* in 1 ms slices.

    Returns True if a signal is pending OR the child exited, so the
    caller can break out of the duty cycle early.  Saves the exit
    status in _child_exit_status when the child is reaped.
    """
    global _child_exit_status
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if _pending_signal:
            return True
        try:
            wpid, wst = os.waitpid(child_pid, os.WNOHANG)
            if wpid == child_pid:
                _child_exit_status = wst
                return True
        except ChildProcessError:
            return True
        remaining = end - time.monotonic()
        if remaining <= 0:
            break
        time.sleep(min(remaining, 0.001))
    return bool(_pending_signal)


def _kill_survivors(pgid, term_grace=2.0):
    """TERM the process group, wait *term_grace* s, then KILL loop until ESRCH.

    Called after the direct child has been reaped, to clean up any
    descendants that are still alive (e.g. grandchildren that ignored
    the forwarded signal).
    """
    try:
        os.killpg(pgid, signal.SIGTERM)
    except (ProcessLookupError, OSError):
        return

    deadline = time.monotonic() + term_grace
    while time.monotonic() < deadline:
        time.sleep(0.05)

    # KILL loop — exit when no process in the group remains.
    for _ in range(200):                     # safety bound ≈ 10 s
        try:
            os.killpg(pgid, signal.SIGKILL)
        except ProcessLookupError:
            return
        except OSError as e:
            if e.errno == errno.ESRCH:
                return
            break                            # unexpected error
        time.sleep(0.05)


def _signal_forward_and_reap(pid, sig, grace_s=5.0):
    """Forward *sig* to the process group, reap child, kill survivors.

    Always exits with 128 + sig (preserving the requested signal).
    """
    # CONT in case child is stopped
    try:
        os.killpg(pid, signal.SIGCONT)
    except (ProcessLookupError, OSError):
        pass
    try:
        os.killpg(pid, sig)
    except (ProcessLookupError, OSError):
        pass

    # Wait for the direct child to exit (grace period).
    grace_end = time.monotonic() + grace_s
    while time.monotonic() < grace_end:
        try:
            wpid, wst = os.waitpid(pid, os.WNOHANG)
            if wpid == pid:
                break
        except ChildProcessError:
            break
        time.sleep(0.05)

    # Clean up any survivors in the process group.
    _kill_survivors(pid)

    sys.exit(128 + sig)


def _waitpid_retry(pid, options):
    """waitpid wrapper that retries on EINTR (InterruptedError)."""
    while True:
        try:
            return os.waitpid(pid, options)
        except InterruptedError:
            continue


# -- duty-cycle contract ---------------------------------------------------

def duty_cycle_seconds(limit, cycle_ms):
    """Return Rust-supervisor-compatible work/sleep quanta in seconds.

    A zero limit retains a 0.1 ms bootstrap quantum so the child can exec and
    exit; configured production limits are in the inclusive range 1..=80.
    """
    work_ms = 0.1 if limit == 0 else max(1, cycle_ms * limit // 100)
    return work_ms / 1000.0, (cycle_ms - work_ms) / 1000.0


# -- main ------------------------------------------------------------------

def main():
    global _pending_signal, _child_exit_status, _child_pgid

    ap = argparse.ArgumentParser(description="CPU duty-cycle limiter")
    ap.add_argument(
        "--limit", type=int, default=80,
        help="CPU limit percent 0-80 (default 80, hard cap 80)",
    )
    ap.add_argument(
        "--cycle-ms", type=int, default=10,
        help="Duty cycle period in ms (default 10)",
    )
    ap.add_argument(
        "--trace-file", type=str, default=None,
        help="If set, touch this file on startup (integration-test aid)",
    )
    ap.add_argument(
        "cmd", nargs=argparse.REMAINDER,
        help="Command to run (after -- )",
    )
    args = ap.parse_args()

    limit = max(0, min(80, args.limit))
    cycle_ms = max(1, args.cycle_ms)
    cmd = args.cmd
    # argparse REMAINDER keeps the '--' token when present -- strip it
    if cmd and cmd[0] == "--":
        cmd = cmd[1:]
    if not cmd:
        print("cpu-limit-exec: no command specified", file=sys.stderr)
        sys.exit(1)

    if args.trace_file:
        try:
            with open(args.trace_file, "a") as tf:
                tf.write(f"pid={os.getpid()}\\n")
        except OSError:
            pass

    _install_signal_handlers()

    on_time, off_time = duty_cycle_seconds(limit, cycle_ms)

    # -- fork ----------------------------------------------------------
    pid = os.fork()
    if pid == 0:
        # --- child ---
        os.setsid()                            # new session + process group
        os.kill(os.getpid(), signal.SIGSTOP)   # wait for parent handshake
        # After CONT: exec the payload
        os.execvp(cmd[0], cmd)
        os._exit(127)

    # -- parent: publish child pgid for TSTP handler -------------------
    _child_pgid = pid

    # -- orphan reaper: if we are SIGKILL'd, the detached reaper kills the
    # payload's process group (child called setsid -> pid == pgid) instead of
    # leaving it orphaned under launchd.
    _spawn_orphan_reaper(os.getpid(), pid)

    # -- parent: bootstrap handshake -----------------------------------
    try:
        _wpid, status = _waitpid_retry(pid, os.WUNTRACED)
    except ChildProcessError:
        sys.exit(0)
    if os.WIFEXITED(status) or os.WIFSIGNALED(status):
        sys.exit(_exit_code_from_status(status))

    # Child is stopped -- CONT so it execs
    try:
        os.kill(pid, signal.SIGCONT)
    except (ProcessLookupError, OSError):
        pass

    # -- duty cycle ----------------------------------------------------
    try:
        while not _pending_signal:
            # --- ON phase: let child run ---
            if _sleep_check(on_time, pid):
                break

            # --- OFF phase: stop child ---
            try:
                os.killpg(pid, signal.SIGSTOP)
            except (ProcessLookupError, OSError):
                break
            # Wait for stop to take effect
            try:
                _wpid, status = _waitpid_retry(pid, os.WUNTRACED)
                if os.WIFEXITED(status) or os.WIFSIGNALED(status):
                    sys.exit(_exit_code_from_status(status))
            except ChildProcessError:
                break

            if _pending_signal:
                break

            if _sleep_check(off_time, pid):
                break

            # --- CONT for next on-phase ---
            try:
                os.killpg(pid, signal.SIGCONT)
            except (ProcessLookupError, OSError):
                break

    except (ProcessLookupError, ChildProcessError, OSError):
        pass

    # -- signal forwarding ---------------------------------------------
    if _pending_signal:
        _signal_forward_and_reap(pid, _pending_signal, grace_s=5.0)
        # _signal_forward_and_reap calls sys.exit -- unreachable here
        return

    # -- normal exit: reap child, clean up process group ---------------
    if _child_exit_status is not None:
        exit_code = _exit_code_from_status(_child_exit_status)
    else:
        exit_code = 0
        try:
            while True:
                wpid, wst = _waitpid_retry(pid, 0)
                if wpid == pid:
                    exit_code = _exit_code_from_status(wst)
                    break
        except ChildProcessError:
            pass

    # Kill any remaining descendants in the process group
    _kill_survivors(pid)

    sys.exit(exit_code)


if __name__ == "__main__":
    main()
