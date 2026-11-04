use anyhow::Context;
use arena_factory::{Backend, Factory};

/// The value after `name` on the command line.
fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip_while(|a| a != name);
    args.nth(1)
}

/// `--plant-config <JSON object>`: the plant part's config in the world (default `{}`).
fn plant_config() -> anyhow::Result<serde_json::Value> {
    let Some(text) = arg("--plant-config") else {
        return Ok(serde_json::json!({}));
    };
    let v: serde_json::Value = serde_json::from_str(&text).context("--plant-config is not JSON")?;
    anyhow::ensure!(v.is_object(), "--plant-config must be a JSON object");
    Ok(v)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let backend = match arg("--backend").as_deref() {
        None | Some("standin") => Backend::Standin,
        Some("isaac") => Backend::Isaac,
        Some(other) => anyhow::bail!("--backend must be standin or isaac, not {other}"),
    };
    let data = arg("--data").unwrap_or_else(|| arena_factory::DEFAULT_DATA.to_string());
    let factory = Factory::new(backend, data.into()).with_plant_config(plant_config()?);
    arena_part::serve_benchmark(factory).await
}
