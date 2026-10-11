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
/// H-PERF-001 option (b) — supervision is opt-in. Presence of
/// `ASGREP_CPU_LIMIT_PERCENT` in the environment re-enables the supervisor;
/// unset means a single in-process run. A malformed value still counts as
/// opted-in and keeps the fail-safe default bound (`DEFAULT_CPU_LIMIT`) via
/// `parse_cpu_limit` — it never silently disables the CPU contract.
#[cfg(unix)]
pub fn supervision_requested() -> bool {
    std::env::var_os(CPU_LIMIT_ENV).is_some()
}
#[cfg(not(unix))]
pub fn supervision_requested() -> bool {
    false
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
    #[cfg(target_os = "macos")]
    mac_guardian::clear_role_envs();
}

#[cfg(target_os = "macos")]
pub fn guardian_requested() -> bool {
    mac_guardian::requested()
}

#[cfg(target_os = "macos")]
pub fn run_guardian() -> ! {
    // A background guardian must never write to a TOSTOP terminal. Failure
    // closes the private channel; its owner reports the operational failure.
    std::process::exit(mac_guardian::run().unwrap_or(2))
}
/// In-process run for the unsupervised (default) mode. Keeps the supervised
/// worker's thread-cap environment so embed/native threading behavior does
/// not drift between the two modes, and clears the worker handshake
/// variables defensively. The supervisor's process-lifecycle guards (orphan
/// reaping, parent-watch) remain supervision-scoped by design (EV card
/// option b risk acceptance).
#[cfg(unix)]
pub fn run_unsupervised() -> anyhow::Result<()> {
    clear_internal_envs();
    for var in THREAD_ENV_VARS {
        std::env::set_var(var, "1");
    }
    crate::run_process()
}
#[cfg(unix)]
pub fn supervise() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    return mac_guardian::supervise();
    #[cfg(not(target_os = "macos"))]
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
pub fn worker_start() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    use anyhow::Context;
    use nix::sys::signal::{self, Signal};
    use nix::unistd::{getppid, Pid};
    // Retain the authenticated parent's identity, not a fresh PPID sampled
    // after it may have died between authentication and guard installation.
    let supervisor = Pid::from_raw(std::env::var(SUPERVISOR_PID_ENV)?.parse()?);
    clear_internal_envs();
    // Kernel delivery survives normal exec and kills even a stopped worker.
    // A polling thread cannot cover either case. Privileged exec is not used.
    #[cfg(target_os = "linux")]
    nix::sys::prctl::set_pdeathsig(Signal::SIGKILL)
        .context("install supervisor parent-death signal")?;
    // Close the race where the parent died before the kernel guard was armed.
    if getppid() != supervisor {
        std::process::exit(128 + signal_hook::consts::SIGTERM);
    }
    // macOS uses a separately runnable guardian that owns and reaps this
    // worker. Other Unix keeps its runnable-only fallback; no portability
    // certificate is implied for that thread.
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = std::thread::Builder::new()
        .name("asgrep-parent-watch".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(20));
            if getppid() != supervisor {
                let _ = signal::kill(nix::unistd::getpid(), Signal::SIGTERM);
                std::process::exit(128 + signal_hook::consts::SIGTERM);
            }
        });
    // Stay in the supervisor process group. Duty-cycle STOP/CONT target this pid.
    let _ = signal::raise(Signal::SIGSTOP);
    Ok(())
}
// macOS has no kernel parent-death signal. An independently runnable owner
// detects exclusive control-channel EOF and kills/reaps its unreaped child,
// including after exec or SIGSTOP. Only opted-in macOS runs pay this extra
// process/IPC cost; nominal windows are retained, not a zero-overhead claim.
#[cfg(target_os = "macos")]
mod mac_guardian {
    use super::unix_impl::SignalSet;
    use super::*;
    use anyhow::{ensure, Context};
    use nix::errno::Errno;
    use nix::sys::signal::{self, Signal};
    use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
    use nix::unistd::{getpgrp, getppid, Pid};
    use std::io::{ErrorKind, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    const ADDRESS: &str = "ASGREP_GUARDIAN_ADDRESS";
    const PARENT: &str = "ASGREP_GUARDIAN_PARENT";
    const TOKEN: &str = "ASGREP_GUARDIAN_TOKEN";
    const GROUP: &str = "ASGREP_GUARDIAN_GROUP";
    const CONTINUE: u8 = b'C';
    const STOP: u8 = b'S';
    const TERMINATE: u8 = b'T';
    const ACK: u8 = b'A';
    const READY: u8 = b'R';

    pub(super) fn requested() -> bool {
        std::env::var_os(ADDRESS).is_some()
    }

    pub(super) fn clear_role_envs() {
        for name in [ADDRESS, PARENT, TOKEN, GROUP] {
            std::env::remove_var(name);
        }
    }

    fn transport(stream: &TcpStream) -> std::io::Result<()> {
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_millis(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(1)))
    }

