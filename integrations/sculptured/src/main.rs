#[tokio::main]
async fn main() -> anyhow::Result<()> {
    arena_part::serve(arena_sculptured::Sculptured::from_env()).await
}
