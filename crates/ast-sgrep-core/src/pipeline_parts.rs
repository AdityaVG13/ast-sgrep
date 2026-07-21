//! In-process microbench of core search/index pipeline parts (sub1ms gate).
use crate::index::{IndexOptions, Indexer};
use crate::intent;
use crate::query::ParsedQuery;
use crate::search::{format_hit_line, SearchOptions, Searcher};
use crate::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
pub const CORE_PARTS: &[&str] = &[
    "query_parse_intent",
    "literal_retrieval",
    "lexical_fts",
    "symbol_graph",
    "hybrid_fusion_rank",
    "semantic_embed",
    "result_format",
    "index_update_one_file",
];
pub const BUDGET_MS: f64 = 1.0;
#[derive(Debug, Clone, Serialize)]
pub struct PartTiming {
    pub name: String,
    pub iterations: u32,
    pub warmup: u32,
    pub median_ms: f64,
    pub mean_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub p95_ms: f64,
    pub work_units: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub fixture: String,
    pub fixture_root: String,
    pub warmup: u32,
    pub iterations: u32,
    pub budget_ms: f64,
    pub all_under_budget: bool,
    pub notes: String,
    pub parts: Vec<PartTiming>,
}
#[derive(Debug, Clone)]
pub struct Config {
    pub warmup: u32,
    pub iterations: u32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            warmup: 25,
            iterations: 120,
        }
    }
}
pub fn measure(root: &Path, index_dir: &Path, cfg: &Config) -> Result<Report> {
    let searcher = warm_searcher(root, index_dir)?;
    let parts = vec![
        measure_query_parse_intent(&searcher, cfg)?,
        measure_literal(&searcher, cfg),
        measure_lexical(&searcher, cfg),
        measure_symbol_graph(&searcher, cfg),
        measure_hybrid_fusion(&searcher, cfg)?,
        measure_semantic(&searcher, cfg),
        measure_format(&searcher, cfg)?,
        measure_index_update(root, index_dir, cfg)?,
    ];
    Ok(Report {
        fixture: "sample".into(),
        fixture_root: root.display().to_string(),
        warmup: cfg.warmup,
        iterations: cfg.iterations,
        budget_ms: BUDGET_MS,
        all_under_budget: parts.iter().all(|p| p.median_ms < BUDGET_MS),
        notes: "In-process, warm sample fixture, offline local embed (no network).".into(),
        parts,
    })
}
pub fn assert_under_budget(report: &Report) -> std::result::Result<(), String> {
    for name in CORE_PARTS {
        if !report.parts.iter().any(|p| p.name == *name) {
            return Err(format!("missing part: {name}"));
        }
    }
    let mut errors = Vec::new();
    for p in &report.parts {
        if p.work_units == 0 {
            errors.push(format!("{}: zero work_units (no-op?)", p.name));
        }
        if p.median_ms >= BUDGET_MS {
            errors.push(format!(
                "{}: {:.4}ms >= {}ms",
                p.name, p.median_ms, BUDGET_MS
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}
pub fn write_json(report: &Report, path: &Path) -> std::io::Result<()> {
    serde_json::to_writer_pretty(std::fs::File::create(path)?, report)
        .map_err(std::io::Error::other)
}
pub fn sample_root() -> PathBuf {
    let relative = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/sample");
    relative.canonicalize().unwrap_or(relative)
}
fn warm_searcher(root: &Path, index_dir: &Path) -> Result<Searcher> {
    let index_path = index_dir.join("index.db");
    Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: true,
        ..IndexOptions::default()
    })?
    .index_all()?;
    let searcher = Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path),
        limit: 16,
        use_embed: true,
        ..SearchOptions::default()
    })?;
    let _ = searcher.search("process_request")?;
    let _ = searcher.search_semantic("auth refresh")?;
    let _ = searcher.search_literal("process_request")?;
    Ok(searcher)
}
fn measure_query_parse_intent(searcher: &Searcher, cfg: &Config) -> Result<PartTiming> {
    let queries = [
        "process_request",
        "defs:auth_refresh",
        "callers:process_request",
        "how does auth refresh work",
        "literal:token",
    ];
    let mut hits = searcher.search("process_request")?.hits;
    let (samples, work) = time_loop(cfg, || {
        let mut units = 0u64;
        for q in &queries {
            let parsed = ParsedQuery::parse(q);
            let intent = intent::classify(&parsed);
            intent::route_hits(&parsed, &mut hits);
            units += parsed.terms.len() as u64 + intent.as_str().len() as u64;
        }
        units
    });
    Ok(summarize("query_parse_intent", cfg, samples, work))
}
fn measure_literal(searcher: &Searcher, cfg: &Config) -> PartTiming {
    let (samples, work) = time_loop(cfg, || {
        searcher
            .search_literal("process_request")
            .expect("literal")
            .hits
            .len() as u64
    });
    summarize("literal_retrieval", cfg, samples, work)
}
fn measure_lexical(searcher: &Searcher, cfg: &Config) -> PartTiming {
    let (samples, work) = time_loop(cfg, || {
        searcher
            .search_lexical("auth refresh")
            .expect("lexical")
            .hits
            .len() as u64
    });
    summarize("lexical_fts", cfg, samples, work)
}
fn measure_symbol_graph(searcher: &Searcher, cfg: &Config) -> PartTiming {
    let (samples, work) = time_loop(cfg, || {
        let defs = searcher.search("defs:auth_refresh").expect("defs");
        let callers = searcher.search("callers:process_request").expect("callers");
        let sym = searcher
            .search_symbol_pass("process_request")
            .expect("symbol");
        (defs.hits.len() + callers.hits.len() + sym.hits.len()) as u64
    });
    summarize("symbol_graph", cfg, samples, work)
}
fn measure_hybrid_fusion(searcher: &Searcher, cfg: &Config) -> Result<PartTiming> {
    let query = "how does auth refresh work";
    let parsed = ParsedQuery::parse(query);
    let mut candidates = searcher.search_lexical(query)?.hits;
    candidates.extend(searcher.search_symbol_pass(query)?.hits);
    candidates.extend(searcher.search_semantic(query)?.hits);
    assert!(
        candidates.len() >= 2,
        "hybrid fusion needs multi-pass candidates, got {}",
        candidates.len()
    );
    let opts = searcher.options().clone();
    let (samples, work) = time_loop(cfg, || {
        let mut hits = candidates.clone();
        intent::route_hits(&parsed, &mut hits);
        crate::search::finish_response(&parsed, &opts, hits, true)
            .hits
            .len() as u64
    });
    Ok(summarize("hybrid_fusion_rank", cfg, samples, work))
}
fn measure_semantic(searcher: &Searcher, cfg: &Config) -> PartTiming {
    let (samples, work) = time_loop(cfg, || {
        searcher
            .search_semantic("how does auth refresh work")
            .expect("semantic")
            .hits
            .len() as u64
    });
    summarize("semantic_embed", cfg, samples, work)
}
fn measure_format(searcher: &Searcher, cfg: &Config) -> Result<PartTiming> {
    let hits = searcher.search("process_request")?.hits;
    assert!(!hits.is_empty(), "format part needs real hits");
    let (samples, work) = time_loop(cfg, || {
        let mut bytes = 0u64;
        for h in &hits {
            bytes += format_hit_line(h).len() as u64;
        }
        bytes + serde_json::to_string(&hits).expect("json").len() as u64
    });
    Ok(summarize("result_format", cfg, samples, work))
}
fn measure_index_update(root: &Path, index_dir: &Path, cfg: &Config) -> Result<PartTiming> {
    const REL: &str = "src/lib.ts";
    let original = std::fs::read_to_string(root.join(REL)).map_err(crate::StoreError::Io)?;
    let work_root = index_dir.join("update_tree");
    copy_tree(root, &work_root)?;
    let work_file = work_root.join(REL);
    let mut indexer = Indexer::new(IndexOptions {
        root: work_root,
        index_path: Some(index_dir.join("update.db")),
        embed_semantic: true,
        ..IndexOptions::default()
    })?;
    indexer.index_all()?;
    let content_a = original;
    let content_b = format!("{content_a}\n// sub1ms-bench-marker\n");
    for content in [&content_a, &content_b, &content_a, &content_b] {
        std::fs::write(&work_file, content).map_err(crate::StoreError::Io)?;
        indexer.update_paths(&[work_file.clone()])?;
    }
    for _ in 0..cfg.warmup {
        std::fs::write(&work_file, &content_b).map_err(crate::StoreError::Io)?;
        indexer.update_paths(&[work_file.clone()])?;
        std::fs::write(&work_file, &content_a).map_err(crate::StoreError::Io)?;
        indexer.update_paths(&[work_file.clone()])?;
    }
    let mut flip = false;
    let mut samples = Vec::with_capacity(cfg.iterations as usize);
    let mut last_work = 0u64;
    for _ in 0..cfg.iterations {
        flip = !flip;
        let content = if flip { &content_b } else { &content_a };
        std::fs::write(&work_file, content).map_err(crate::StoreError::Io)?;
        let t0 = Instant::now();
        let stats = indexer.update_paths(&[work_file.clone()])?;
        samples.push(ms(t0.elapsed()));
        last_work =
            (stats.files_indexed + stats.files_skipped + stats.files_removed + stats.files_failed)
                as u64;
        assert!(last_work > 0, "update_paths did no work for {REL}");
    }
    Ok(summarize("index_update_one_file", cfg, samples, last_work))
}
fn time_loop(cfg: &Config, mut body: impl FnMut() -> u64) -> (Vec<f64>, u64) {
    for _ in 0..cfg.warmup {
        let _ = body();
    }
    let mut samples = Vec::with_capacity(cfg.iterations as usize);
    let mut last_work = 0u64;
    for _ in 0..cfg.iterations {
        let t0 = Instant::now();
        last_work = body();
        samples.push(ms(t0.elapsed()));
    }
    (samples, last_work)
}
fn summarize(name: &str, cfg: &Config, mut samples: Vec<f64>, work: u64) -> PartTiming {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = samples.len();
    let median = match n {
        0 => 0.0,
        n if n % 2 == 1 => samples[n / 2],
        n => (samples[n / 2 - 1] + samples[n / 2]) / 2.0,
    };
    let mean = if n == 0 {
        0.0
    } else {
        samples.iter().sum::<f64>() / n as f64
    };
    let p95_i = ((n as f64) * 0.95).ceil() as usize;
    let p95 = samples
        .get(p95_i.saturating_sub(1).min(n.saturating_sub(1)))
        .copied()
        .unwrap_or(0.0);
    PartTiming {
        name: name.into(),
        iterations: cfg.iterations,
        warmup: cfg.warmup,
        median_ms: median,
        mean_ms: mean,
        min_ms: samples.first().copied().unwrap_or(0.0),
        max_ms: samples.last().copied().unwrap_or(0.0),
        p95_ms: p95,
        work_units: work,
    }
}
fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).map_err(crate::StoreError::Io)?;
    for entry in std::fs::read_dir(src).map_err(crate::StoreError::Io)? {
        let entry = entry.map_err(crate::StoreError::Io)?;
        let to = dst.join(entry.file_name());
        let ty = entry.file_type().map_err(crate::StoreError::Io)?;
        if ty.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), to).map_err(crate::StoreError::Io)?;
        }
    }
    Ok(())
}
