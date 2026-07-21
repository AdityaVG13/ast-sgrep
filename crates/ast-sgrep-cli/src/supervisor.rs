#[cfg(unix)] const WORKER_MARKER: &str = "ASGREP_WORKER_MARKER";
#[cfg(unix)] const SUPERVISOR_PID_ENV: &str = "ASGREP_SUPERVISOR_PID";
#[cfg(unix)] const WORKER_NONCE_ENV: &str = "ASGREP_WORKER_NONCE";
const CPU_LIMIT_ENV: &str = "ASGREP_CPU_LIMIT_PERCENT";
pub const DEFAULT_CPU_LIMIT: u8 = 80; pub const MIN_CPU_LIMIT: u8 = 1; pub const MAX_CPU_LIMIT: u8 = 80; pub const CYCLE_MS: u64 = 10;
#[cfg(unix)]
const THREAD_ENV_VARS: &[&str] = &["OMP_NUM_THREADS","OPENBLAS_NUM_THREADS","MKL_NUM_THREADS","VECLIB_MAXIMUM_THREADS","NUMEXPR_NUM_THREADS","ORT_DISABLE_THREADING","ASGREP_NEURAL_INTRA_THREADS","ASGREP_RERANK_INTRA_THREADS"];
#[cfg(unix)] pub fn is_worker() -> bool { std::env::var(WORKER_MARKER).is_ok() }
pub fn cpu_limit_percent() -> u8 { parse_cpu_limit(&std::env::var(CPU_LIMIT_ENV).unwrap_or_default()) }
pub fn parse_cpu_limit(raw: &str) -> u8 {
    raw.trim().parse::<u8>().ok().filter(|&p| (MIN_CPU_LIMIT..=MAX_CPU_LIMIT).contains(&p)).unwrap_or(DEFAULT_CPU_LIMIT)
}
/// Returns the enforced work/sleep window. Effective service capacity is
/// `mu_effective = mu_raw * work_ms / CYCLE_MS`; operators must keep arrival
/// rate below that capacity or queue latency grows without bound.
pub fn duty_cycle_ms(limit_pct: u8) -> (u64, u64) {
    let work_ms = if limit_pct == 0 { 0 } else { ((CYCLE_MS * u64::from(limit_pct)) / 100).max(1) };
    (work_ms, CYCLE_MS.saturating_sub(work_ms))
}
#[cfg(unix)] pub fn clear_internal_envs() { std::env::remove_var(WORKER_MARKER); std::env::remove_var(SUPERVISOR_PID_ENV); std::env::remove_var(WORKER_NONCE_ENV); }
#[cfg(unix)] pub fn supervise() -> anyhow::Result<()> { unix_impl::supervise() }
#[cfg(unix)]
pub fn worker_authenticate() -> bool {
    if std::env::var(WORKER_MARKER).is_err() { return false; }
    let fail = || { clear_internal_envs(); false };
    let Some(supervisor_pid) = std::env::var(SUPERVISOR_PID_ENV).ok().and_then(|v| v.parse::<i32>().ok()) else { return fail(); };
    if nix::unistd::getppid().as_raw() != supervisor_pid { return fail(); }
    match std::env::var(WORKER_NONCE_ENV) { Ok(ref v) if !v.is_empty() => {} _ => return fail() }
    #[cfg(target_os = "linux")] {
        let parent_exe = std::fs::read_link(format!("/proc/{supervisor_pid}/exe")).ok(); let self_exe = std::env::current_exe().ok();
        if !matches!((parent_exe, self_exe), (Some(p), Some(s)) if p == s) { return fail(); }
    }
    true
}
#[cfg(unix)]
pub fn worker_start() {
    use nix::sys::signal; use nix::unistd::{self, Pid};
    clear_internal_envs(); let _ = unistd::setpgid(Pid::this(), Pid::this()); let _ = signal::raise(signal::Signal::SIGSTOP);
}
#[cfg(unix)]
mod unix_impl {
    use super::*; use anyhow::Context; use nix::sys::signal::{self, Signal}; use nix::sys::wait::{self, WaitPidFlag, WaitStatus};
    use nix::unistd::Pid; use std::sync::atomic::{AtomicBool, Ordering}; use std::sync::Arc; use std::time::{Duration, Instant};
    struct SignalSet { _ids: [signal_hook::SigId; 5], sigint: Arc<AtomicBool>, sigterm: Arc<AtomicBool>, sighup: Arc<AtomicBool>, sigquit: Arc<AtomicBool>, tstp: Arc<AtomicBool> }
    impl SignalSet {
        fn install() -> anyhow::Result<Self> {
            fn reg(sig: i32) -> anyhow::Result<(signal_hook::SigId, Arc<AtomicBool>)> {
                let flag = Arc::new(AtomicBool::new(false)); let id = signal_hook::flag::register(sig, Arc::clone(&flag)).context("register signal handler")?; Ok((id, flag))
            }
            let (i0, sigint) = reg(signal_hook::consts::SIGINT)?; let (i1, sigterm) = reg(signal_hook::consts::SIGTERM)?;
            let (i2, sighup) = reg(signal_hook::consts::SIGHUP)?; let (i3, sigquit) = reg(signal_hook::consts::SIGQUIT)?; let (i4, tstp) = reg(signal_hook::consts::SIGTSTP)?;
            Ok(Self { _ids: [i0, i1, i2, i3, i4], sigint, sigterm, sighup, sigquit, tstp })
        }
        fn shutdown_any(&self) -> bool {
            self.sigterm.load(Ordering::SeqCst) || self.sigint.load(Ordering::SeqCst) || self.sigquit.load(Ordering::SeqCst) || self.sighup.load(Ordering::SeqCst)
        }
        fn shutdown_signal(&self) -> i32 {
            if self.sigterm.load(Ordering::SeqCst) { signal_hook::consts::SIGTERM }
            else if self.sigint.load(Ordering::SeqCst) { signal_hook::consts::SIGINT }
            else if self.sigquit.load(Ordering::SeqCst) { signal_hook::consts::SIGQUIT }
            else if self.sighup.load(Ordering::SeqCst) { signal_hook::consts::SIGHUP } else { 0 }
        }
    }
    struct ChildGuard { child_pid: Pid, armed: bool }
    impl ChildGuard { fn new(child_pid: Pid) -> Self { Self { child_pid, armed: true } } fn disarm(&mut self) { self.armed = false; } }
    impl Drop for ChildGuard { fn drop(&mut self) { if self.armed { kill_and_reap(self.child_pid); } } }
    pub(super) fn supervise() -> anyhow::Result<()> {
        let (work_ms, sleep_ms) = duty_cycle_ms(cpu_limit_percent()); let sigs = SignalSet::install()?;
        let mut cmd = std::process::Command::new(std::env::current_exe().context("current_exe")?);
        cmd.args(std::env::args_os().skip(1)); cmd.env(WORKER_MARKER, "1"); cmd.env(SUPERVISOR_PID_ENV, std::process::id().to_string());
        cmd.env(WORKER_NONCE_ENV, "1"); for var in THREAD_ENV_VARS { cmd.env(var, "1"); }
        cmd.stdin(std::process::Stdio::inherit()); cmd.stdout(std::process::Stdio::inherit()); cmd.stderr(std::process::Stdio::inherit());
        let mut child = cmd.spawn().context("failed to spawn worker")?; let child_pid = Pid::from_raw(child.id() as i32);
        let mut guard = ChildGuard::new(child_pid); let _ = nix::unistd::setpgid(child_pid, child_pid); wait_for_child_stop(child_pid)?;
        let pgid_neg = Pid::from_raw(-child_pid.as_raw());
        loop {
            // Duty-cycle: SIGCONT for work window, SIGSTOP for sleep window (PR#9).
            if sigs.tstp.swap(false, Ordering::SeqCst) {
                let _ = signal::kill(pgid_neg, Signal::SIGSTOP); let _ = signal::raise(Signal::SIGSTOP); let _ = signal::kill(pgid_neg, Signal::SIGCONT);
            }
            let _ = signal::kill(pgid_neg, Signal::SIGCONT);
            if !sleep_checking(work_ms, &mut child, child_pid, &sigs)? { guard.disarm(); return Ok(()); }
            let _ = signal::kill(pgid_neg, Signal::SIGSTOP);
            if !sleep_checking(sleep_ms, &mut child, child_pid, &sigs)? { guard.disarm(); return Ok(()); }
        }
    }
    fn wait_for_child_stop(child_pid: Pid) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match wait::waitpid(child_pid, Some(WaitPidFlag::WNOHANG | WaitPidFlag::WUNTRACED)) {
                Ok(WaitStatus::Stopped(_, _)) => return Ok(()),
                Ok(WaitStatus::Exited(_, c)) => std::process::exit(c),
                Ok(WaitStatus::Signaled(_, sig, _)) => { let _ = signal::raise(sig); std::process::exit(128 + sig as i32); }
                Ok(WaitStatus::StillAlive) | Err(_) => {
                    if Instant::now() >= deadline { anyhow::bail!("worker child (pid {}) did not stop within 10 s", child_pid); }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Ok(other) => anyhow::bail!("unexpected worker status while waiting for stop: {other:?}"),
            }
        }
    }
    pub(super) fn kill_and_reap(child_pid: Pid) {
        let pgid_neg = Pid::from_raw(-child_pid.as_raw());
        let _ = signal::kill(pgid_neg, Signal::SIGCONT); let _ = signal::kill(pgid_neg, Signal::SIGTERM);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(WaitStatus::Exited(_, _) | WaitStatus::Signaled(_, _, _)) = wait::waitpid(child_pid, Some(WaitPidFlag::WNOHANG)) { break; }
            if Instant::now() >= deadline {
                let _ = signal::kill(pgid_neg, Signal::SIGKILL); let _ = signal::kill(child_pid, Signal::SIGKILL);
                if let Ok(WaitStatus::Exited(_, _) | WaitStatus::Signaled(_, _, _)) = wait::waitpid(child_pid, Some(WaitPidFlag::WNOHANG)) { break; }
                let _ = wait::waitpid(child_pid, None); break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let drain_end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < drain_end {
            if signal::kill(pgid_neg, Signal::SIGTERM).is_err() { break; }
            std::thread::sleep(Duration::from_millis(50));
            if Instant::now() >= drain_end { break; }
            if signal::kill(pgid_neg, Signal::SIGKILL).is_err() { break; }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    fn exit_shutdown(child_pid: Pid, sigs: &SignalSet) -> ! { kill_and_reap(child_pid); std::process::exit(128 + sigs.shutdown_signal()); }
    fn sleep_checking(ms: u64, child: &mut std::process::Child, child_pid: Pid, sigs: &SignalSet) -> anyhow::Result<bool> {
        let end = Instant::now() + Duration::from_millis(ms);
        loop {
            if sigs.shutdown_any() { exit_shutdown(child_pid, sigs); }
            if let Ok(Some(status)) = child.try_wait() {
                if status.success() { return Ok(false); }
                #[cfg(unix)] { use std::os::unix::process::ExitStatusExt; if let Some(sig) = status.signal() { std::process::exit(128 + sig); } }
                std::process::exit(status.code().unwrap_or(1));
            }
            if Instant::now() >= end { return Ok(true); }
            std::thread::sleep(end.saturating_duration_since(Instant::now()).min(Duration::from_millis(10)));
        }
    }
}
