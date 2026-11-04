//! What the benchmark can do without the factory example, which is not public yet.
use arena_factory::{Backend, Factory, TASK};
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
    assert_eq!(parts, ["erp", "hub", "plant", "approvals"]);
    assert!(d.scored && d.tools.is_empty());
    assert_eq!(d.tasks[0].id, TASK);
    assert_eq!(d.links[0].to, "plant");
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
    assert_eq!(d.world[2].config, json!({}));
    let config = json!({"record": "run.mp4", "camera": "auto"});
    let f = factory("/no/such/file").with_plant_config(config.clone());
    let d = f.describe().await.unwrap();
    assert_eq!(
        (d.world[2].part.as_str(), &d.world[2].config),
        ("plant", &config)
    );
}
