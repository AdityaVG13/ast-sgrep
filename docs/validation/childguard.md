# ChildGuard / Pid::from_raw (`732x` / `l115`)

Unix supervisor (`crates/ast-sgrep-cli/src/supervisor.rs`):

- `ChildGuard` arms on spawn; `Drop` calls `kill_and_reap` unless `disarm()`ed
  after a clean child exit.
- `Pid::from_raw(child.id() as i32)` is the nix bridge from `std::process::Child`
  PIDs. Negative PGID form `Pid::from_raw(-pid)` targets the process group for
  SIGCONT/SIGTERM/SIGKILL.
- Reap path: SIGTERM → wait with deadline → SIGKILL → blocking wait.
- Signal set: SIGTERM/INT/QUIT/HUP shutdown; SIGTSTP cooperatively stops the
  worker group then the supervisor.

Tests: `supervisor` unit tests under `ast-sgrep-cli` (duty cycle / kill helpers
where platform allows).
