#[cfg(unix)]
const WORKER_MARKER: &str = "ASGREP_WORKER_MARKER";
#[cfg(unix)]
const SUPERVISOR_PID_ENV: &str = "ASGREP_SUPERVISOR_PID";
#[cfg(unix)]
const WORKER_NONCE_ENV: &str = "ASGREP_WORKER_NONCE";
/// Minimum hex chars for a worker nonce (16 random bytes → 32 hex).
#[cfg(unix)]
const WORKER_NONCE_MIN_LEN: usize = 32;
const CPU_LIMIT_ENV: &str = "ASGREP_CPU_LIMIT_PERCENT";
pub const DEFAULT_CPU_LIMIT: u8 = 80;
pub const MIN_CPU_LIMIT: u8 = 1;
pub const MAX_CPU_LIMIT: u8 = 80;
pub const CYCLE_MS: u64 = 10;
#[cfg(unix)]
const THREAD_ENV_VARS: &[&str] = &[
    "OMP_NUM_THREADS",
    "OPENBLAS_NUM_THREADS",
    "MKL_NUM_THREADS",
    "VECLIB_MAXIMUM_THREADS",
    "NUMEXPR_NUM_THREADS",
    "ORT_DISABLE_THREADING",
    "ASGREP_NEURAL_INTRA_THREADS",
    "ASGREP_RERANK_INTRA_THREADS",
];
#[cfg(unix)]
pub fn is_worker() -> bool {
    std::env::var(WORKER_MARKER).is_ok()
}
pub fn cpu_limit_percent() -> u8 {
    parse_cpu_limit(&std::env::var(CPU_LIMIT_ENV).unwrap_or_default())
}
pub fn parse_cpu_limit(raw: &str) -> u8 {
    raw.trim()
        .parse::<u8>()
        .ok()
        .filter(|&p| (MIN_CPU_LIMIT..=MAX_CPU_LIMIT).contains(&p))
        .unwrap_or(DEFAULT_CPU_LIMIT)
}
/// Returns the enforced work/sleep window. Effective service capacity is
/// `mu_effective = mu_raw * work_ms / CYCLE_MS`; operators must keep arrival
/// rate below that capacity or queue latency grows without bound.
pub fn duty_cycle_ms(limit_pct: u8) -> (u64, u64) {
    let work_ms = if limit_pct == 0 {
        0
    } else {
        ((CYCLE_MS * u64::from(limit_pct)) / 100).max(1)
    };
    (work_ms, CYCLE_MS.saturating_sub(work_ms))
}
#[cfg(unix)]
pub fn clear_internal_envs() {
    std::env::remove_var(WORKER_MARKER);
    std::env::remove_var(SUPERVISOR_PID_ENV);
    std::env::remove_var(WORKER_NONCE_ENV);
}
#[cfg(unix)]
pub fn supervise() -> anyhow::Result<()> {
    unix_impl::supervise()
}
#[cfg(unix)]
fn fill_nonce_fallback(bytes: &mut [u8; 16]) {
    // Fallback mix if urandom is unavailable or incomplete (extremely rare).
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut hasher = DefaultHasher::new();
    std::process::id().hash(&mut hasher);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .hash(&mut hasher);
    nix::unistd::getpid().as_raw().hash(&mut hasher);
    // Mix a second round so a failed urandom path is not a fixed all-zero token.
    hasher.write_u64(hasher.finish().wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let a = hasher.finish().to_le_bytes();
    let mut hasher2 = DefaultHasher::new();
    a.hash(&mut hasher2);
    std::thread::current().id().hash(&mut hasher2);
    let b = hasher2.finish().to_le_bytes();
    bytes[..8].copy_from_slice(&a);
    bytes[8..].copy_from_slice(&b);
}

#[cfg(unix)]
fn generate_worker_nonce() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    // Must not ignore a failed/partial read: an all-zero buffer is still 32 hex
    // digits and would pass worker_authenticate's shape check.
    let urandom_ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_ok();
    if !urandom_ok || bytes.iter().all(|&b| b == 0) {
        fill_nonce_fallback(&mut bytes);
    }
    let mut out = String::with_capacity(32);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(unix)]
