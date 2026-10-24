//! Runtime posture contract: asupersync is the sole async runtime.
//!
//! INTENT: no crate in the tokio family may resolve into ANY native build
//! target's dependency graph (the three release triples), and asupersync
//! must be present in each native graph (the serve shell's runtime — the
//! active side of the cutover).
//!
//! Cargo.lock may still carry target-gated entries for asupersync's wasm32
//! lane: wasm-bindgen-futures 0.4.79 declares a tokio dependency behind
//! `cfg(all(target_os = "emscripten", wasm_bindgen_unstable_tokio))`, which
//! cargo locks but no native target can ever activate (verified 2026-10-24:
//! no single-crate pin avoids it — the wasm-bindgen/web-sys matrix is rigid
//! across 0.4.49..0.4.72 — and a multi-crate fake pin matrix was rejected
//! as brittle). Lockfile presence is not runtime use; THIS contract is the
//! enforcement that it never becomes runtime use.
//!
//! If this test fails, a new dependency dragged tokio into a real build.
//! Replace it with an asupersync-native crate, or route it through an
//! explicit compat boundary with downstream compile+runtime evidence,
//! before landing.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

/// The tokio family: tokio itself plus the satellite crates that only ever
/// appear as its runtime/codec/test companions.
const TOKIO_FAMILY: [&str; 5] = [
    "tokio",
    "tokio-macros",
    "tokio-util",
    "tokio-stream",
    "tokio-test",
];

/// Native release triples (the DSR release lane set).
const NATIVE_TARGETS: [&str; 3] = [
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

/// Platform-filtered package names from `cargo metadata`. Resolution only —
/// no compilation. Cross-target resolution may download registry entries
/// for non-host crates (never compiled); a stale lockfile fails loudly,
/// which is itself a posture violation worth catching.
fn resolved_package_names(target: &str) -> Vec<String> {
    let workspace_root: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", ".."].iter().collect();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1"])
        .arg("--filter-platform")
        .arg(target)
        .current_dir(&workspace_root)
        .output()
        .expect("spawn cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata --filter-platform {target} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("cargo metadata emits JSON");
    value["packages"]
        .as_array()
        .expect("metadata has packages")
        .iter()
        .map(|package| {
            package["name"]
                .as_str()
                .expect("package has a name")
                .to_owned()
        })
        .collect()
}

#[test]
fn tokio_family_absent_and_asupersync_present_on_every_native_target() {
    for target in NATIVE_TARGETS {
        let names = resolved_package_names(target);
        let offenders: Vec<&str> = TOKIO_FAMILY
            .iter()
            .copied()
            .filter(|family| names.iter().any(|name| name == family))
            .collect();
        assert!(
            offenders.is_empty(),
            "target {target} resolves tokio-family crates {offenders:?}: \
             asupersync is the sole async runtime; replace the offending \
             dependency or route it through an explicit compat boundary"
        );
        assert!(
            names.iter().any(|name| name == "asupersync"),
            "target {target} does not resolve asupersync: the serve shell's \
             runtime must be present in every native graph"
        );
    }
}
