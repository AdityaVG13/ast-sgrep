use ast_sgrep_core::search::passes::embed::member_indices_for_files;
use std::collections::HashSet;

#[test]
fn member_indices_match_linear_scan() {
    let paths: Vec<String> = vec![
        "b.rs".into(),
        "a.rs".into(),
        "a.rs".into(),
        "c.rs".into(),
        "a.rs".into(),
    ];
    let mut path_order: Vec<u32> = (0..paths.len() as u32).collect();
    path_order.sort_by(|&x, &y| paths[x as usize].cmp(&paths[y as usize]));
    let allowed = HashSet::from(["a.rs".into(), "c.rs".into(), "z.rs".into()]);
    let got = member_indices_for_files(&paths, &path_order, &allowed);
    let expect: Vec<usize> = paths
        .iter()
        .enumerate()
        .filter(|(_, p)| allowed.contains(*p))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(got, expect);
}


#[test]
fn python_docstring_below_def_populates_doc_field() {
    use ast_sgrep_core::semantic_chunk::build_semantic_chunks;
    use ast_sgrep_core::store::{CallerRow, SymbolRow};

    // PEP 257: the docstring is the first statement of the body — a string
    // literal directly below the def. The docs field must carry it so the
    // docs vector (and the `doc:` field text) exists for python functions,
    // not just for languages with doc comments above items.
    let lines: Vec<(u32, String)> = vec![
        "def send_json_response(data):".to_string(),
        "    \"\"\"Serialize data to JSON and create a response object.\"\"\"".to_string(),
        "    body = json.dumps(data)".to_string(),
        "    return make_response(body)".to_string(),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, l)| (i as u32 + 1, l.clone()))
    .collect();
    let symbols = [SymbolRow {
        name: "send_json_response".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 4,
        byte_start: 0,
        byte_end: 400,
    }];
    let chunks = build_semantic_chunks(&symbols, &[] as &[CallerRow], &lines, Some("python"));
    let chunk = chunks
        .iter()
        .find(|c| c.symbol_name == "send_json_response")
        .expect("chunk emitted");
    assert!(
        chunk.doc.contains("Serialize data to JSON"),
        "docstring below def must populate the doc field (got {:?})",
        chunk.doc
    );

    // A function whose first body statement is code has no docstring: doc
    // stays empty (no false capture of arbitrary strings).
    let code_first: Vec<(u32, String)> = vec![
        "def compute(x):".to_string(),
        "    msg = \"not a docstring\"".to_string(),
        "    return x".to_string(),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, l)| (i as u32 + 1, l.clone()))
    .collect();
    let symbols2 = [SymbolRow {
        name: "compute".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 3,
        byte_start: 0,
        byte_end: 100,
    }];
    let chunks2 =
        build_semantic_chunks(&symbols2, &[] as &[CallerRow], &code_first, Some("python"));
    assert!(
        chunks2
            .iter()
            .find(|c| c.symbol_name == "compute")
            .expect("chunk emitted")
            .doc
            .is_empty(),
        "code-first body must not be treated as a docstring"
    );
}