    fn transient(error: &std::io::Error) -> bool {
        matches!(
            error.kind(),
            ErrorKind::Interrupted | ErrorKind::WouldBlock | ErrorKind::TimedOut
        )
    }

    // A guardian is never forcibly killed as ordinary outer cleanup: closing
    // our CLOEXEC stream wakes its independently runnable child owner first.
    struct Controller {
        child: Child,
        stream: Option<TcpStream>,
    }

    impl Drop for Controller {
        fn drop(&mut self) {
            self.stream.take();
            let _ = self.child.wait();
        }
    }

    impl Controller {
        fn reply(&mut self, expected: u8) -> anyhow::Result<Option<ExitStatus>> {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut byte = [0];
            loop {
                match self
                    .stream
                    .as_mut()
                    .context("guardian channel unavailable")?
                    .read(&mut byte)
                {
                    Ok(1) => {
                        ensure!(byte[0] == expected, "invalid guardian acknowledgement");
                        return Ok(None);
                    }
                    Ok(_) => return Ok(Some(self.child.wait()?)),
                    Err(error) if transient(&error) => {
                        if let Some(status) = self.child.try_wait()? {
                            return Ok(Some(status));
                        }
                        ensure!(
                            Instant::now() < deadline,
                            "guardian control request exceeded 10 seconds"
                        );
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }

        fn command(&mut self, command: u8) -> anyhow::Result<Option<ExitStatus>> {
            if let Err(error) = self
                .stream
                .as_mut()
                .context("guardian channel unavailable")?
                .write_all(&[command])
            {
                if matches!(
                    error.kind(),
                    ErrorKind::BrokenPipe | ErrorKind::ConnectionReset
                ) {
                    return Ok(Some(self.child.wait()?));
                }
                return Err(error.into());
            }
            self.reply(ACK)
        }

        fn sleep(
            &mut self,
            millis: u64,
            signals: &SignalSet,
        ) -> anyhow::Result<Option<ExitStatus>> {
            let deadline = Instant::now() + Duration::from_millis(millis);
            loop {
                if signals.shutdown_signal() != 0 {
                    self.stream
                        .as_mut()
                        .context("guardian channel unavailable")?
                        .write_all(&[TERMINATE])?;
                    return Ok(Some(self.child.wait()?));
                }
                if let Some(status) = self.child.try_wait()? {
                    return Ok(Some(status));
                }
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                std::thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(2)),
                );
            }
        }
    }

