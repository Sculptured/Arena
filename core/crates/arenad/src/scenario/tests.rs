use parts::proto::{ChosenDisruption, SetupTemplate};
use serde_json::json;

use super::*;

fn param(
    name: &str,
    kind: ParamKind,
    choices: Option<Vec<Value>>,
    default: Value,
) -> ScenarioParam {
    let bounded = kind == ParamKind::Int;
    ScenarioParam {
        name: name.into(),
        label: name.into(),
        kind,
        choices,
        min: bounded.then_some(0),
        max: bounded.then_some(5),
        default,
    }
}

fn option(id: &str, part: &str, params: Vec<ScenarioParam>, template: Value) -> DisruptionOption {
    DisruptionOption {
        id: id.into(),
        label: id.into(),
        description: "".into(),
        part: part.into(),
        default: false,
        params,
        template,
    }
}

fn options() -> ScenarioOptions {
    let carriers = Some(vec![json!("Tundra"), json!("Borealis")]);
    let params = vec![
        param("after", ParamKind::Int, None, json!(1)),
        param("carrier", ParamKind::Choice, carriers, json!("Tundra")),
    ];
    let full = json!({"id": "full-{carrier}", "when": {"after_calls": "{after}"},
                      "do": {"kind": "lane_capacity", "carrier": "{carrier}", "capacity": 0},
                      "text": "{carrier} is full after {after} quotes"});
    let fails = json!({"id": "fails", "when": {"after_calls": 0}, "do": {"kind": "result_fails"}, "text": "t"});
    let setup = SetupTemplate {
        tool: "lims_scenario".into(),
        args: json!({"name": "base"}),
    };
    ScenarioOptions {
        base: vec!["t1".into()],
        disruptions: vec![
            option("full", "logistics", params, full),
            option("fails", "lims", vec![], fails),
        ],
        setup: [("lims".to_string(), setup)].into_iter().collect(),
    }
}

fn req(chosen: &[(&str, Value)]) -> ScenarioRequest {
    let one = |(id, params): &(&str, Value)| ChosenDisruption {
        id: id.to_string(),
        params: params.as_object().unwrap().clone(),
    };
    ScenarioRequest {
        disruptions: chosen.iter().map(one).collect(),
    }
}

#[test]
fn a_template_takes_its_params_whole_or_as_text() {
    let s = expand(Some(&options()), &req(&[("full", json!({"after": 3}))])).unwrap();
    let e = &s.disruptions[0];
    assert_eq!((e.id.as_str(), e.part.as_str()), ("full", "logistics"));
    assert_eq!(
        e.params,
        *json!({"after": 3, "carrier": "Tundra"})
            .as_object()
            .unwrap()
    );
    assert_eq!(
        e.disruption,
        json!({"id": "full-Tundra", "when": {"after_calls": 3},
               "do": {"kind": "lane_capacity", "carrier": "Tundra", "capacity": 0},
               "text": "Tundra is full after 3 quotes"})
    );
    let untouched = fill(&json!({"n": 2, "s": "{nope}", "b": true}), &Map::new());
    assert_eq!(untouched, json!({"n": 2, "s": "{nope}", "b": true}));
}

#[test]
fn bad_choices_are_refused_with_why() {
    let o = options();
    let refused = |chosen: &[(&str, Value)]| expand(Some(&o), &req(chosen)).unwrap_err();
    assert!(refused(&[("nope", json!({}))]).contains("no disruption \"nope\""));
    assert!(refused(&[("full", json!({"after": 9}))]).contains("after must be from"));
    assert!(refused(&[("full", json!({"after": "1"}))]).contains("whole number"));
    assert!(refused(&[("full", json!({"carrier": "Zeta"}))]).contains("one of"));
    assert!(refused(&[("full", json!({"speed": 1}))]).contains("no param \"speed\""));
    assert!(refused(&[("full", json!({})), ("full", json!({}))]).contains("chosen twice"));
    let none = expand(None, &req(&[("full", json!({}))])).unwrap_err();
    assert!(none.contains("offers no disruptions"));
    assert!(expand(None, &req(&[])).unwrap().is_empty());
}

#[test]
fn disruptions_join_the_parts_setup_call_or_a_new_one() {
    let o = options();
    let s = expand(Some(&o), &req(&[("full", json!({})), ("fails", json!({}))])).unwrap();
    let task = vec![SetupCall {
        part: "logistics".into(),
        tool: "logistics_scenario".into(),
        args: json!({"name": "carrier-no-capacity"}),
    }];
    let calls = setup_calls(&task, &s, Some(&o)).unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].args["name"], "carrier-no-capacity");
    assert_eq!(calls[0].args["disruptions"][0]["id"], "full-Tundra");
    assert_eq!(
        (calls[1].part.as_str(), calls[1].tool.as_str()),
        ("lims", "lims_scenario")
    );
    assert_eq!(
        calls[1].args,
        json!({"name": "base", "disruptions": [s.disruptions[1].disruption]})
    );
    let bare = setup_calls(&[], &s, Some(&o)).unwrap_err();
    assert!(bare.contains("no setup call of the part logistics"));
    assert_eq!(
        setup_calls(&task, &Scenario::default(), None).unwrap(),
        task
    );
}
