#[tokio::main]
async fn main() -> anyhow::Result<()> {
    arena_part::serve(arena_postgres::Postgres::from_env()).await
}
