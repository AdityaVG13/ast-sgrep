use crate::types::{
    Position, Range, TextDocumentContentChangeEvent, SYMBOL_KIND_FUNCTION, SYMBOL_KIND_METHOD,
    SYMBOL_KIND_STRING,
};
use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::store::SymbolRow;
use ast_sgrep_core::{EmbedBackend, IndexOptions, SearchHit, SearchOptions};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let Some(len) = read_content_length(reader)? else {
        return Ok(None);
    };
    if len > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Content-Length {len} exceeds max {MAX_MESSAGE_BYTES}"),
        ));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    String::from_utf8(buf)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
fn read_content_length(reader: &mut impl BufRead) -> io::Result<Option<usize>> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            content_length = Some(rest.trim().parse().map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("bad Content-Length: {e}"),
                )
            })?);
        }
    }
    content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header"))
        .map(Some)
}
pub fn write_message(writer: &mut impl Write, body: &str) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    writer.flush()
}
pub fn send_response(writer: &mut impl Write, id: &Value, result: Value) -> io::Result<()> {
    write_message(
        writer,
        &serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
    )
}
pub fn send_error(writer: &mut impl Write, id: &Value, code: i64, message: &str) -> io::Result<()> {
    write_message(writer, &serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string())
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsgrepSettings {
    pub no_embed: Option<bool>,
    pub cloud_embed: Option<bool>,
    pub ollama_embed: Option<bool>,
    pub semantic_only: Option<bool>,
    pub ann_threshold: Option<usize>,
    pub embed_backend: Option<String>,
    pub index_path: Option<String>,
}
impl AsgrepSettings {
    pub fn from_initialization_options(value: &Value) -> Self {
        if let Some(nested) = value.get("asgrep") {
            if let Ok(s) = serde_json::from_value(nested.clone()) {
                return s;
            }
        }
        serde_json::from_value(value.clone()).unwrap_or_default()
    }
    fn apply_path_ann(&self, index_path: &mut Option<PathBuf>, ann: &mut Option<usize>) {
        if let Some(t) = self.ann_threshold {
            *ann = Some(t);
        }
        if let Some(ref p) = self.index_path {
            *index_path = Some(PathBuf::from(p));
        }
    }
    pub fn apply_to_index_options(&self, opts: &mut IndexOptions) {
        if let Some(no) = self.no_embed {
            opts.embed_semantic = !no;
        }
        if let Some(ref backend) = self.embed_backend {
            opts.embed_backend = EmbedBackend::parse(backend);
        }
        if self.cloud_embed == Some(true) {
            opts.embed_backend = EmbedBackend::Cloud;
        }
        if self.ollama_embed == Some(true) {
            opts.embed_backend = EmbedBackend::Ollama;
        }
        if self.semantic_only == Some(true) {
            opts.embed_backend = EmbedBackend::Semantic;
        }
        self.apply_path_ann(&mut opts.index_path, &mut opts.ann_threshold);
    }
    pub fn apply_to_search_options(&self, opts: &mut SearchOptions) {
        if let Some(no) = self.no_embed {
            opts.use_embed = !no;
        }
        if let Some(c) = self.cloud_embed {
            opts.use_cloud_embed = c;
        }
        if let Some(o) = self.ollama_embed {
            opts.use_ollama_embed = o;
        }
        if let Some(s) = self.semantic_only {
            opts.use_semantic_only = s;
        }
        self.apply_path_ann(&mut opts.index_path, &mut opts.ann_threshold);
    }
}
pub fn line_at_index(content: &str, line_index: usize) -> Option<String> {
    content.split('\n').nth(line_index).map(str::to_string)
}
pub fn innermost_symbol(
    symbols: &[SymbolRow],
    line_no: u32,
    byte_in_line: usize,
) -> Option<&SymbolRow> {
    symbols
        .iter()
        .filter(|s| line_no >= s.line_start && line_no <= s.line_end)
        .min_by(|a, b| tightness(a, byte_in_line).cmp(&tightness(b, byte_in_line)))
}
fn tightness(sym: &SymbolRow, byte_in_line: usize) -> (u32, usize) {
    let span = sym.line_end - sym.line_start;
    if sym.line_start == sym.line_end
        && sym.byte_end > sym.byte_start
        && byte_in_line >= sym.byte_start
        && byte_in_line <= sym.byte_end
    {
        return (0, sym.byte_end - sym.byte_start);
    }
    (span, sym.byte_end.saturating_sub(sym.byte_start))
}
pub fn path_to_file_uri(path: &Path) -> String {
    let s = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/");
    if s.len() >= 2 && s.as_bytes()[1] == b':' {
        format!("file:///{}", pct_enc(&s))
    } else if s.starts_with('/') {
        format!("file://{}", pct_enc(&s))
    } else {
        format!("file:///{}", pct_enc(&s))
    }
}
pub fn file_uri_to_path(uri: &str) -> anyhow::Result<PathBuf> {
    let rest = uri
        .strip_prefix("file://")
        .or_else(|| uri.strip_prefix("file:"))
        .ok_or_else(|| anyhow::anyhow!("not a file URI: {uri}"))?;
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let decoded = pct_dec(rest);
    let path = if rest.starts_with('/') && decoded.len() >= 2 && decoded.as_bytes()[1] == b':' {
        decoded.trim_start_matches('/').to_string()
    } else {
        decoded
    };
    Ok(PathBuf::from(path))
}
pub fn uri_to_rel_path(uri: &str, root: &Path) -> anyhow::Result<String> {
    let abs = file_uri_to_path(uri)?;
    let croots = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let cabs = abs.canonicalize().unwrap_or(abs);
    if !cabs.starts_with(&croots) {
        anyhow::bail!("document URI outside workspace root");
    }
    let rel = cabs
        .strip_prefix(&croots)
        .map_err(|_| anyhow::anyhow!("document URI outside workspace root"))?;
    if rel.components().any(|c| matches!(c, Component::ParentDir)) {
        anyhow::bail!("path traversal in document URI");
    }
    Ok(rel.to_string_lossy().replace('\\', "/"))
}
pub fn canonicalize_workspace_root(root: PathBuf) -> PathBuf {
    root.canonicalize().unwrap_or(root)
}
fn pct_enc(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
fn pct_dec(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
pub fn utf16_char_to_byte(line: &str, utf16_offset: u32) -> usize {
    let mut u = 0u32;
    for (bi, ch) in line.char_indices() {
        let units = ch.len_utf16() as u32;
        if utf16_offset < u + units {
            return bi;
        }
        u += units;
    }
    line.len()
}
pub fn apply_text_edit(content: &str, change: &TextDocumentContentChangeEvent) -> String {
    let Some(range) = &change.range else {
        return change.text.clone();
    };
    let start = pos_to_byte(content, &range.start);
    let end = change.range_length.map_or_else(
        || pos_to_byte(content, &range.end),
        |len| utf16_span_end(content, &range.start, len),
    );
    if start > end || end > content.len() {
        return content.to_string();
    }
    let mut out = String::with_capacity(content.len().saturating_add(change.text.len()));
    out.push_str(&content[..start]);
    out.push_str(&change.text);
    out.push_str(&content[end..]);
    out
}
fn utf16_span_end(content: &str, start: &Position, utf16_len: u32) -> usize {
    let sb = pos_to_byte(content, start);
    let mut u = 0u32;
    for (bi, ch) in content[sb..].char_indices() {
        u += ch.len_utf16() as u32;
        if u >= utf16_len {
            return sb + bi + ch.len_utf8();
        }
    }
    content.len()
}
fn pos_to_byte(content: &str, pos: &Position) -> usize {
    let mut offset = 0usize;
    for (line_no, line) in content.split_inclusive('\n').enumerate() {
        if line_no as u32 == pos.line {
            let body = line.strip_suffix('\n').unwrap_or(line);
            return offset + utf16_char_to_byte(body, pos.character);
        }
        offset += line.len();
    }
    content.len()
}
pub fn extract_identifier_at(line: &str, byte_offset: usize) -> Option<String> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let idx = ident_idx(&chars, byte_offset)?;
    let (lo, hi) = ident_span(line, &chars, idx);
    let ident = line.get(lo..hi)?.trim();
    (!ident.is_empty()).then(|| ident.to_string())
}
fn ident_idx(chars: &[(usize, char)], byte_offset: usize) -> Option<usize> {
    let mut idx = chars
        .iter()
        .position(|(o, _)| *o >= byte_offset)
        .unwrap_or_else(|| chars.len().saturating_sub(1));
    if !is_ident(chars[idx].1) && idx > 0 {
        idx -= 1;
    }
    is_ident(chars[idx].1).then_some(idx)
}
fn ident_span(line: &str, chars: &[(usize, char)], idx: usize) -> (usize, usize) {
    let mut lo = idx;
    let mut hi = idx;
    while lo > 0 && is_ident(chars[lo - 1].1) {
        lo -= 1;
    }
    while hi + 1 < chars.len() && is_ident(chars[hi + 1].1) {
        hi += 1;
    }
    (
        chars[lo].0,
        chars.get(hi + 1).map(|(o, _)| *o).unwrap_or(line.len()),
    )
}
fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
pub fn workspace_symbol(root: &Path, file: &str, hit: &SearchHit) -> Option<Value> {
    let name = hit
        .symbol
        .clone()
        .or_else(|| hit.callee.clone())
        .unwrap_or_else(|| hit.excerpt.chars().take(60).collect());
    let kind = match hit.kind {
        HitKind::Embed => SYMBOL_KIND_STRING,
        HitKind::Caller | HitKind::Graph => SYMBOL_KIND_METHOD,
        _ => SYMBOL_KIND_FUNCTION,
    };
    let detail = match hit.kind {
        HitKind::Embed => format!("semantic · score {:.2}", hit.score),
        other => format!("{} · score {:.2}", other.as_str(), hit.score),
    };
    Some(json!({
        "name": name, "kind": kind, "location": location_value(root, file, hit.line_start, hit.line_end), "containerName": file, "detail": detail,
        "data": { "asgrepKind": hit.kind.as_str(), "score": hit.score, "excerpt": hit.excerpt.chars().take(120).collect::<String>(), "semantic": hit.kind == HitKind::Embed }
    }))
}
pub fn location_value(root: &Path, file: &str, line_start: u32, line_end: u32) -> Value {
    json!({ "uri": path_to_file_uri(&root.join(file)), "range": line_range(line_start, line_end) })
}
pub fn line_range(line_start: u32, line_end: u32) -> Range {
    line_range_ext(line_start, line_end, None)
}
pub fn line_range_ext(line_start: u32, line_end: u32, end_line_text: Option<&str>) -> Range {
    Range {
        start: Position {
            line: line_start.saturating_sub(1),
            character: 0,
        },
        end: Position {
            line: line_end.saturating_sub(1),
            character: end_line_text.map(line_utf16_len).unwrap_or(0),
        },
    }
}
pub fn call_hierarchy_endpoint(root: &Path, file: &str, line: u32, name: &str) -> Value {
    let range = line_range(line, line);
    json!({ "name": name, "kind": SYMBOL_KIND_FUNCTION, "uri": path_to_file_uri(&root.join(file)), "range": range, "selectionRange": range })
}
fn line_utf16_len(line: &str) -> u32 {
    line.chars().map(|c| c.len_utf16() as u32).sum()
}
