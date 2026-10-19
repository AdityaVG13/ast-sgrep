use ast_sgrep_core::search::passes::symbol::*;

fn row(path: &str, name: &str, kind: &str, line: u32) -> SymbolSpanRow {
    (
        path.to_string(),
        Some("python".to_string()),
        name.to_string(),
        kind.to_string(),
        line,
        line,
    )
}

#[test]
fn quota_buckets_reserve_minority_slots_over_budget() {
    // 10 functions + 3 classes, budget 8: other_keep = max(8/2,4) = 4,
    // so all 3 classes survive and functions take the remaining 5.
    let mut rows = Vec::new();
    for i in 0..10 {
        rows.push(row(
            &format!("src/m{i}.py"),
            &format!("qzz_f{i}"),
            "function",
            1,
        ));
    }
    for i in 0..3 {
        rows.push(row(
            &format!("src/c{i}.py"),
            &format!("qzz_C{i}"),
            "class",
            1,
        ));
    }
    rows.sort_by(cmp_symbol_rows);
    let kept = quota_partition(rows, |row| kind_rank(row.3.as_str()), 8);
    assert_eq!(kept.len(), 8);
    let kinds: Vec<&str> = kept.iter().map(|row| row.3.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "function", "function", "function", "function", "function", "class", "class", "class",
        ],
        "functions first, then every surviving class"
    );
    // Within-bucket order is (path, line, name): deterministic across
    // index builds, never rowid order.
    let names: Vec<&str> = kept.iter().map(|row| row.2.as_str()).collect();
    assert_eq!(
        names,
        vec!["qzz_f0", "qzz_f1", "qzz_f2", "qzz_f3", "qzz_f4", "qzz_C0", "qzz_C1", "qzz_C2",]
    );
}

#[test]
fn quota_partition_is_noop_under_budget() {
    let mut rows = vec![
        row("src/b.py", "qzz_b", "function", 3),
        row("src/a.py", "qzz_A", "class", 1),
    ];
    rows.sort_by(cmp_symbol_rows);
    let kept = quota_partition(rows, |row| kind_rank(row.3.as_str()), 32);
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].2, "qzz_b");
    assert_eq!(kept[1].2, "qzz_A");
}

// INTENT: a flooded term's matches must not spend the whole admission
// window — round-robin across term groups keeps every term represented.
#[test]
fn term_fair_select_interleaves_term_groups() {
    let items: Vec<(usize, usize)> = (0..10)
        .map(|i| (0, i))
        .chain([(1, 100), (1, 101)])
        .collect();
    let kept = term_fair_select(items, |item| item.0, 8);
    let kept_terms: Vec<usize> = kept.iter().map(|item| item.0).collect();
    assert_eq!(kept_terms, vec![0, 1, 0, 1, 0, 0, 0, 0]);
}

#[test]
fn term_fair_select_is_identity_under_budget() {
    let items = vec![(0usize, "a"), (1, "b"), (0, "c")];
    let kept = term_fair_select(items.clone(), |item| item.0, 8);
    assert_eq!(kept, items);
}
