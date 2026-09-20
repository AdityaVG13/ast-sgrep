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
