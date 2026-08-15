use super::*;

fn function(line_start: u32, line_end: u32) -> SymbolRow {
    SymbolRow {
        name: "renew_account".into(),
        kind: "function".into(),
        line_start,
        line_end,
        byte_start: 0,
        byte_end: 100,
    }
}

#[test]
fn maps_distinct_ast_children_back_to_the_parent_symbol() {
    let symbol = function(2, 8);
    let nodes = vec![
        PatternNode {
            signature: "decl:fn:renew_account".into(),
            line_start: 2,
            line_end: 8,
            excerpt: "whole parent".into(),
        },
        PatternNode {
            signature: "call:charge".into(),
            line_start: 4,
            line_end: 4,
            excerpt: "charge(subscription)".into(),
        },
        PatternNode {
            signature: "identifier".into(),
            line_start: 4,
            line_end: 4,
            excerpt: "charge".into(),
        },
        PatternNode {
            signature: "call:notify".into(),
            line_start: 6,
            line_end: 6,
            excerpt: "notify_customer()".into(),
        },
    ];
    let lines = [(2, "whole parent".into())];
    let chunks = build_semantic_chunks_with_patterns(&[symbol], &[], &nodes, &lines, None);
    // Bounded by MAX_CHILD_CHUNKS_PER_PARENT: the two call: nodes win
    // priority; the bare identifier is dropped.
    assert_eq!(chunks.len(), 2);
    assert!(chunks
        .iter()
        .all(|chunk| (chunk.line_start, chunk.line_end) == (2, 8)));
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.excerpt.as_str())
            .collect::<Vec<_>>(),
        vec!["charge(subscription)", "notify_customer()"]
    );
}

#[test]
fn assigns_nested_nodes_only_to_the_nearest_parent() {
    let mut outer = function(1, 10);
    outer.name = "outer".into();
    outer.byte_end = 200;
    let mut inner = function(3, 5);
    inner.name = "inner".into();
    inner.byte_start = 40;
    inner.byte_end = 80;
    let lines = (1..=10)
        .map(|line| (line, format!("line {line}")))
        .collect::<Vec<_>>();
    let nodes = [PatternNode {
        signature: "call:inside".into(),
        line_start: 4,
        line_end: 4,
        excerpt: "inside_call()".into(),
    }];
    let chunks = build_semantic_chunks_with_patterns(&[outer, inner], &[], &nodes, &lines, None);
    let owners = chunks
        .iter()
        .filter(|chunk| chunk.excerpt == "inside_call()")
        .map(|chunk| chunk.symbol_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(owners, vec!["inner"]);
}

#[test]
fn keeps_a_child_from_a_one_line_parent() {
    let lines = [(1, "fn renew_account() { charge() }".to_string())];
    let nodes = [
        PatternNode {
            signature: "decl:fn:renew_account".into(),
            line_start: 1,
            line_end: 1,
            excerpt: lines[0].1.clone(),
        },
        PatternNode {
            signature: "call:charge".into(),
            line_start: 1,
            line_end: 1,
            excerpt: "charge()".into(),
        },
    ];
    let chunks = build_semantic_chunks_with_patterns(&[function(1, 1)], &[], &nodes, &lines, None);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].excerpt, "charge()");
    assert_eq!((chunks[0].line_start, chunks[0].line_end), (1, 1));
}

#[test]
fn maps_top_level_nodes_to_a_file_parent() {
    let lines = [
        (1, "const TIMEOUT: u64 = 30;".into()),
        (2, "type UserId = String;".into()),
    ];
    let nodes = [PatternNode {
        signature: "constant:TIMEOUT".into(),
        line_start: 1,
        line_end: 1,
        excerpt: "const TIMEOUT: u64 = 30;".into(),
    }];
    let chunks = build_semantic_chunks_with_patterns(&[], &[], &nodes, &lines, None);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].kind, "file");
    assert!(chunks[0].symbol_name.is_empty());
    assert_eq!((chunks[0].line_start, chunks[0].line_end), (1, 2));
}

#[test]
fn bounds_children_and_falls_back_to_the_parent_excerpt() {
    let nodes = (2..=50)
        .map(|line| PatternNode {
            signature: format!("identifier:{line}"),
            line_start: line,
            line_end: line,
            excerpt: format!("child_{line}"),
        })
        .collect::<Vec<_>>();
    let chunks = build_semantic_chunks_with_patterns(&[function(1, 60)], &[], &nodes, &[], None);
    assert_eq!(chunks.len(), MAX_CHILD_CHUNKS_PER_PARENT);

    let lines = [(1, "fn renew_account() {}".into())];
    let fallback = build_semantic_chunks_with_patterns(&[function(1, 1)], &[], &[], &lines, None);
    assert_eq!(fallback.len(), 1);
    assert_eq!(fallback[0].excerpt, "fn renew_account() {}");
}

