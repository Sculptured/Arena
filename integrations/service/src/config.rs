//! The config of a `service` instance.
use std::collections::BTreeMap;
use std::path::PathBuf;

use arena_part::helpers::compose::{valid_volume, DEFAULT_HELPER};
use arena_part::PartError;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub compose_file: PathBuf,
    /// The compose file's volumes that a save holds.
    pub volumes: Vec<String>,
    /// The image whose `tar` copies volumes (default `redis:6.2-alpine`).
    #[serde(default)]
    pub helper_image: Option<String>,
    /// Extra environment for `docker compose` (variables the compose file uses).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl Config {
    pub fn parse(v: &Value) -> Result<Config, PartError> {
        let c: Config = serde_json::from_value(v.clone())
            .map_err(|e| PartError::invalid(format!("config: {e}")))?;
        if !c.compose_file.is_file() {
            return Err(PartError::invalid(format!(
                "compose_file {:?} is not a file",
                c.compose_file
            )));
        }
        match c.volumes.iter().find(|v| !valid_volume(v)) {
            Some(v) => Err(PartError::invalid(format!("{v:?} is not a volume name"))),
            None => Ok(c),
        }
    }

    pub fn helper(&self) -> &str {
        self.helper_image.as_deref().unwrap_or(DEFAULT_HELPER)
    }
}
