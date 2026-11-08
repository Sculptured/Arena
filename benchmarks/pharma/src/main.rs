use arena_pharma::Pharma;

/// The value after `name` on the command line.
fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip_while(|a| a != name);
    args.nth(1)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let team = arg("--team").unwrap_or_else(|| "single".into());
    anyhow::ensure!(
        arena_pharma::team::DEPLOYMENTS.contains(&team.as_str()),
        "--team must be single, dispatch or team, not {team}"
    );
    arena_part::serve_benchmark(Pharma { team }).await
}
