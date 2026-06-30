use crate::extract::{
    add_named_symbol, field_child, is_inside_kind, node_text, parse_and_extract, walk_tree,
    NodeHandlers,
};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};

pub struct JavaParser;

impl LanguageParser for JavaParser {
    fn language(&self) -> Language {
        Language::Java
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_and_extract(tree_sitter_java::LANGUAGE.into(), source, |tree, src| {
            let handlers = NodeHandlers::new(|ext, node, source| {
                match node.kind() {
                    "method_declaration" | "constructor_declaration" => {
                        let kind = if is_inside_kind(node, "class_declaration") {
                            SymbolKind::Method
                        } else {
                            SymbolKind::Function
                        };
                        add_named_symbol(ext, node, source, kind);
                    }
                    "field_declaration" => {
                        let mut cursor = node.walk();
                        for child in node.children(&mut cursor) {
                            if child.kind() == "variable_declarator" {
                                add_named_symbol(ext, &child, source, SymbolKind::Method);
                            }
                        }
                    }
                    "method_invocation" => {
                        if let Some(name_node) = field_child(node, "name") {
                            ext.add_call(node, source, &name_node);
                        }
                    }
                    "import_declaration" => {
                        if let Some(path) = java_import_path(node, source) {
                            ext.add_import(node, source, &path);
                        }
                    }
                    _ => {}
                }
            });
            walk_tree(tree, src, &handlers)
        })
    }
}

fn java_import_path(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "scoped_identifier" || child.kind() == "identifier" {
            if let Some(text) = node_text(&child, source) {
                if text != "import" && text != "static" {
                    return Some(text.to_string());
                }
            }
        }
        if let Some(path) = java_import_path(&child, source) {
            return Some(path);
        }
    }
    let ids: Vec<String> = crate::extract::collect_identifiers(node, source)
        .into_iter()
        .filter(|s| s != "import" && s != "static")
        .collect();
    if ids.is_empty() {
        None
    } else {
        Some(ids.join("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_constructors_fields_and_static_imports() {
        let src = r#"
import static java.util.Collections.emptyList;
import java.util.List;

public class Demo {
    private String name;
    public Demo() {}
    public void run() { helper(); }
    void helper() {}
}
"#;
        let result = JavaParser.parse(src).unwrap();
        assert!(result.imports.iter().any(|i| i.module_path.contains("Collections")));
        assert!(result.imports.iter().any(|i| i.module_path.contains("List")));
        assert!(result.symbols.iter().any(|s| s.name == "Demo"));
        assert!(result.symbols.iter().any(|s| s.name == "name"));
        assert!(result.symbols.iter().any(|s| s.name == "run"));
    }
}
