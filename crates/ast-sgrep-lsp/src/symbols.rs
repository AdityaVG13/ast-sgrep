//! Symbol resolution at cursor positions.

use ast_sgrep_core::store::SymbolRow;

pub fn line_at_index(content: &str, line_index: usize) -> Option<String> {
    content.split('\n').nth(line_index).map(|l| l.to_string())
}

pub fn innermost_symbol<'a>(
    symbols: &'a [SymbolRow],
    line_no: u32,
    byte_in_line: usize,
) -> Option<&'a SymbolRow> {
    symbols
        .iter()
        .filter(|sym| line_no >= sym.line_start && line_no <= sym.line_end)
        .min_by(|a, b| {
            symbol_tightness(a, line_no, byte_in_line)
                .cmp(&symbol_tightness(b, line_no, byte_in_line))
        })
}

fn symbol_tightness(sym: &SymbolRow, _line_no: u32, byte_in_line: usize) -> (u32, usize) {
    let line_span = sym.line_end - sym.line_start;
    if sym.line_start == sym.line_end && sym.byte_end > sym.byte_start {
        if byte_in_line >= sym.byte_start && byte_in_line <= sym.byte_end {
            return (0, sym.byte_end - sym.byte_start);
        }
    }
    (line_span, sym.byte_end.saturating_sub(sym.byte_start))
}
