//! Sub-1ms gate for the eight core pipeline parts.
//!
//! Times real shipped library paths on the warm polyglot sample fixture.
//!
//! ```sh
//! cargo test -p ast-sgrep-core --test sub1ms --release
//! ```
//!
//! Optional: `ASGREP_PARTS_OUT=/path/report.json` writes the full timing report.
//!
//! The median &lt; 1.0 ms assert runs only in release builds (debug is too slow).

use ast_sgrep_core::pipeline_parts::{
    assert_under_budget, measure, sample_root, write_json, Config, BUDGET_MS, CORE_PARTS,
};
use tempfile::TempDir;

#[test]
fn core_pipeline_parts_median_under_1ms() {
    let root = sample_root();
    assert!(
        root.join("src/main.rs").is_file(),
        "sample fixture missing at {}",
        root.display()
    );

    let temp = TempDir::new().expect("tempdir");
    let report = measure(&root, temp.path(), &Config::default()).expect("measure");

    if let Ok(out) = std::env::var("ASGREP_PARTS_OUT") {
        write_json(&report, std::path::Path::new(&out)).expect("write report");
        eprintln!("wrote report to {out}");
    }

    eprintln!(
        "sub1ms budget={}ms fixture={} warm={} iters={}",
        BUDGET_MS, report.fixture, report.warmup, report.iterations
    );
    for p in &report.parts {
        eprintln!(
            "  {:24} median={:.4}ms mean={:.4}ms p95={:.4}ms work={}",
            p.name, p.median_ms, p.mean_ms, p.p95_ms, p.work_units
        );
    }

    assert_eq!(report.parts.len(), CORE_PARTS.len());
    for name in CORE_PARTS {
        assert!(
            report.parts.iter().any(|p| p.name == *name),
            "missing part {name}"
        );
        let p = report.parts.iter().find(|p| p.name == *name).unwrap();
        assert!(p.work_units > 0, "{name}: timed path was a no-op");
    }

    if cfg!(debug_assertions) {
        eprintln!("debug build: skip budget assert; re-run with --release to gate");
        return;
    }

    if let Err(e) = assert_under_budget(&report) {
        panic!("sub-1ms gate failed: {e}\n{report:#?}");
    }
    assert!(report.all_under_budget);
}
