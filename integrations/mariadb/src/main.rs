#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut p = arena_mariadb::Mariadb::from_env();
    if std::env::args().any(|a| a == "--sql-tools") {
        p = p.with_sql_tools();
    }
    arena_part::serve(p).await
}
