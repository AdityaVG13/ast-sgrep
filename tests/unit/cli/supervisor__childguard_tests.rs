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
