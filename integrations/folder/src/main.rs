#[tokio::main]
async fn main() -> anyhow::Result<()> {
    arena_part::serve(arena_folder::Folder::from_env()).await
}
