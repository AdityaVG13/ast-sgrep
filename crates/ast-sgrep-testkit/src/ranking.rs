use std::path::Path;

use ast_sgrep_core::{SearchHit, SearchResponse};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct RankingFixture {
    pub fixture: String,
    pub cases: Vec<RankingCase>,
}

#[derive(Debug, Deserialize)]
pub struct RankingCase {
    pub name: String,
    pub query: String,
    pub top_k: usize,
    pub must_include: Vec<RankingExpectation>,
}

#[derive(Debug, Deserialize)]
pub struct RankingExpectation {
    pub kind: Option<String>,
    pub symbol: Option<String>,
    pub callee: Option<String>,
    pub file: Option<String>,
    #[serde(default = "default_max_rank")]
    pub max_rank: usize,
}

fn default_max_rank() -> usize {
    16
}

pub fn ranking_fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/ranking/cases.json")
}

pub fn load_ranking_fixture() -> RankingFixture {
    let path = ranking_fixture_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

pub fn hit_matches(expect: &RankingExpectation, hit: &SearchHit) -> bool {
    if let Some(kind) = &expect.kind {
        if hit.kind.as_str() != kind {
            return false;
        }
    }
    if let Some(symbol) = &expect.symbol {
        if hit.symbol.as_deref() != Some(symbol.as_str()) {
            return false;
        }
    }
    if let Some(callee) = &expect.callee {
        if hit.callee.as_deref() != Some(callee.as_str()) {
            return false;
        }
    }
    if let Some(file) = &expect.file {
        if !hit.file.ends_with(file) && hit.file != *file {
            return false;
        }
    }
    true
}

pub fn assert_ranking_case(case: &RankingCase, response: &SearchResponse) {
    let top: Vec<&SearchHit> = response.hits.iter().take(case.top_k).collect();
    for expect in &case.must_include {
        let found = top.iter().enumerate().find_map(|(idx, hit)| {
            if hit_matches(expect, hit) {
                Some(idx + 1)
            } else {
                None
            }
        });
        match found {
            Some(rank) if rank <= expect.max_rank => {}
            Some(rank) => panic!(
                "case {}: {:?} matched at rank {rank} but max_rank is {}",
                case.name, expect, expect.max_rank
            ),
            None => panic!(
                "case {}: expected {:?} in top {} hits for query {:?}, got: {}",
                case.name,
                expect,
                case.top_k,
                case.query,
                summarize_hits(&top)
            ),
        }
    }
}

fn summarize_hits(hits: &[&SearchHit]) -> String {
    hits.iter()
        .enumerate()
        .map(|(i, h)| {
            format!(
                "#{} {} sym={:?} callee={:?} file={}",
                i + 1,
                h.kind.as_str(),
                h.symbol,
                h.callee,
                h.file
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}