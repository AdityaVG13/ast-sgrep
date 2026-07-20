//! Hollow supervisor: no duty-cycle process control; CLI runs in-process.

pub const DEFAULT_CPU_LIMIT: u8 = 80; pub const MIN_CPU_LIMIT: u8 = 1; pub const MAX_CPU_LIMIT: u8 = 80; pub const CYCLE_MS: u64 = 10;

#[cfg(unix)] pub fn is_worker() -> bool { false }

pub fn cpu_limit_percent() -> u8 { parse_cpu_limit(&std::env::var("ASGREP_CPU_LIMIT_PERCENT").unwrap_or_default()) }

pub fn parse_cpu_limit(raw: &str) -> u8 {
    raw.trim()
        .parse::<u8>() .ok() .filter(|&p| (MIN_CPU_LIMIT..=MAX_CPU_LIMIT).contains(&p)) .unwrap_or(DEFAULT_CPU_LIMIT)
}

pub fn duty_cycle_ms(limit_pct: u8) -> (u64, u64) {
    let work_ms = if limit_pct == 0 {
        0
    } else {
        ((CYCLE_MS * u64::from(limit_pct)) / 100).max(1)
    }; (work_ms, CYCLE_MS.saturating_sub(work_ms))
}

#[cfg(unix)] pub fn clear_internal_envs() {}

#[cfg(unix)] pub fn supervise() -> anyhow::Result<()> {
    // Passthrough: caller should run the process directly.
    Ok(())
}

#[cfg(unix)] pub fn worker_authenticate() -> bool { false }

#[cfg(unix)] pub fn worker_start() {}
