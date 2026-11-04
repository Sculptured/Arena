use arena_factory::{Backend, Factory};

/// The value after `name` on the command line.
fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip_while(|a| a != name);
    args.nth(1)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let backend = match arg("--backend").as_deref() {
        None | Some("standin") => Backend::Standin,
        Some("isaac") => Backend::Isaac,
        Some(other) => anyhow::bail!("--backend must be standin or isaac, not {other}"),
    };
    let data = arg("--data").unwrap_or_else(|| arena_factory::DEFAULT_DATA.to_string());
    arena_part::serve_benchmark(Factory::new(backend, data.into())).await
}
