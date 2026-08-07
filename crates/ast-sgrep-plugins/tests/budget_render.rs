//! m38g: budget chooses detail per result; excerpts stay verifiable source.
use ast_sgrep_core::search::{HitKind, HitSignal, SearchHit};
use ast_sgrep_plugins::budget::{plan_cost, render, select, DetailLevel, OutputBudget, GAP_MARKER};

fn long_function(name: &str) -> String {
    let mut body = format!("fn {name}(session: &Session) -> Result<Token> {{\n");
    for index in 0..40 {
        if index == 20 {
            body.push_str("    if session.is_expired() {\n");
            body.push_str("        return rotate_credentials(session);\n");
            body.push_str("    }\n");
        } else {
            body.push_str(&format!("    let step_{index} = compute({index});\n"));
        }
    }
    body.push_str("}\n");
    body
}

fn hit(name: &str, score: f64) -> SearchHit {
    SearchHit {
        kind: HitKind::Def,
        file: format!("src/{name}.rs"),
        line_start: 1,
        line_end: 44,
        symbol: Some(name.to_owned()),
        caller: None,
        callee: None,
        language: Some("rust".into()),
        score,
        signal: HitSignal::Exact,
        contributors: vec![HitKind::Def],
        margin: 0.0,
        confidence: 0.8,
        resolution: None,
        excerpt: long_function(name),
    }
}

#[test]
fn detail_levels_cost_strictly_more_as_they_show_more() {
    let hit = hit("refresh_token", 9.0);
    let mut previous = 0;
    for level in DetailLevel::ALL {
        let rendered = render(&hit, level);
        assert!(
            rendered.cost >= previous,
            "{level:?} must not cost less than a lesser level"
        );
        previous = rendered.cost;
    }
    assert_eq!(render(&hit, DetailLevel::Metadata).cost, 0);
    assert!(render(&hit, DetailLevel::Full).cost > render(&hit, DetailLevel::Block).cost);
}

#[test]
fn block_detail_keeps_signature_and_control_flow_and_marks_gaps() {
    let hit = hit("refresh_token", 9.0);
    let block = render(&hit, DetailLevel::Block).body;

    assert!(
        block.starts_with("fn refresh_token(session: &Session) -> Result<Token> {"),
        "declaration must survive: {block}"
    );
    assert!(
        block.contains("if session.is_expired() {"),
        "control flow must survive: {block}"
    );
    assert!(
        block.contains(GAP_MARKER),
        "omitted source must be marked: {block}"
    );
    // Every emitted line is real source, or a gap marker. Nothing invented.
    for line in block.lines() {
        let trimmed = line.trim();
        assert!(
            trimmed == GAP_MARKER || hit.excerpt.contains(trimmed),
            "line is not verifiable source: {line}"
        );
    }
}

#[test]
fn budget_is_respected_and_spends_on_the_top_result_first() {
    let hits = vec![hit("alpha", 9.0), hit("beta", 5.0), hit("gamma", 1.0)];
    let tight = OutputBudget {
        max_tokens: 220,
        default_detail: DetailLevel::Full,
    };
    let plan = select(&hits, tight);

    assert_eq!(plan.len(), 3, "a budget degrades detail, never drops results");
    assert!(
        plan_cost(&plan) <= tight.max_tokens,
        "plan cost {} exceeded budget {}",
        plan_cost(&plan),
        tight.max_tokens
    );
    assert!(
        plan[0].detail >= plan[2].detail,
        "rank order must be funded first: {:?} vs {:?}",
        plan[0].detail,
        plan[2].detail
    );
}

#[test]
fn a_generous_budget_upgrades_everything_and_a_zero_budget_still_lists_results() {
    let hits = vec![hit("alpha", 9.0), hit("beta", 5.0)];

    let generous = select(
        &hits,
        OutputBudget {
            max_tokens: 100_000,
            default_detail: DetailLevel::Full,
        },
    );
    assert!(generous.iter().all(|r| r.detail == DetailLevel::Full));

    let zero = select(
        &hits,
        OutputBudget {
            max_tokens: 0,
            default_detail: DetailLevel::Full,
        },
    );
    assert_eq!(zero.len(), 2, "results stay addressable at zero budget");
    assert!(zero.iter().all(|r| r.detail == DetailLevel::Metadata));
    assert_eq!(plan_cost(&zero), 0);
}

#[test]
fn selection_is_deterministic() {
    let hits = vec![hit("alpha", 9.0), hit("beta", 5.0), hit("gamma", 1.0)];
    let budget = OutputBudget {
        max_tokens: 700,
        default_detail: DetailLevel::Full,
    };
    let first = select(&hits, budget);
    for _ in 0..8 {
        assert_eq!(select(&hits, budget), first, "selection must be stable");
    }
}

#[test]
fn tighter_budgets_never_produce_larger_output() {
    let hits = vec![hit("alpha", 9.0), hit("beta", 5.0), hit("gamma", 1.0)];
    let mut previous = 0;
    for max_tokens in [0, 100, 300, 900, 5_000] {
        let cost = plan_cost(&select(
            &hits,
            OutputBudget {
                max_tokens,
                default_detail: DetailLevel::Full,
            },
        ));
        assert!(
            cost >= previous,
            "raising the budget must not shrink output ({previous} -> {cost})"
        );
        assert!(cost <= max_tokens.max(0), "cost {cost} exceeded {max_tokens}");
        previous = cost;
    }
}