    pub(super) fn supervise() -> anyhow::Result<()> {
        let signals = SignalSet::install()?;
        // A loopback ephemeral listener avoids pathname-length and SIGKILL
        // filesystem cleanup obligations. The bounded nonce handshake is
        // topology authentication, not protection against the same user.
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let token = generate_worker_nonce();
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(std::env::args_os().skip(1))
            .env(ADDRESS, listener.local_addr()?.to_string())
            .env(PARENT, std::process::id().to_string())
            .env(TOKEN, &token)
            .env(GROUP, getpgrp().as_raw().to_string())
            .env_remove(WORKER_MARKER)
            .env_remove(SUPERVISOR_PID_ENV)
            .env_remove(WORKER_NONCE_ENV)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .process_group(0);
        let mut controller = Controller {
            child: command.spawn().context("spawn macOS worker guardian")?,
            stream: None,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match listener.accept() {
                Ok((stream, peer)) => {
                    ensure!(peer.ip().is_loopback(), "non-loopback guardian connection");
                    transport(&stream)?;
                    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
                    controller.stream = Some(stream);
                    let mut received = [0; WORKER_NONCE_MIN_LEN];
                    controller
                        .stream
                        .as_mut()
                        .unwrap()
                        .read_exact(&mut received)?;
                    ensure!(
                        received.as_slice() == token.as_bytes(),
                        "guardian nonce mismatch"
                    );
                    controller.stream.as_mut().unwrap().write_all(&[ACK])?;
                    controller
                        .stream
                        .as_mut()
                        .unwrap()
                        .set_read_timeout(Some(Duration::from_millis(2)))?;
                    break;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    ensure!(
                        controller.child.try_wait()?.is_none(),
                        "guardian exited before connecting"
                    );
                    ensure!(
                        signals.shutdown_signal() == 0,
                        "supervision interrupted during guardian startup"
                    );
                    ensure!(
                        Instant::now() < deadline,
                        "guardian startup exceeded 10 seconds"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => return Err(error.into()),
            }
        }
        drop(listener);
        ensure!(
            controller.reply(READY)?.is_none(),
            "guardian exited before worker bootstrap"
        );
        let (work_ms, sleep_ms) = duty_cycle_ms(cpu_limit_percent());
        let terminal = loop {
            if signals.tstp.swap(false, Ordering::SeqCst) {
                if let Some(status) = controller.command(STOP)? {
                    break status;
                }
                signal::raise(Signal::SIGSTOP)?;
            }
            if let Some(status) = controller.child.try_wait()? {
                break status;
            }
            if let Some(status) = controller.command(CONTINUE)? {
                break status;
            }
            if let Some(status) = controller.sleep(work_ms, &signals)? {
                break status;
            }
            if let Some(status) = controller.command(STOP)? {
                break status;
            }
            if let Some(status) = controller.sleep(sleep_ms, &signals)? {
                break status;
            }
        };
        let shutdown = signals.shutdown_signal();
        drop(controller);
        let code = if shutdown != 0 {
            128 + shutdown
        } else {
            terminal
                .code()
                .unwrap_or_else(|| 128 + terminal.signal().unwrap_or(1))
        };
        if code != 0 {
            std::process::exit(code);
        }
        Ok(())
    }

    // Only this single waiter can revoke signal authority. A terminal wait or
    // ECHILD disarms before any fallible transport operation or unwinding.
    struct Worker {
        pid: Pid,
        owned: bool,
    }

    impl Worker {
        fn observe(&mut self) -> anyhow::Result<WaitStatus> {
            match waitpid(
                self.pid,
                Some(WaitPidFlag::WNOHANG | WaitPidFlag::WUNTRACED | WaitPidFlag::WCONTINUED),
            ) {
                Ok(status @ (WaitStatus::Exited(..) | WaitStatus::Signaled(..))) => {
                    self.owned = false;
                    Ok(status)
                }
                Ok(status) => Ok(status),
                Err(Errno::EINTR) => Ok(WaitStatus::StillAlive),
                Err(Errno::ECHILD) => {
                    self.owned = false;
                    anyhow::bail!("guardian lost worker wait ownership")
                }
                Err(error) => Err(error.into()),
            }
        }
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            if !self.owned {
                return;
            }
            let _ = signal::kill(self.pid, Signal::SIGKILL);
            loop {
                match waitpid(self.pid, None) {
                    Err(Errno::EINTR) => continue,
                    _ => {
                        self.owned = false;
                        break;
                    }
                }
            }
        }
    }

    pub(super) fn run() -> anyhow::Result<i32> {
        let parent: i32 = std::env::var(PARENT)?.parse()?;
        ensure!(
            parent > 0 && getppid() == Pid::from_raw(parent),
            "guardian parent identity changed"
        );
        let address: SocketAddr = std::env::var(ADDRESS)?.parse()?;
        ensure!(
            address.ip() == std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            "guardian address must be loopback IPv4"
        );
        let token = std::env::var(TOKEN)?;
        ensure!(
            token.len() == WORKER_NONCE_MIN_LEN
                && token.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid guardian token"
        );
        let group: i32 = std::env::var(GROUP)?.parse()?;
        ensure!(group > 0, "invalid worker job group");
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1))?;
        transport(&stream)?;
        stream.write_all(token.as_bytes())?;
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        let mut ack = [0];
        stream.read_exact(&mut ack)?;
        ensure!(ack[0] == ACK, "guardian authentication rejected");
        stream.set_read_timeout(Some(Duration::from_millis(2)))?;
        clear_role_envs();
        let signals = SignalSet::install()?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(std::env::args_os().skip(1))
            .env(WORKER_MARKER, "1")
            .env(SUPERVISOR_PID_ENV, std::process::id().to_string())
            .env(WORKER_NONCE_ENV, generate_worker_nonce())
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .process_group(group);
        for name in THREAD_ENV_VARS {
            command.env(name, "1");
        }
        let child = command.spawn().context("guardian spawn worker")?;
        let mut worker = Worker {
            pid: Pid::from_raw(i32::try_from(child.id())?),
            owned: true,
        };
        drop(child); // std Child::drop does not wait; Worker is the sole reaper.
        let bootstrap = Instant::now() + Duration::from_secs(10);
        let mut ready = false;
        let mut stopped = false;
        let mut stop_pending = false;
        let mut terminate_at = None;
        loop {
            match worker.observe()? {
                WaitStatus::Exited(_, code) => return Ok(code),
                WaitStatus::Signaled(_, signal, _) => return Ok(128 + signal as i32),
                WaitStatus::Stopped(..) => stopped = true,
                WaitStatus::Continued(..) => stopped = false,
                _ => {}
            }
            if stopped && !ready {
                stream.write_all(&[READY])?;
                ready = true;
            }
            if stopped && stop_pending {
                stream.write_all(&[ACK])?;
                stop_pending = false;
            }
            ensure!(
                ready || Instant::now() < bootstrap,
                "worker bootstrap exceeded 10 seconds"
            );
            if terminate_at.is_none() && signals.shutdown_signal() != 0 {
                signal::kill(worker.pid, Signal::SIGCONT)?;
                signal::kill(worker.pid, Signal::SIGTERM)?;
                terminate_at = Some(Instant::now() + Duration::from_millis(100));
            }
            if terminate_at.is_some_and(|deadline| Instant::now() >= deadline) {
                return Ok(2);
            }
            let mut byte = [0];
            match stream.read(&mut byte) {
                Ok(0) => return Ok(2), // Drop kills even a stopped child, then reaps.
                Ok(1) => match byte[0] {
                    CONTINUE if ready && terminate_at.is_none() => {
                        signal::kill(worker.pid, Signal::SIGCONT)?;
                        stopped = false;
                        stream.write_all(&[ACK])?;
                    }
                    STOP if ready && terminate_at.is_none() => {
                        signal::kill(worker.pid, Signal::SIGSTOP)?;
                        stop_pending = true;
                    }
                    TERMINATE => {
                        signal::kill(worker.pid, Signal::SIGCONT)?;
                        signal::kill(worker.pid, Signal::SIGTERM)?;
                        terminate_at = Some(Instant::now() + Duration::from_millis(100));
                    }
                    _ => anyhow::bail!("invalid guardian command"),
                },
                Err(error) if transient(&error) => {}
                Err(error) => return Err(error.into()),
                _ => unreachable!("one-byte control read"),
            }
        }
    }
}

