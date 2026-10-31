use arena_retail::{data_dir_from_env, Retail};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let retail = Retail::new(&data_dir_from_env())?;
    arena_part::serve_benchmark(retail).await
}