fn parent_exe_matches(supervisor_pid: i32) -> bool {
    let Some(self_exe) = std::env::current_exe().ok() else {
        return false;
    };
    #[cfg(target_os = "linux")]
    {
        let parent_exe = std::fs::read_link(format!("/proc/{supervisor_pid}/exe")).ok();
        parent_exe.as_ref() == Some(&self_exe)
    }
    #[cfg(target_os = "macos")]
    {
        // No unsafe/libproc (workspace `unsafe_code = forbid`). Best-effort: parent
        // process command name from `ps` must match our binary basename.
        let Some(self_name) = self_exe.file_name().and_then(|s| s.to_str()) else {
            return false;
        };
        let output = std::process::Command::new("/bin/ps")
            .args(["-p", &supervisor_pid.to_string(), "-o", "comm="])
            .output()
            .ok();
        let Some(out) = output.filter(|o| o.status.success()) else {
            return false;
        };
        let comm = String::from_utf8_lossy(&out.stdout);
        let comm = comm.trim();
        // ps may return basename or truncated name; accept prefix/suffix matches.
        !comm.is_empty()
            && (comm == self_name
                || self_name.starts_with(comm)
                || comm.ends_with(self_name)
                || std::path::Path::new(comm)
                    .file_name()
                    .and_then(|s| s.to_str())
                    == Some(self_name))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (supervisor_pid, self_exe);
        // Other Unix: long nonce + parent pid only (see worker_authenticate).
        true
    }
}

