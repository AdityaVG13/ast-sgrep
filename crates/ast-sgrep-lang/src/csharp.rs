use regex::Regex;

use crate::{CallSite, ExtractionResult, ImportSite, Language, LanguageParser, SymbolDef, SymbolKind};

/// C# extractor using lightweight regex (tree-sitter-c-sharp requires TS 0.25+).
pub struct CSharpParser;

impl LanguageParser for CSharpParser {
    fn language(&self) -> Language {
        Language::CSharp
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        let mut result = ExtractionResult::default();
        let method_re = Regex::new(
            r"(?m)^\s*(?:public|private|protected|internal|static|\s)+[\w<>\[\]?]+\s+(\w+)\s*\(",
        )?;
        let call_re = Regex::new(r"(\w+)\s*\(")?;
        let using_re = Regex::new(r"(?m)^\s*using\s+([\w.]+)\s*;")?;

        let mut current_fn: Option<String> = None;
        for (i, line) in source.lines().enumerate() {
            let line_no = (i + 1) as u32;
            if let Some(cap) = method_re.captures(line) {
                let name = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
                if !name.is_empty() && name != "if" && name != "for" {
                    current_fn = Some(name.clone());
                    result.symbols.push(SymbolDef {
                        name,
                        kind: SymbolKind::Function,
                        line_start: line_no,
                        line_end: line_no,
                        byte_start: 0,
                        byte_end: line.len(),
                    });
                }
            }
            if let Some(ref caller) = current_fn {
                for cap in call_re.captures_iter(line) {
                    let callee = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
                    if is_callable_name(&callee) && callee != *caller {
                        result.calls.push(CallSite {
                            caller: caller.clone(),
                            callee,
                            line: line_no,
                            byte_start: 0,
                            byte_end: line.len(),
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

fn is_callable_name(name: &str) -> bool {
    !matches!(
        name,
        "if" | "for" | "while" | "switch" | "catch" | "return" | "new" | "typeof" | "public"
            | "private" | "static" | "void" | "string" | "int" | "var"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