#[cfg(unix)]
mod unix_impl {
    #[cfg(not(target_os = "macos"))]
    use super::*;
    use anyhow::Context;
    #[cfg(not(target_os = "macos"))]
    use nix::sys::signal::{self, Signal};
    #[cfg(not(target_os = "macos"))]
    use nix::sys::wait::{self, WaitPidFlag, WaitStatus};
    #[cfg(not(target_os = "macos"))]
    use nix::unistd::Pid;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    #[cfg(not(target_os = "macos"))]
    use std::time::{Duration, Instant};
    pub(super) struct SignalSet {
        _ids: [signal_hook::SigId; 5],
        sigint: Arc<AtomicBool>,
        sigterm: Arc<AtomicBool>,
        sighup: Arc<AtomicBool>,
        sigquit: Arc<AtomicBool>,
        pub(super) tstp: Arc<AtomicBool>,
    }
    impl SignalSet {
        pub(super) fn install() -> anyhow::Result<Self> {
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
        pub(super) fn shutdown_signal(&self) -> i32 {
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
        #[cfg(not(target_os = "macos"))]
        fn shutdown_any(&self) -> bool {
            self.shutdown_signal() != 0
        }
    }
    #[cfg(not(target_os = "macos"))]
    struct ChildGuard {
        child_pid: Pid,
        armed: bool,
    }
    #[cfg(not(target_os = "macos"))]
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
    #[cfg(not(target_os = "macos"))]
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
    #[cfg(not(target_os = "macos"))]
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
        // Stay in the supervisor process group so a parent SIGTERM/SIGHUP
        // reaches the worker with default terminate disposition. Duty-cycle
        // STOP/CONT target the worker pid, not a private process group.
        let mut child = cmd.spawn().context("failed to spawn worker")?;
        // Pid::from_raw bridges std Child::id() into nix. Child ids are OS PIDs;
        // casting to i32 matches nix's Pid representation on supported unix targets (l115/732x).
        let child_pid = Pid::from_raw(child.id() as i32);
        let mut guard = ChildGuard::new(child_pid);
        wait_for_child_stop(child_pid, &sigs)?;
        loop {
            // Duty-cycle: SIGCONT for work window, SIGSTOP for sleep window (PR#9).
            if sigs.tstp.swap(false, Ordering::SeqCst) {
                let _ = signal::kill(child_pid, Signal::SIGSTOP);
                let _ = signal::raise(Signal::SIGSTOP);
                let _ = signal::kill(child_pid, Signal::SIGCONT);
            }
            let _ = signal::kill(child_pid, Signal::SIGCONT);
            if !sleep_checking(work_ms, &mut child, child_pid, &sigs)? {
                guard.disarm();
                return Ok(());
            }
            let _ = signal::kill(child_pid, Signal::SIGSTOP);
            if !sleep_checking(sleep_ms, &mut child, child_pid, &sigs)? {
                guard.disarm();
                return Ok(());
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    fn wait_for_child_stop(child_pid: Pid, sigs: &SignalSet) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if sigs.shutdown_any() {
                exit_shutdown(child_pid, sigs);
            }
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
    #[cfg(not(target_os = "macos"))]
    pub(super) fn kill_and_reap(child_pid: Pid) {
        let _ = signal::kill(child_pid, Signal::SIGCONT);
        let _ = signal::kill(child_pid, Signal::SIGTERM);
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            if let Ok(WaitStatus::Exited(_, _) | WaitStatus::Signaled(_, _, _)) =
                wait::waitpid(child_pid, Some(WaitPidFlag::WNOHANG))
            {
                break;
            }
            if Instant::now() >= deadline {
                let _ = signal::kill(child_pid, Signal::SIGKILL);
                if let Ok(WaitStatus::Exited(_, _) | WaitStatus::Signaled(_, _, _)) =
                    wait::waitpid(child_pid, Some(WaitPidFlag::WNOHANG))
                {
                    break;
                }
                let _ = wait::waitpid(child_pid, None);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[cfg(not(target_os = "macos"))]
    fn exit_shutdown(child_pid: Pid, sigs: &SignalSet) -> ! {
        kill_and_reap(child_pid);
        std::process::exit(128 + sigs.shutdown_signal());
    }
    #[cfg(not(target_os = "macos"))]
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
