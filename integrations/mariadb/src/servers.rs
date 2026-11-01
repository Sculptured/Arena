//! The servers this program has used, by container name.
use std::collections::HashMap;
use std::sync::Mutex;

use arena_part::helpers::mariadb::{Server, ServerConfig, ServerRef};
use arena_part::PartError;
use serde_json::json;

use crate::settings::Settings;

#[derive(Default)]
pub struct Servers {
    known: Mutex<HashMap<String, Server>>,
    /// One `ensure` at a time, so two starts never both create the container.
    ensuring: tokio::sync::Mutex<()>,
}

impl Servers {
    /// The server of `config`, started if it is not running.
    pub async fn ensure(&self, config: ServerConfig) -> Result<Server, PartError> {
        let _one = self.ensuring.lock().await;
        let server = Server::new(config).map_err(|e| PartError::invalid(format!("{e:#}")))?;
        server.ensure().await?;
        let name = server.config().container.clone();
        self.known
            .lock()
            .expect("servers lock")
            .insert(name, server.clone());
        Ok(server)
    }

    /// The server a manifest names: the one this program used under that container name, else a
    /// handle with the configured password (env or default). Nothing is started.
    pub fn for_ref(&self, r: &ServerRef) -> Result<Server, PartError> {
        if let Some(s) = self.known.lock().expect("servers lock").get(&r.container) {
            return Ok(s.clone());
        }
        let mut config = Settings::from_config(&json!({}))?.server;
        config.container.clone_from(&r.container);
        config.port = r.port;
        Server::new(config).map_err(PartError::internal_from)
    }

    /// Every server this program started or used.
    pub fn known(&self) -> Vec<Server> {
        self.known
            .lock()
            .expect("servers lock")
            .values()
            .cloned()
            .collect()
    }
}
