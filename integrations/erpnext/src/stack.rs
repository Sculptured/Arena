//! The ERPNext install: the compose project, the MariaDB server on its network as `db`, and
//! commands run in its containers.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use arena_part::helpers::blocking;
use arena_part::helpers::compose::Project;
use arena_part::helpers::mariadb::Server;

use crate::frappe::parse_versions;
use crate::rest::Rest;
use crate::settings::StackConfig;
use crate::sites::SitesDir;

/// How long the stack may take to answer HTTP after `up`.
const READY_TIMEOUT: Duration = Duration::from_secs(180);

/// Deletes the Redis cache keys matching `ARGV[1]` (Frappe prefixes a site's keys with its
/// database name and `|`).
const CLEAR_SCRIPT: &str =
    "for _, k in ipairs(redis.call('KEYS', ARGV[1])) do redis.call('DEL', k) end return 0";

pub struct Stack {
    pub cfg: StackConfig,
    pub server: Server,
    pub sites: SitesDir,
    pub rest: Rest,
    project: Project,
}

impl Stack {
    /// The stack of `cfg` with its sites in `sites` (an absolute path). Nothing is started.
    pub fn new(cfg: StackConfig, server: Server, sites: PathBuf) -> Result<Stack> {
        let env = BTreeMap::from([
            (
                "ARENA_ERPNEXT_SITES".to_string(),
                sites.to_string_lossy().into_owned(),
            ),
            ("ARENA_ERPNEXT_PORT".to_string(), cfg.port.to_string()),
            ("ARENA_ERPNEXT_IMAGE".to_string(), cfg.image.clone()),
        ]);
        let project = Project::new(&cfg.project, &cfg.compose_file, &env)?;
        Ok(Stack {
            rest: Rest::new(cfg.port)?,
            server,
            sites: SitesDir::new(sites),
            project,
            cfg,
        })
    }

    /// Starts the server and the project (each only when not running), joins them, and waits
    /// until ERPNext answers.
    pub async fn up(&self) -> Result<()> {
        self.server.ensure().await?;
        self.sites.prepare()?;
        let project = self.project.clone();
        let container = self.server.config().container.clone();
        blocking(move || {
            project.up()?;
            project.connect(&container, "db")
        })
        .await?;
        self.wait_ready().await
    }

    async fn wait_ready(&self) -> Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        while !self.rest.answers().await {
            if Instant::now() > deadline {
                bail!("ERPNext did not answer on 127.0.0.1:{}", self.cfg.port);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Ok(())
    }

    /// Runs `bench <args>` in the backend container; its output.
    pub async fn bench(&self, args: &[&str]) -> Result<String> {
        let project = self.project.clone();
        let mut all = vec!["bench".to_string()];
        all.extend(args.iter().map(|a| a.to_string()));
        blocking(move || {
            let args: Vec<&str> = all.iter().map(String::as_str).collect();
            project.exec("backend", &args)
        })
        .await
    }

    /// Drops the Redis cache of the site on database `db` (left from an earlier site of that
    /// name).
    pub async fn clear_cache(&self, db: &str) -> Result<()> {
        let project = self.project.clone();
        let pattern = format!("{db}|*");
        blocking(move || {
            let args = ["redis-cli", "EVAL", CLEAR_SCRIPT, "0", &pattern];
            project.exec("redis-cache", &args).map(drop)
        })
        .await
    }
}

/// The app versions in `image` (`bench version` in a throwaway container: no network, never
/// pulled).
pub async fn image_versions(image: String) -> Result<BTreeMap<String, String>> {
    blocking(move || {
        let out = Command::new("docker")
            .args(["run", "--rm", "--pull", "never", "--network", "none"])
            .args(["--entrypoint", "bench", &image, "version"])
            .output()
            .context("cannot run docker")?;
        if !out.status.success() {
            bail!(
                "bench version failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let versions = parse_versions(&String::from_utf8_lossy(&out.stdout));
        let need: BTreeSet<&str> = ["erpnext", "frappe"].into();
        if !need.iter().all(|a| versions.contains_key(*a)) {
            bail!("bench version did not list frappe and erpnext");
        }
        Ok(versions)
    })
    .await
}
