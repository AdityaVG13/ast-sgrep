use ast_sgrep_core::search::passes::lexical::hits_from_matches;
use std::collections::HashMap;

/// The lexical channel must emit hits in (path, line_no) order.
/// LineMatches is a randomly seeded HashMap, so an unsorted
/// `into_iter` re-rolls the emission order every call and every process.
#[test]
fn lexical_emission_is_key_sorted_every_call() {
    type SeedRow = (Vec<usize>, Option<String>, String);
    let build = || {
        let mut matches: HashMap<(String, u32), SeedRow> = HashMap::new();
        for i in 0..6 {
            matches.insert(
                (format!("src/mod{i}.rs"), 7),
                (vec![i as usize], Some("rust".into()), "row".into()),
            );
        }
        matches
    };
    let reference = hits_from_matches(build());
    for run in 0..20 {
        let hits = hits_from_matches(build());
        let keys: Vec<(String, u32)> = hits
            .iter()
            .map(|hit| (hit.file.clone(), hit.line_start))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "run {run}: emission must be key-sorted");
        let reference_keys: Vec<(String, u32)> = reference
            .iter()
            .map(|hit| (hit.file.clone(), hit.line_start))
            .collect();
        assert_eq!(keys, reference_keys, "run {run}: emission must be stable");
    }
}
