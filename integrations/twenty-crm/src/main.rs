#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let p = arena_twenty_crm::TwentyCrm::from_env()?;
    arena_part::serve(p).await
}
