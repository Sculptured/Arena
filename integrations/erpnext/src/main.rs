#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let p = arena_erpnext::Erpnext::from_env()?;
    arena_part::serve(p).await
}
