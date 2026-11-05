//! Which server this program runs: from env, then the config of the first start. arenad runs
//! programs with a cleared environment, so the config is how a test names a server of its own.
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde_json::Value;

pub const DEFAULT_IMAGE: &str = "twentycrm/twenty:v2.51.0";
const DEFAULT_PROJECT: &str = "arena_twenty";
const DEFAULT_DB_PORT: u16 = 5546;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The database server's compose project; instances' projects start with it.
    pub project: String,
    pub db_port: u16,
    pub image: String,
    /// The folder holding `compose.yml` and `instance.yml`.
    pub files: PathBuf,
}

fn project_ok(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            project: DEFAULT_PROJECT.into(),
            db_port: DEFAULT_DB_PORT,
            image: DEFAULT_IMAGE.into(),
            files: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        }
    }
}

impl Settings {
    /// `ARENA_TWENTY_PROJECT`, `_DB_PORT`, `_IMAGE`, else the defaults.
    pub fn from_env() -> Result<Settings> {
        let mut s = Settings::default();
        if let Ok(project) = std::env::var("ARENA_TWENTY_PROJECT") {
            s.project = project;
        }
        if let Ok(port) = std::env::var("ARENA_TWENTY_DB_PORT") {
            s.db_port = port.parse().context("ARENA_TWENTY_DB_PORT")?;
        }
        if let Ok(image) = std::env::var("ARENA_TWENTY_IMAGE") {
            s.image = image;
        }
        s.check()?;
        Ok(s)
    }

    /// These settings with a start's config applied (`project`, `db_port`).
    pub fn with_config(&self, config: &Value) -> Result<Settings> {
        let mut s = self.clone();
        if let Some(project) = config["project"].as_str() {
            s.project = project.into();
        }
        if let Some(port) = config["db_port"].as_u64() {
            s.db_port = u16::try_from(port).context("db_port")?;
        }
        s.check()?;
        Ok(s)
    }

    /// Whether `config` names another server than these settings.
    pub fn conflicts_with(&self, config: &Value) -> bool {
        let project = config["project"]
            .as_str()
            .is_some_and(|p| p != self.project);
        let port = config["db_port"]
            .as_u64()
            .is_some_and(|p| p != u64::from(self.db_port));
        project || port
    }

    fn check(&self) -> Result<()> {
        if !project_ok(&self.project) {
            bail!("{:?} is not a project name", self.project);
        }
        Ok(())
    }

    /// The prefix of the Docker names of containers, volumes and the network.
    pub fn docker_name(&self) -> String {
        self.project.replace('_', "-")
    }

    pub fn admin_url(&self) -> String {
        format!("postgres://arena:arena@127.0.0.1:{}/postgres", self.db_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_config_overrides_and_bad_names_are_refused() {
        let s =
            Settings::default().with_config(&json!({"project": "arena_twenty_t1", "db_port": 7}));
        let s = s.unwrap();
        assert_eq!((s.project.as_str(), s.db_port), ("arena_twenty_t1", 7));
        assert_eq!(s.docker_name(), "arena-twenty-t1");
        assert!(s.admin_url().contains("127.0.0.1:7/"));
        assert!(Settings::default()
            .with_config(&json!({"project": "A b"}))
            .is_err());
        assert!(Settings::default()
            .with_config(&json!({"db_port": 70000}))
            .is_err());
    }

    #[test]
    fn it_pins_a_version() {
        assert!(Settings::default().image.contains(":v2."));
        let compose = include_str!("../instance.yml");
        assert!(
            compose.contains(DEFAULT_IMAGE),
            "instance.yml and DEFAULT_IMAGE must agree"
        );
    }
}
