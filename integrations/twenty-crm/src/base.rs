//! The base every instance without a save starts from: a database `tb_base` with a workspace, no
//! demo records and an API key. Made once, on a server that runs the image's migrations, then that
//! server is removed. The sign-in file is written last; a base without one is made again.
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::rows;
use crate::stack::{bring_up, free_port, take_down, ServerSpec, Stack};
use crate::workspace::{self, Creds};

pub const BASE_DB: &str = "tb_base";
const BUILD_DB: &str = "tb_build";
/// Migrations on an empty database took 48 s on the machine this was written on.
const BOOT_TIMEOUT: Duration = Duration::from_secs(300);

fn creds_path(root: &Path) -> PathBuf {
    root.join(".base/api.json")
}

/// The base's sign-in, making the base if there is none.
pub async fn load_or_make(stack: &Stack, root: &Path) -> Result<Creds> {
    let path = creds_path(root);
    if stack.pg.exists(BASE_DB).await? {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return serde_json::from_str(&text).context("reading the base's sign-in");
        }
    }
    let creds = make(stack).await.context("making the base workspace")?;
    write_creds(&path, &creds)?;
    Ok(creds)
}

async fn make(stack: &Stack) -> Result<Creds> {
    for db in [BASE_DB, BUILD_DB] {
        stack.pg.drop(db).await?;
    }
    stack.pg.create_empty(BUILD_DB).await?;
    let creds = with_build_server(stack).await?;
    stack
        .pg
        .with_db(BUILD_DB, async |c| rows::purge_demo(c).await)
        .await?;
    stack.pg.create_from_exclusive(BASE_DB, BUILD_DB).await?;
    stack.pg.drop(BUILD_DB).await?;
    Ok(creds)
}

/// A server on the build database, for as long as the workspace takes; removed either way.
async fn with_build_server(stack: &Stack) -> Result<Creds> {
    let port = free_port()?;
    let spec = ServerSpec {
        instance: "base",
        db: BUILD_DB,
        port,
        migrate: true,
    };
    let project = stack.server(&spec)?;
    let made = async {
        bring_up(project.clone(), port, BOOT_TIMEOUT).await?;
        workspace::make(port).await
    }
    .await;
    take_down(project).await?;
    made
}

fn write_creds(path: &Path, creds: &Creds) -> Result<()> {
    let dir = path.parent().expect("the sign-in file is in .base");
    std::fs::create_dir_all(dir)?;
    let scratch = path.with_extension("tmp");
    std::fs::write(&scratch, serde_json::to_vec(creds)?)?;
    std::fs::rename(&scratch, path)?;
    Ok(())
}
