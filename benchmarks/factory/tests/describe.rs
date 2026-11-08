//! What the benchmark can do without the factory example, which is not public yet.
use arena_factory::{Backend, Factory, SHIFT_TASK, TASK};
use arena_part::proto::*;
use arena_part::Benchmark;
use serde_json::{json, Map};

fn factory(data: &str) -> Factory {
    Factory::new(Backend::Standin, data.into())
}

#[tokio::test]
async fn it_describes_itself_without_the_factory_example() {
    let d = factory("/no/such/file").describe().await.unwrap();
    let parts: Vec<&str> = d.world.iter().map(|p| p.part.as_str()).collect();
    assert_eq!(
        parts,
        [
            "erp",
            "hub",
            "plant",
            "approvals",
            "crm",
            "utility",
            "network",
            "roster"
        ]
    );
    assert!(d.scored && d.tools.is_empty());
    let tasks: Vec<&str> = d.tasks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(tasks, [TASK, SHIFT_TASK]);
    assert_eq!(d.tasks[1].setup[0].tool, "plant_scenario");
    assert_eq!(d.links[0].to, "plant");
    assert_eq!(d.world[4].integration, "twenty-crm");
    assert_eq!(d.version, "0.4.0");
    assert_eq!(d.scenario_options.unwrap().disruptions.len(), 4);
    assert_eq!(d.view.unwrap().docs.len(), 6);
    assert_eq!(d.team.unwrap().id, "incident");
}

#[tokio::test]
async fn seeding_and_scoring_without_it_say_where_it_should_be() {
    let f = factory("/no/such/file");
    let score = ScoreParams {
        task: TASK.into(),
        docs: Map::new(),
    };
    let e = f.score(score).await.unwrap_err();
    assert!(e.message.contains("/no/such/file"), "{}", e.message);
    let mut connections = Map::new();
    connections.insert("erp".to_string(), json!({}));
    let e = f.seed(SeedParams { connections }).await.unwrap_err();
    assert!(e.message.contains("not public yet"), "{}", e.message);
}

#[tokio::test]
async fn the_plant_config_reaches_the_plant() {
    let d = factory("/no/such/file").describe().await.unwrap();
    assert_eq!(d.world[2].config["cells"][0]["id"], "CELL-R1");
    let config = json!({"record": "run.mp4", "camera": "auto"});
    let f = factory("/no/such/file").with_plant_config(config.clone());
    let d = f.describe().await.unwrap();
    assert_eq!(d.world[2].part, "plant");
    assert_eq!(d.world[2].config["record"], "run.mp4");
    assert_eq!(d.world[2].config["power"]["meter"], "Plant 2 main meter");
    let shift = factory("/no/such/file").with_team("shift");
    assert_eq!(shift.describe().await.unwrap().team.unwrap().id, "shift");
}
