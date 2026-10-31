#[tokio::main]
async fn main() -> anyhow::Result<()> {
    arena_part::serve(arena_service::Service::from_env()).await
}
