//! Prepare / hash / extract-row helpers for indexing.
//! Extracted from `index.rs` (EXP-007 / F-002 prepare/hash cluster). Leaf-ward of
//! `Indexer`; watch-path helpers live in `index_watch` (EXP-008); FORCE_SIDECAR
//! stays in `index` (F-003).

use crate::index::{split_content_lines, IndexOptions, SplitLines};
use crate::store::{CallerRow, ImportRow, SymbolRow};
use ast_sgrep_lang::{detect_language, ExtractionResult, Language, ParserRegistry};
use blake3::Hasher;
use std::path::Path;
use std::time::SystemTime;

pub(crate) type ExtractedRows = (
    Vec<SymbolRow>,
    Vec<CallerRow>,
    Vec<ImportRow>,
    Vec<ast_sgrep_lang::PatternNode>,
);

pub(crate) struct PreparedFile {
    pub(crate) hash: String,
    pub(crate) body_hash: String,
    pub(crate) language: Option<String>,
    pub(crate) mtime_secs: i64,
    pub(crate) mtime_nanos: u32,
    pub(crate) lines: Vec<(u32, String)>,
    pub(crate) eol: String,
    pub(crate) symbols: Vec<SymbolRow>,
    pub(crate) callers: Vec<CallerRow>,
    pub(crate) imports: Vec<ImportRow>,
    pub(crate) pattern_nodes: Vec<ast_sgrep_lang::PatternNode>,
    pub(crate) semantic_chunks: Vec<crate::semantic_chunk::SemanticChunkInput>,
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum PrepareOutcome {
    Unchanged,
    Filtered,
    SkippedBinary,
    Failed(String),
    Ready(PreparedFile),
}

/// Hash with trailing blank/line-comment trivia removed. Equal ⇒ structure unchanged for trailing edits.
pub(crate) fn body_structure_hash(content: &str, language: Option<Language>) -> String {
    let mut end = content.len();
    let bytes = content.as_bytes();
    while end > 0 {
        while end > 0 && (bytes[end - 1] == b'\n' || bytes[end - 1] == b'\r') {
            end -= 1;
        }
        if end == 0 {
            break;
        }
        let line_start = content[..end].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = content[line_start..end].trim();
        if !is_trailing_trivia_line(line, language) {
            break;
        }
        end = line_start;
        if end > 0 && bytes[end - 1] == b'\n' {
            end -= 1;
        }
        if end > 0 && bytes[end - 1] == b'\r' {
            end -= 1;
        }
    }
    let mut h = Hasher::new();
    h.update(&bytes[..end]);
    h.finalize().to_hex().to_string()
}

/// Table-driven trailing trivia: hash-style vs C-family line/block comment prefixes.
fn is_trailing_trivia_line(line: &str, language: Option<Language>) -> bool {
    if line.is_empty() {
        return true;
    }
    const HASH_PREFIXES: &[&str] = &["#"];
    const C_FAMILY_PREFIXES: &[&str] = &["//", "/*", "*"];
    let prefixes: &[&str] = match language {
        Some(Language::Python | Language::Ruby) => HASH_PREFIXES,
        Some(
            Language::Rust
            | Language::TypeScript
            | Language::JavaScript
            | Language::Go
            | Language::Java
            | Language::CSharp
            | Language::Swift
            | Language::C
            | Language::Cpp
            | Language::Kotlin
            | Language::Php,
        ) => C_FAMILY_PREFIXES,
        None => return false,
    };
    prefixes.iter().any(|p| line.starts_with(p))
}

pub(crate) fn hash_content(content: &str) -> String {
    let mut h = Hasher::new();
    h.update(content.as_bytes());
    h.finalize().to_hex().to_string()
}

/// Shared prepare→upsert materialization: line split, body hash, optional semantic chunks.
pub(crate) struct UpsertMaterial {
    pub(crate) split: SplitLines,
    pub(crate) body_hash: String,
    pub(crate) semantic_chunks: Vec<crate::semantic_chunk::SemanticChunkInput>,
}

pub(crate) fn materialize_upsert(
    content: &str,
    language: Option<Language>,
    symbols: &[SymbolRow],
    callers: &[CallerRow],
    pattern_nodes: &[ast_sgrep_lang::PatternNode],
    embed_semantic: bool,
    body_hash: String,
) -> UpsertMaterial {
    let split = split_content_lines(content);
    let semantic_chunks = if embed_semantic {
        crate::semantic_chunk::build_semantic_chunks_with_patterns(
            symbols,
            callers,
            pattern_nodes,
            &split.lines,
            language.map(|l| l.as_str()),
        )
    } else {
        vec![]
    };
    UpsertMaterial {
        split,
        body_hash,
        semantic_chunks,
    }
}

pub(crate) fn prepare_file(
    abs: &Path,
    rel: &str,
    current_hash: Option<&str>,
    options: &IndexOptions,
    root_dir: &crate::io_bounds::RootDir,
    semantic_identity_ok: bool,
    perf_run_id: Option<u64>,
) -> PrepareOutcome {
    let source =
        match root_dir.read_text_capped(Path::new(rel), crate::io_bounds::MAX_INDEX_FILE_BYTES) {
            Ok(source) => source,
            Err(error) if error.is_binary_file() && detect_language(abs, None).is_none() => {
                return PrepareOutcome::SkippedBinary
            }
            Err(error) => return PrepareOutcome::Failed(error.to_string()),
        };
    let mtime = source.metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let (mtime_secs, mtime_nanos) = system_time_to_parts(mtime);
    let content = source.text;
    let hash = {
        let t0 = std::time::Instant::now();
        let h = hash_content(&content);
        if crate::perf_profile::enabled() {
            crate::perf_profile::record_sample_for(
                perf_run_id,
                "embed_hash",
                "index",
                "blake3 hash_content per file",
                t0.elapsed().as_micros() as u64,
                false,
            );
        }
        h
    };
    let language = detect_language(abs, Some(&content));
    if let Some(filter) = options.lang_filter.as_deref() {
        if language.is_none_or(|l| l.as_str() != filter) {
            return PrepareOutcome::Filtered;
        }
    }
    if !options.force_reindex && current_hash == Some(hash.as_str()) && semantic_identity_ok {
        return PrepareOutcome::Unchanged;
    }
    let (symbols, callers, imports, pattern_nodes) = match language {
        Some(lang) => {
            // One ParserRegistry per rayon worker — building all language parsers
            // on every file was pure fixed cost on the hot index path.
            thread_local! {
                static REGISTRY: ParserRegistry = ParserRegistry::new();
            }
            match REGISTRY.with(|registry| registry.parse(lang, &content)) {
                Ok(extraction) => rows_from_extraction(&extraction),
                Err(e) => {
                    return PrepareOutcome::Failed(format!(
                        "failed to parse {rel} as {}: {e}",
                        lang.as_str()
                    ))
                }
            }
        }
        None => (vec![], vec![], vec![], vec![]),
    };
    let material = materialize_upsert(
        &content,
        language,
        &symbols,
        &callers,
        &pattern_nodes,
        options.embed_semantic,
        body_structure_hash(&content, language),
    );
    PrepareOutcome::Ready(PreparedFile {
        hash,
        body_hash: material.body_hash,
        language: language.map(|l| l.as_str().to_string()),
        mtime_secs,
        mtime_nanos,
        lines: material.split.lines,
        eol: material.split.eol.to_string(),
        symbols,
        callers,
        imports,
        pattern_nodes,
        semantic_chunks: material.semantic_chunks,
    })
}

pub(crate) fn rows_from_extraction(extraction: &ExtractionResult) -> ExtractedRows {
    (
        extraction
            .symbols
            .iter()
            .map(|s| SymbolRow {
                name: s.name.clone(),
                kind: format!("{:?}", s.kind).to_lowercase(),
                line_start: s.line_start,
                line_end: s.line_end,
                byte_start: s.byte_start,
                byte_end: s.byte_end,
            })
            .collect(),
        extraction
            .calls
            .iter()
            .map(|c| CallerRow {
                caller: c.caller.clone(),
                callee: c.callee.clone(),
                line_no: c.line,
                byte_start: c.byte_start,
                byte_end: c.byte_end,
            })
            .collect(),
        extraction
            .imports
            .iter()
            .map(|i| ImportRow {
                module_path: i.module_path.clone(),
                line_no: i.line,
            })
            .collect(),
        extraction.pattern_nodes.clone(),
    )
}

pub(crate) fn should_prune_missing_files(walk_errors: bool) -> bool {
    !walk_errors
}

pub(crate) fn system_time_to_parts(time: SystemTime) -> (i64, u32) {
    let d = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    (d.as_secs() as i64, d.subsec_nanos())
}
