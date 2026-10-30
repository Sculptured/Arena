use arenad::store::Store;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("ARENA_PG_URL")
        .unwrap_or_else(|_| "postgres://arena:arena@127.0.0.1:5544/postgres".to_string());
    let admin = world::pg::Admin::connect(&url).await?;
    let _store = Store::open(&admin).await?;
    println!("arenad: store ready");
    Ok(())
}