#[test]
fn rust_derive_attribute_is_not_doc_comment() {
    let symbols = [SymbolRow {
        name: "foo".into(),
        kind: "function".into(),
        line_start: 2,
        line_end: 2,
        byte_start: 20,
        byte_end: 40,
    }];
    let lines = [(1u32, "#[derive(Debug)]".into()), (2, "fn foo() {}".into())];
    let chunks = build_semantic_chunks_with_patterns(&symbols, &[], &[], &lines, Some("rust"));
    assert_eq!(chunks.len(), 1);
    assert!(
        chunks[0].doc.is_empty(),
        "#[derive] must not become doc text; got {:?}",
        chunks[0].doc
    );
    let rendered = render_chunk_text(&chunks[0]);
    assert!(
        !rendered.contains("doc:"),
        "rendered chunk must not inject derive as doc; got {rendered}"
    );
}

#[test]
fn render_chunk_text_puts_body_before_metadata() {
    let chunk = SemanticChunkInput {
        symbol_name: "renew_account".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 3,
        excerpt: "fn renew_account() { charge(subscription) }".into(),
        callers: vec!["main".into()],
        callees: vec!["charge".into()],
        doc: "renews the billing account".into(),
        scope: "Billing".into(),
    };
    let rendered = render_chunk_text(&chunk);
    let excerpt_at = rendered.find("excerpt:").expect("excerpt field");
    for field in ["symbol:", "kind:", "scope:", "doc:", "called_by:", "calls:"] {
        let at = rendered.find(field).unwrap_or_else(|| panic!("{field}"));
        assert!(
            excerpt_at < at,
            "body must precede {field} so metadata is what truncates; got {rendered}"
        );
    }
    assert!(
        rendered.starts_with("excerpt:"),
        "rendered text must start with the body; got {rendered}"
    );
}

#[test]
fn chunk_field_texts_split_name_docs_body_graph() {
    let chunk = SemanticChunkInput {
        symbol_name: "renew_account".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 3,
        excerpt: "fn renew_account() { charge(subscription) }".into(),
        callers: vec!["main".into()],
        callees: vec!["charge".into()],
        doc: "renews the billing account".into(),
        scope: "Billing".into(),
    };
    let fields = chunk_field_texts(&chunk);
    assert!(fields.name.contains("renew_account"), "{}", fields.name);
    assert!(fields.name.contains("Billing"), "{}", fields.name);
    assert!(
        fields.docs.contains("renews the billing account"),
        "{}",
        fields.docs
    );
    assert!(
        fields
            .body
            .contains("fn renew_account() { charge(subscription) }"),
        "{}",
        fields.body
    );
    assert!(fields.graph.contains("main"), "{}", fields.graph);
    assert!(fields.graph.contains("charge"), "{}", fields.graph);
    assert!(
        !fields.body.contains("called_by:"),
        "body field must not mix graph text: {}",
        fields.body
    );
    assert!(
        !fields.name.contains("excerpt:"),
        "name field must not mix body text: {}",
        fields.name
    );
}

#[test]
fn rust_line_doc_comments_still_captured() {
    let symbols = [SymbolRow {
        name: "foo".into(),
        kind: "function".into(),
        line_start: 2,
        line_end: 2,
        byte_start: 20,
        byte_end: 40,
    }];
    let lines = [(1u32, "/// does a thing".into()), (2, "fn foo() {}".into())];
    let chunks = build_semantic_chunks_with_patterns(&symbols, &[], &[], &lines, Some("rust"));
    assert_eq!(chunks[0].doc, "does a thing");
}

#[test]
fn typescript_private_field_hash_is_not_doc_comment() {
    let symbols = [SymbolRow {
        name: "method".into(),
        kind: "method".into(),
        line_start: 2,
        line_end: 2,
        byte_start: 20,
        byte_end: 40,
    }];
    let lines = [(1u32, "  #foo = 1;".into()), (2, "  method() {}".into())];
    let chunks =
        build_semantic_chunks_with_patterns(&symbols, &[], &[], &lines, Some("typescript"));
    assert_eq!(chunks.len(), 1);
    assert!(
        chunks[0].doc.is_empty(),
        "TS private field #foo must not become doc; got {:?}",
        chunks[0].doc
    );
}

#[test]
fn python_hash_comments_still_captured() {
    let symbols = [SymbolRow {
        name: "foo".into(),
        kind: "function".into(),
        line_start: 2,
        line_end: 2,
        byte_start: 20,
        byte_end: 40,
    }];
    let lines = [(1u32, "# helper".into()), (2, "def foo():".into())];
    let chunks = build_semantic_chunks_with_patterns(&symbols, &[], &[], &lines, Some("python"));
    assert_eq!(chunks[0].doc, "helper");
}
