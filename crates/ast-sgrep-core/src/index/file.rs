use std::path::Path;
use std::time::SystemTime;

use ast_sgrep_lang::{detect_language, ExtractionResult};
use blake3::Hasher;

use super::types::FileIndexStats;
use super::Indexer;
use crate::store::{CallerRow, ImportRow, SymbolRow};
use crate::text::split_content_lines;
use crate::Result;

impl Indexer {
    pub fn index_file(&mut self, abs_path: &Path, rel_path: &str) -> Result<FileIndexStats> {
        let metadata = std::fs::metadata(abs_path)?;
        let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let (mtime_secs, mtime_nanos) = system_time_to_parts(mtime);

        let content = match std::fs::read_to_string(abs_path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                return Err(crate::StoreError::Other(format!("binary file: {rel_path}")));
            }
            Err(e) => return Err(e.into()),
        };

        self.index_content_at(rel_path, &content, abs_path, mtime_secs, mtime_nanos)
    }

    /// Index in-memory content (LSP `didChange` / unsaved buffers).
    pub fn index_content(&mut self, rel_path: &str, content: &str) -> Result<FileIndexStats> {
        let now = SystemTime::now();
        let (mtime_secs, mtime_nanos) = system_time_to_parts(now);
        self.index_content_at(rel_path, content, Path::new(rel_path), mtime_secs, mtime_nanos)
    }

    pub(super) fn index_content_at(
        &mut self,
        rel_path: &str,
        content: &str,
        lang_path: &Path,
        mtime_secs: i64,
        mtime_nanos: u32,
    ) -> Result<FileIndexStats> {
        let hash = hash_content(content);

        if !self.options.force_reindex {
            if let Some(stored_hash) = self.store().file_hash(rel_path)? {
                if stored_hash == hash {
                    return Ok(FileIndexStats {
                        skipped: true,
                        ..Default::default()
                    });
                }
            }
        }

        let language = detect_language(lang_path, Some(content));
        if let Some(ref lang_filter) = self.options.lang_filter {
            match language {
                Some(lang) if lang.as_str() == lang_filter.as_str() => {}
                _ => {
                    if self.store().file_hash(rel_path)?.is_some() {
                        self.store().remove_file(rel_path)?;
                    }
                    return Ok(FileIndexStats::default());
                }
            }
        }

        let split = split_content_lines(content);
        let lines = &split.lines;

        let (symbols, callers, imports) = if let Some(lang) = language {
            let extraction = self.parsers.parse(lang, content).map_err(|e| {
                crate::StoreError::Other(format!(
                    "failed to parse {rel_path} as {}: {e}",
                    lang.as_str()
                ))
            })?;
            rows_from_extraction(&extraction)
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };

        let sym_count = symbols.len();
        let caller_count = callers.len();
        let import_count = imports.len();

        let semantic_chunks = if self.options.embed_semantic {
            crate::semantic_chunk::build_semantic_chunks(&symbols, &callers, lines)
        } else {
            Vec::new()
        };

        self.store().upsert_file(
            rel_path,
            language.map(|l| l.as_str()),
            mtime_secs,
            mtime_nanos,
            &hash,
            lines,
            split.eol,
            &symbols,
            &callers,
            &imports,
            &semantic_chunks,
            self.options.embed_semantic,
            self.options.embed_backend.to_preference(),
        )?;

        Ok(FileIndexStats {
            symbols: sym_count,
            callers: caller_count,
            imports: import_count,
            skipped: false,
        })
    }
}

pub(super) fn hash_content(content: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(content.as_bytes());
    hasher.finalize().to_hex().to_string()
}

pub(super) fn rows_from_extraction(
    extraction: &ExtractionResult,
) -> (Vec<SymbolRow>, Vec<CallerRow>, Vec<ImportRow>) {
    let symbols = extraction
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
        .collect();
    let callers = extraction
        .calls
        .iter()
        .map(|c| CallerRow {
            caller: c.caller.clone(),
            callee: c.callee.clone(),
            line_no: c.line,
            byte_start: c.byte_start,
            byte_end: c.byte_end,
        })
        .collect();
    let imports = extraction
        .imports
        .iter()
        .map(|i| ImportRow {
            module_path: i.module_path.clone(),
            line_no: i.line,
        })
        .collect();
    (symbols, callers, imports)
}

pub(super) fn system_time_to_parts(time: SystemTime) -> (i64, u32) {
    let duration = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    (duration.as_secs() as i64, duration.subsec_nanos())
}
