mod parse;
mod runner;

use std::path::Path;

use crate::search::SearchHit;
use crate::Result;

use parse::parse_ast_grep_json;
use runner::{find_ast_grep_binary, run_ast_grep};

/// Run ast-grep/sg for structural pattern search when available.
pub fn search_pattern(pattern: &str, root: &Path, lang_filter: Option<&str>) -> Result<Vec<SearchHit>> {
    let Some(ast_grep) = find_ast_grep_binary() else {
        return Err(crate::StoreError::Other(
            "ast-grep not found: install from https://github.com/ast-grep/ast-grep".to_string(),
        ));
    };

    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let stdout = run_ast_grep(&ast_grep, pattern, &canonical_root, lang_filter)?;
    parse_ast_grep_json(&stdout, pattern, &canonical_root)
}

pub(super) fn pattern_hit(
    file: String,
    line_start: u32,
    line_end: u32,
    pattern: &str,
    language: Option<String>,
    excerpt: String,
) -> SearchHit {
    use crate::rank::SCORE_PATTERN;
    use crate::search::HitKind;

    SearchHit {
        kind: HitKind::Pattern,
        file,
        line_start,
        line_end,
        symbol: Some(pattern.to_string()),
        caller: None,
        callee: None,
        language,
        score: SCORE_PATTERN,
        excerpt,
    }
}
