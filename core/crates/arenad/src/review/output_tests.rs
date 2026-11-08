use serde_json::json;

use super::*;

fn citable() -> Citable {
    Citable {
        steps_a: [3, 30, 110].into(),
        steps_b: [5, 34].into(),
        runs_a: ["ra".to_string()].into(),
        runs_b: ["rb".to_string()].into(),
        agents: ["supply".to_string()].into(),
    }
}

#[test]
fn json_is_found_bare_fenced_or_inside_text() {
    assert_eq!(json_in("{\"a\":1}").unwrap()["a"], 1);
    assert_eq!(json_in("```json\n{\"a\":2}\n```").unwrap()["a"], 2);
    assert_eq!(json_in("Here it is: {\"a\":3} done").unwrap()["a"], 3);
    assert!(json_in("no json here").is_err());
    assert!(json_in("[1, 2]").is_err());
}

fn messy_run_answer() -> Value {
    json!({
        "verdict": "The deviation was never opened.",
        "went_wrong": [
            {"title": "Deviation not created", "why": "insert_doc lacked a subject",
             "steps": [110, 999], "severity": "Goal"},
            {"title": "Invented", "why": "cites nothing real", "steps": [998], "severity": "safety"},
            {"title": "No severity", "why": "w", "steps": [], "severity": "bad"},
            {"why": "no title"}
        ],
        "suggestions": [
            {"change": "Name the Subject field", "why": "the ERP requires it", "kind": "prompt",
             "agent": "supply", "prompt_text": "Always set subject.", "step": 3},
            {"change": "x", "why": "y", "kind": "nonsense", "agent": "ghost", "step": 4}
        ],
        "went_well": ["Batch held", "", 3],
        "confidence": "HIGH"
    })
}

#[test]
fn a_run_review_keeps_real_steps_and_drops_the_rest() {
    let answer = messy_run_answer();
    let (r, issues) = run_review(&answer, &citable()).unwrap();
    assert_eq!(r.went_wrong.len(), 3);
    assert_eq!(r.went_wrong[0].steps, [110]);
    assert_eq!(r.went_wrong[0].severity, Severity::Goal);
    assert!(!r.went_wrong[0].unverified);
    assert!(r.went_wrong[1].steps.is_empty() && r.went_wrong[1].unverified);
    assert_eq!(r.went_wrong[2].severity, Severity::Goal);
    assert_eq!(r.suggestions[0].agent.as_deref(), Some("supply"));
    assert_eq!(r.suggestions[0].step, Some(3));
    assert_eq!(r.suggestions[1].kind, SuggestionKind::Process);
    assert_eq!(
        (r.suggestions[1].agent.as_ref(), r.suggestions[1].step),
        (None, None)
    );
    assert_eq!(r.went_well, ["Batch held"]);
    assert_eq!(r.confidence, "high");
    assert_run_review_issues(&issues);
}

fn assert_run_review_issues(issues: &Issues) {
    let said = issues.0.join("\n");
    for s in [
        "999",
        "998",
        "ghost",
        "dropped citation 4",
        "unknown severity",
        "without a title",
    ] {
        assert!(said.contains(s), "{s} not in {said}");
    }
}

#[test]
fn a_review_without_a_verdict_is_refused() {
    assert!(run_review(&json!({"went_wrong": []}), &citable()).is_err());
    assert!(run_review(&json!({"verdict": "  "}), &citable()).is_err());
}

#[test]
fn lists_are_bounded() {
    let many: Vec<Value> = (0..20)
        .map(|i| json!({"title": format!("t{i}"), "why": "w", "steps": [3], "severity": "goal"}))
        .collect();
    let (r, _) = run_review(&json!({"verdict": "v", "went_wrong": many}), &citable()).unwrap();
    assert_eq!(r.went_wrong.len(), MAX_WRONG);
}

#[test]
fn a_comparison_checks_each_side_against_its_own_branch() {
    let answer = json!({
        "headline": "B opened the deviation; A did not.",
        "differences": [
            {"title": "Deviation", "a": "failed at 110", "b": "created at 34",
             "why_it_matters": "a goal check", "steps_a": [110], "steps_b": [34, 110]},
            {"title": "Ghost", "a": "", "b": "", "why_it_matters": "none", "steps_a": [7], "steps_b": [8]}
        ],
        "which_is_better": "B",
        "why": "fewer failed checks"
    });
    let (r, issues) = compare_review(&answer, &citable(), Cites::Steps).unwrap();
    assert_eq!(r.which_is_better, Better::B);
    assert_eq!(r.differences[0].steps_a, [json!(110)]);
    assert_eq!(r.differences[0].steps_b, [json!(34)]);
    assert!(r.differences[1].unverified);
    assert_eq!(issues.0.len(), 3);
}

#[test]
fn a_suite_comparison_cites_runs() {
    let answer = json!({
        "headline": "h", "which_is_better": "sideways",
        "differences": [{"title": "t", "why_it_matters": "w", "steps_a": ["ra", "zz"], "steps_b": ["rb"]}]
    });
    let (r, issues) = compare_review(&answer, &citable(), Cites::Runs).unwrap();
    assert_eq!(r.differences[0].steps_a, [json!("ra")]);
    assert_eq!(r.differences[0].steps_b, [json!("rb")]);
    assert_eq!(r.which_is_better, Better::Mixed);
    assert_eq!(issues.0.len(), 2);
}
