//! What the benchmark says about itself, with no service running.
use arena_part::proto::*;
use arena_part::Benchmark;
use arena_pharma::Pharma;
use serde_json::Map;

#[tokio::test]
async fn it_describes_five_scenarios_over_five_parts() {
    let d = Pharma::default().describe().await.unwrap();
    let parts: Vec<&str> = d.world.iter().map(|p| p.part.as_str()).collect();
    assert_eq!(parts, ["erp", "lims", "logistics", "approvals", "crm"]);
    assert!(d.scored);
    assert_eq!(d.tasks.len(), 5);
    assert_eq!(d.tools.len(), 4);
    assert!(d.tools.iter().all(|t| t.effect == Effect::Read));
    assert_eq!(d.team.unwrap().agents.len(), 6);
    let single = Pharma {
        team: "single".into(),
    }
    .describe()
    .await
    .unwrap();
    assert_eq!(single.team.unwrap().agents.len(), 1);
}

#[tokio::test]
async fn scoring_knows_its_tasks_and_an_empty_world_fails_the_goals() {
    let p = Pharma::default();
    let unknown = ScoreParams {
        task: "nope".into(),
        docs: Map::new(),
    };
    assert!(p.score(unknown).await.is_err());
    let empty = ScoreParams {
        task: "carrier-no-capacity".into(),
        docs: Map::new(),
    };
    let checks = p.score(empty).await.unwrap().checks;
    let missed = checks.iter().filter(|c| !c.passed && !c.safety).count();
    let unsafe_ = checks.iter().filter(|c| c.safety && !c.passed).count();
    assert_eq!((missed, unsafe_), (8, 0));
}

#[tokio::test]
async fn an_unknown_tool_is_the_erps_error() {
    let call = BenchCallTool {
        name: "nope".into(),
        args: Default::default(),
        connections: Map::new(),
    };
    let out = Pharma::default().call_tool(call).await.unwrap();
    assert_eq!(out.part, "erp");
    assert!(out.error.unwrap().contains("no tool nope"));
}

#[tokio::test]
async fn it_offers_the_demos_disruptions_and_a_view() {
    let d = Pharma::default().describe().await.unwrap();
    let o = d.scenario_options.unwrap();
    assert_eq!(o.base.len(), 5);
    let ids: Vec<&str> = o.disruptions.iter().map(|x| x.id.as_str()).collect();
    let logistics = ["courier-capacity", "lane-unavailable", "transit-delay"];
    let others = ["first-reallocation-rejected", "retest-fails"];
    assert_eq!(ids, [&logistics[..], &others[..]].concat());
    assert!(
        o.disruptions.iter().all(|x| !x.default),
        "nothing is preselected"
    );
    for x in &o.disruptions {
        assert!(
            d.world.iter().any(|p| p.part == x.part),
            "{} names a part",
            x.id
        );
        assert_eq!(x.template["id"], x.id.as_str());
        assert!(o.setup.contains_key(&x.part), "{} has a setup call", x.id);
    }
    assert_eq!(o.setup["logistics"].tool, "logistics_scenario");
    assert_eq!(d.view.unwrap().docs, ["lims", "logistics", "approvals"]);
    let p = Pharma::default();
    let quiet = p.view(ViewParams {
        connections: Map::new(),
        docs: Map::new(),
    });
    assert!(quiet.await.is_err(), "no ERP connection, no view");
}
