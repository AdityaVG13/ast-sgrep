use regex::Regex;

use crate::{CallSite, ExtractionResult, ImportSite, Language, LanguageParser, SymbolDef, SymbolKind};

/// C# extractor using lightweight regex (tree-sitter-c-sharp needs TS 0.25+).
pub struct CSharpParser;

impl LanguageParser for CSharpParser {
    fn language(&self) -> Language {
        Language::CSharp
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        let method_re = Regex::new(
            r"(?m)^\s*(?:public|private|protected|internal|static|\s)+[\w<>\[\]?]+\s+(\w+)\s*\(",
        )?;
        let call_re = Regex::new(r"(\w+)\s*\(")?;
        let using_re = Regex::new(r"(?m)^\s*using\s+([\w.]+)\s*;")?;
        let line_starts = crate::extract::line_start_offsets(source);

        let mut result = ExtractionResult::default();
        let mut current_fn: Option<String> = None;
        for (i, line) in source.split('\n').enumerate() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let line_no = (i + 1) as u32;
            let (byte_start, byte_end) = line_byte_span(&line_starts, line_no, source.len());
            let trimmed = line.trim();

            if trimmed == "}" || trimmed.starts_with('}') {
                if let Some(ref name) = current_fn {
                    if let Some(sym) = result.symbols.iter_mut().rev().find(|s| &s.name == name) {
                        sym.line_end = line_no;
                        sym.byte_end = byte_end;
                    }
                }
                current_fn = None;
            }

            if let Some(cap) = method_re.captures(line) {
                let name = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                if !name.is_empty() && !matches!(name, "if" | "for") {
                    current_fn = Some(name.to_string());
                    result.symbols.push(SymbolDef {
                        name: name.to_string(),
                        kind: SymbolKind::Function,
                        line_start: line_no,
                        line_end: line_no,
                        byte_start,
                        byte_end,
                    });
                }
            }

            if let Some(ref caller) = current_fn {
                for cap in call_re.captures_iter(line) {
                    let callee = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                    if is_callable_name(callee) && callee != caller {
                        let m = cap.get(1).unwrap();
                        result.calls.push(CallSite {
                            caller: caller.clone(),
                            callee: callee.to_string(),
                            line: line_no,
                            byte_start: byte_start + m.start(),
                            byte_end: byte_start + m.end(),
                        });
                    }
                }
            }

            if let Some(cap) = using_re.captures(line) {
                result.imports.push(ImportSite {
                    module_path: cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string(),
                    line: line_no,
                });
            }
        }
        Ok(result)
    }
}

fn line_byte_span(line_starts: &[usize], line_no: u32, source_len: usize) -> (usize, usize) {
    let idx = line_no.saturating_sub(1) as usize;
    let start = line_starts.get(idx).copied().unwrap_or(0);
    let end = line_starts.get(idx + 1).copied().unwrap_or(source_len);
    (start, end.saturating_sub(1).max(start))
}

fn is_callable_name(name: &str) -> bool {
    !matches!(
        name,
        "if" | "for" | "while" | "switch" | "catch" | "return" | "new" | "typeof" | "public"
            | "private" | "static" | "void" | "string" | "int" | "var" | "foreach" | "lock"
            | "using" | "throw" | "await" | "async" | "get" | "set"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csharp_clears_function_scope_on_close_brace() {
        let src = r#"
public class Program {
    public static void Main() { Helper(); }
    public static void Helper() { }
    public static void Other() { Run(); }
    static void Run() { }
}"#;
        let result = CSharpParser.parse(src).unwrap();
        let main_calls: Vec<_> = result.calls.iter().filter(|c| c.caller == "Main").collect();
        assert_eq!(main_calls.len(), 1);
        assert_eq!(main_calls[0].callee, "Helper");
        assert!(
            !result
                .calls
                .iter()
                .any(|c| c.caller == "Helper" && c.callee == "Run")
        );
    }

    #[test]
    fn extracts_csharp_methods() {
        let src = r#"
public class Program {
    public static void Main() { ProcessRequest("x"); }
    public static string ProcessRequest(string input) { return input; }
}"#;
        let result = CSharpParser.parse(src).unwrap();
        assert!(result.symbols.iter().any(|s| s.name == "ProcessRequest"));
    }

    #[test]
    fn csharp_symbols_have_nonzero_byte_spans() {
        let src = "public class Program {\n    public static void Main() { }\n}";
        let result = CSharpParser.parse(src).unwrap();
        let main = result.symbols.iter().find(|s| s.name == "Main").unwrap();
        assert!(main.byte_end > main.byte_start);
    }
}