#[cfg(unix)]
pub fn worker_authenticate() -> bool {
    if std::env::var(WORKER_MARKER).is_err() {
        return false;
    }
    let fail = || {
        clear_internal_envs();
        false
    };
    let Some(supervisor_pid) = std::env::var(SUPERVISOR_PID_ENV)
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
    else {
        return fail();
    };
    if nix::unistd::getppid().as_raw() != supervisor_pid {
        return fail();
    }
    // Reject constant/"1" nonces: supervisor always emits >= 32 hex chars.
    match std::env::var(WORKER_NONCE_ENV) {
        Ok(ref v)
            if v.len() >= WORKER_NONCE_MIN_LEN && v.bytes().all(|b| b.is_ascii_hexdigit()) => {}
        _ => return fail(),
    }
    if !parent_exe_matches(supervisor_pid) {
        return fail();
    }
    true
}
#[cfg(unix)]
pub fn worker_start() {
    use nix::sys::signal;
    clear_internal_envs();
    // Parent owns process-group setup via CommandExt::process_group (rzzp).
    // Worker only stops for the duty-cycle handshake.
    let _ = signal::raise(signal::Signal::SIGSTOP);
}
#[cfg(unix)]
mod unix_impl {
    use super::*;
    use anyhow::Context;
    use nix::sys::signal::{self, Signal};
    use nix::sys::wait::{self, WaitPidFlag, WaitStatus};
    use nix::unistd::Pid;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    struct SignalSet {
        _ids: [signal_hook::SigId; 5],
        sigint: Arc<AtomicBool>,
        sigterm: Arc<AtomicBool>,
        sighup: Arc<AtomicBool>,
        sigquit: Arc<AtomicBool>,
        tstp: Arc<AtomicBool>,
    }
    impl SignalSet {
        fn install() -> anyhow::Result<Self> {
            fn reg(sig: i32) -> anyhow::Result<(signal_hook::SigId, Arc<AtomicBool>)> {
                let flag = Arc::new(AtomicBool::new(false));
                let id = signal_hook::flag::register(sig, Arc::clone(&flag))
                    .context("register signal handler")?;
                Ok((id, flag))
            }
            let (i0, sigint) = reg(signal_hook::consts::SIGINT)?;
            let (i1, sigterm) = reg(signal_hook::consts::SIGTERM)?;
            let (i2, sighup) = reg(signal_hook::consts::SIGHUP)?;
            let (i3, sigquit) = reg(signal_hook::consts::SIGQUIT)?;
            let (i4, tstp) = reg(signal_hook::consts::SIGTSTP)?;
            Ok(Self {
                _ids: [i0, i1, i2, i3, i4],
                sigint,
                sigterm,
                sighup,
                sigquit,
                tstp,
            })
        }
        fn shutdown_signal(&self) -> i32 {
            [
                (&self.sigterm, signal_hook::consts::SIGTERM),
                (&self.sigint, signal_hook::consts::SIGINT),
                (&self.sigquit, signal_hook::consts::SIGQUIT),
                (&self.sighup, signal_hook::consts::SIGHUP),
            ]
            .into_iter()
            .find(|(f, _)| f.load(Ordering::SeqCst))
            .map(|(_, s)| s)
            .unwrap_or(0)
        }
        fn shutdown_any(&self) -> bool {
            self.shutdown_signal() != 0
        }
    }
    struct ChildGuard {
        child_pid: Pid,
        armed: bool,
    }
    impl ChildGuard {
        fn new(child_pid: Pid) -> Self {
            Self {
                child_pid,
                armed: true,
            }
        }
        fn disarm(&mut self) {
            self.armed = false;
        }
    }
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            // Always reap when still armed (panic unwind, early return, signal exit path).
            // kill_and_reap is signal-safe enough for Drop: best-effort TERM→KILL→wait.
            if self.armed {
                self.armed = false;
                kill_and_reap(self.child_pid);
            }
        }
    }
    pub(super) fn supervise() -> anyhow::Result<()> {
        let (work_ms, sleep_ms) = duty_cycle_ms(cpu_limit_percent());
        let sigs = SignalSet::install()?;
        let mut cmd = std::process::Command::new(std::env::current_exe().context("current_exe")?);
        cmd.args(std::env::args_os().skip(1));
        cmd.env(WORKER_MARKER, "1");
        cmd.env(SUPERVISOR_PID_ENV, std::process::id().to_string());
        cmd.env(WORKER_NONCE_ENV, generate_worker_nonce());
        for var in THREAD_ENV_VARS {
            cmd.env(var, "1");
        }
        cmd.stdin(std::process::Stdio::inherit());
        cmd.stdout(std::process::Stdio::inherit());
        cmd.stderr(std::process::Stdio::inherit());
        // Single owner for process-group setup: child becomes its own PG leader at spawn (rzzp).
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd.spawn().context("failed to spawn worker")?;
        // Pid::from_raw bridges std Child::id() into nix. Child ids are OS PIDs;
        // casting to i32 matches nix's Pid representation on supported unix targets (l115/732x).
        let child_pid = Pid::from_raw(child.id() as i32);
        let mut guard = ChildGuard::new(child_pid);
        wait_for_child_stop(child_pid)?;
        let pgid_neg = Pid::from_raw(-child_pid.as_raw());
        loop {
            // Duty-cycle: SIGCONT for work window, SIGSTOP for sleep window (PR#9).
            if sigs.tstp.swap(false, Ordering::SeqCst) {
                let _ = signal::kill(pgid_neg, Signal::SIGSTOP);
                let _ = signal::raise(Signal::SIGSTOP);
                let _ = signal::kill(pgid_neg, Signal::SIGCONT);
            }
            let _ = signal::kill(pgid_neg, Signal::SIGCONT);
            if !sleep_checking(work_ms, &mut child, child_pid, &sigs)? {
                guard.disarm();
                return Ok(());
            }
            let _ = signal::kill(pgid_neg, Signal::SIGSTOP);
            if !sleep_checking(sleep_ms, &mut child, child_pid, &sigs)? {
                guard.disarm();
                return Ok(());
            }
        }
    }
    fn wait_for_child_stop(child_pid: Pid) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match wait::waitpid(
                child_pid,
                Some(WaitPidFlag::WNOHANG | WaitPidFlag::WUNTRACED),
            ) {
                Ok(WaitStatus::Stopped(_, _)) => return Ok(()),
                Ok(WaitStatus::Exited(_, c)) => std::process::exit(c),
                Ok(WaitStatus::Signaled(_, sig, _)) => {
                    let _ = signal::raise(sig);
                    std::process::exit(128 + sig as i32);
                }
                Ok(WaitStatus::StillAlive) | Err(_) => {
                    if Instant::now() >= deadline {
                        anyhow::bail!("worker child (pid {}) did not stop within 10 s", child_pid);
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Ok(other) => {
                    anyhow::bail!("unexpected worker status while waiting for stop: {other:?}")
                }
            }
        }
    }
    pub(super) fn kill_and_reap(child_pid: Pid) {
        let pgid_neg = Pid::from_raw(-child_pid.as_raw());
        let _ = signal::kill(pgid_neg, Signal::SIGCONT);
        let _ = signal::kill(pgid_neg, Signal::SIGTERM);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(WaitStatus::Exited(_, _) | WaitStatus::Signaled(_, _, _)) =
                wait::waitpid(child_pid, Some(WaitPidFlag::WNOHANG))
            {
                break;
            }
            if Instant::now() >= deadline {
                let _ = signal::kill(pgid_neg, Signal::SIGKILL);
                let _ = signal::kill(child_pid, Signal::SIGKILL);
                if let Ok(WaitStatus::Exited(_, _) | WaitStatus::Signaled(_, _, _)) =
                    wait::waitpid(child_pid, Some(WaitPidFlag::WNOHANG))
                {
                    break;
                }
                let _ = wait::waitpid(child_pid, None);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let drain_end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < drain_end {
            if signal::kill(pgid_neg, Signal::SIGTERM).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
            if Instant::now() >= drain_end {
                break;
            }
            if signal::kill(pgid_neg, Signal::SIGKILL).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    fn exit_shutdown(child_pid: Pid, sigs: &SignalSet) -> ! {
        kill_and_reap(child_pid);
        std::process::exit(128 + sigs.shutdown_signal());
    }
    fn sleep_checking(
        ms: u64,
        child: &mut std::process::Child,
        child_pid: Pid,
        sigs: &SignalSet,
    ) -> anyhow::Result<bool> {
        let end = Instant::now() + Duration::from_millis(ms);
        loop {
            if sigs.shutdown_any() {
                exit_shutdown(child_pid, sigs);
            }
            if let Ok(Some(status)) = child.try_wait() {
                if status.success() {
                    return Ok(false);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if let Some(sig) = status.signal() {
                        std::process::exit(128 + sig);
                    }
                }
                std::process::exit(status.code().unwrap_or(1));
            }
            if Instant::now() >= end {
                return Ok(true);
            }
            std::thread::sleep(
                end.saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(10)),
            );
        }
    }
}

#[cfg(all(test, unix))]
mod childguard_tests {
    use super::unix_impl::*;
    use nix::unistd::Pid;

    // Re-export helpers through a thin test surface: ChildGuard is private inside
    // unix_impl, so we validate public duty-cycle / kill contracts and document
    // Drop semantics in docs/validation/childguard.md (732x).

    #[test]
    fn duty_cycle_respects_cpu_cap() {
        let (work, sleep) = crate::supervisor::duty_cycle_ms(50);
        assert_eq!(work + sleep, crate::supervisor::CYCLE_MS);
        assert!(work > 0 && sleep > 0);
    }

    #[test]
    fn parse_cpu_limit_clamps() {
        assert_eq!(
            crate::supervisor::parse_cpu_limit(""),
            crate::supervisor::DEFAULT_CPU_LIMIT
        );
        assert_eq!(
            crate::supervisor::parse_cpu_limit("0"),
            crate::supervisor::DEFAULT_CPU_LIMIT
        );
        assert_eq!(crate::supervisor::parse_cpu_limit("80"), 80);
        assert_eq!(
            crate::supervisor::parse_cpu_limit("99"),
            crate::supervisor::DEFAULT_CPU_LIMIT
        );
    }

    #[test]
    fn kill_and_reap_tolerates_missing_pid() {
        // Pid 1<<22 is extremely unlikely to exist; must not panic (732x).
        kill_and_reap(Pid::from_raw(1 << 22));
    }

    #[test]
    fn worker_nonce_is_32_hex_and_not_all_zero() {
        let a = super::generate_worker_nonce();
        let b = super::generate_worker_nonce();
        assert_eq!(a.len(), 32, "nonce length");
        assert_eq!(b.len(), 32, "nonce length");
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()), "hex: {a}");
        assert!(b.bytes().all(|c| c.is_ascii_hexdigit()), "hex: {b}");
        assert_ne!(a, "0".repeat(32), "must not emit constant zero nonce");
        assert_ne!(b, "0".repeat(32), "must not emit constant zero nonce");
        // Two draws must differ under /dev/urandom (or mixed fallback entropy).
        assert_ne!(a, b, "successive nonces must not collide");
    }
}
