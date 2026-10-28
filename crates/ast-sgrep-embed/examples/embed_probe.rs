//! Offline evidence probe: embed arbitrary texts through the shipped static
//! engine. Each stdin line is `id<TAB>MODE<TAB>text` where MODE is
//! `E` (apply `expand_concepts`, exactly like index-time chunk text) or
//! `R` (raw, exactly like query-side embedding). Text uses `\n`, `\t`,
//! `\\` escapes (chunk texts are multi-line). Output: `id<TAB>v1,v2,...`.
//! Used by retrieval-quality investigations (chunk-text composition probes)
//! to measure candidate representations without touching the pipeline.
use std::io::{self, BufRead, Write};

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let Some((id, rest)) = line.split_once('\t') else {
            continue;
        };
        let (mode, text) = match rest.split_once('\t') {
            Some((m, t)) => (m, t),
            None => continue,
        };
        let text = unescape(text);
        let final_text = if mode.eq_ignore_ascii_case("E") {
            ast_sgrep_embed::expand_concepts(&text)
        } else {
            text
        };
        let vec = ast_sgrep_embed::embed_text(&final_text);
        let csv = vec
            .iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let _ = writeln!(out, "{id}\t{csv}");
    }
}
