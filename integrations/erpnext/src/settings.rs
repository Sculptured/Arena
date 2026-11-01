//! Settings: the ERPNext install (one per program: env `ARENA_ERPNEXT_*` or defaults, which the
//! config of the first start may override) and an instance's settings (from the world config).
use std::path::PathBuf;

use arena_part::helpers::compose::valid_image;
use arena_part::helpers::mariadb::{valid_docker_name, ServerConfig};
use arena_part::PartError;
use serde_json::Value;

/// Restores read at most this many binlog bytes before a save writes a dump instead.
pub const DEFAULT_DUMP_AFTER: u64 = 64 << 20;

/// The doctypes `diff` compares unless the config names others.
pub const DEFAULT_DIFF_DOCTYPES: [&str; 7] = [
    "Item",
    "Supplier",
    "Customer",
    "Purchase Order",
    "Sales Order",
    "Work Order",
    "Journal Entry",
];

/// Frappe's settings for MariaDB, as in the factory's compose file.
const SERVER_FLAGS: [&str; 3] = [
    "--character-set-server=utf8mb4",
    "--collation-server=utf8mb4_unicode_ci",
    "--skip-character-set-client-handshake",
];

/// The ERPNext install: its compose project and the MariaDB server its sites use.
#[derive(Debug, Clone, PartialEq)]
pub struct StackConfig {
    pub server: ServerConfig,
    pub project: String,
    pub port: u16,
    pub image: String,
    pub compose_file: PathBuf,
}

impl Default for StackConfig {
    fn default() -> Self {
        StackConfig {
            server: ServerConfig {
                container: "arena-erpnext-db".into(),
                port: 3308,
                volume: "arena_erpnext_db".into(),
                root_password: "arena".into(),
                flags: SERVER_FLAGS.map(String::from).into(),
            },
            project: "arena_erpnext".into(),
            port: 8180,
            image: "frappe/erpnext:v16.53.0".into(),
            compose_file: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("compose.yml"),
        }
    }
}

/// `var` parsed, or `default` when it is not set.
fn env_or<T: std::str::FromStr>(var: &str, default: T) -> Result<T, PartError> {
    match std::env::var(var) {
        Err(_) => Ok(default),
        Ok(s) => s
            .parse()
            .map_err(|_| PartError::invalid(format!("{var} has a bad value: {s}"))),
    }
}

impl StackConfig {
    /// The defaults, each overridden by its env var when set.
    pub fn from_env() -> Result<StackConfig, PartError> {
        let d = StackConfig::default();
        let s = &d.server;
        let c = StackConfig {
            server: ServerConfig {
                container: env_or("ARENA_ERPNEXT_DB_CONTAINER", s.container.clone())?,
                port: env_or("ARENA_ERPNEXT_DB_PORT", s.port)?,
                volume: env_or("ARENA_ERPNEXT_DB_VOLUME", s.volume.clone())?,
                root_password: env_or("ARENA_ERPNEXT_DB_PASSWORD", s.root_password.clone())?,
                flags: s.flags.clone(),
            },
            project: env_or("ARENA_ERPNEXT_PROJECT", d.project.clone())?,
            port: env_or("ARENA_ERPNEXT_PORT", d.port)?,
            image: env_or("ARENA_ERPNEXT_IMAGE", d.image.clone())?,
            compose_file: env_or("ARENA_ERPNEXT_COMPOSE", d.compose_file.clone())?,
        };
        c.check()?;
        Ok(c)
    }

    /// These settings with the config's stack keys (`db_container`, `db_port`, `db_volume`,
    /// `db_root_password`, `project`, `http_port`) put over them.
    pub fn with_config(&self, config: &Value) -> Result<StackConfig, PartError> {
        let mut c = self.clone();
        let s = &mut c.server;
        set_text(config, "db_container", &mut s.container)?;
        set_text(config, "db_volume", &mut s.volume)?;
        set_text(config, "db_root_password", &mut s.root_password)?;
        set_text(config, "project", &mut c.project)?;
        set_port(config, "db_port", &mut c.server.port)?;
        set_port(config, "http_port", &mut c.port)?;
        c.check()?;
        Ok(c)
    }

    fn check(&self) -> Result<(), PartError> {
        let s = &self.server;
        if !valid_docker_name(&s.container) || !valid_docker_name(&s.volume) {
            return Err(PartError::invalid("bad MariaDB container or volume name"));
        }
        let project_ok = !self.project.is_empty()
            && self
                .project
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !project_ok {
            return Err(PartError::invalid("project must be [a-z0-9_]+"));
        }
        if !valid_image(&self.image) {
            return Err(PartError::invalid(format!("bad image {:?}", self.image)));
        }
        Ok(())
    }

    /// The image's tag (`v16.53.0`).
    pub fn image_tag(&self) -> &str {
        self.image.rsplit(':').next().unwrap_or(&self.image)
    }
}

/// Sets `slot` to the config's text `key`, when it has one.
fn set_text(config: &Value, key: &str, slot: &mut String) -> Result<(), PartError> {
    match config.get(key) {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(s)) => {
            slot.clone_from(s);
            Ok(())
        }
        Some(_) => Err(PartError::invalid(format!("{key} must be text"))),
    }
}

/// Sets `slot` to the config's port `key`, when it has one.
fn set_port(config: &Value, key: &str, slot: &mut u16) -> Result<(), PartError> {
    match config.get(key) {
        None | Some(Value::Null) => Ok(()),
        Some(v) => {
            let port = v.as_u64().and_then(|n| u16::try_from(n).ok());
            *slot = port.ok_or_else(|| PartError::invalid(format!("{key} must be a port")))?;
            Ok(())
        }
    }
}

/// An instance's settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub diff_doctypes: Vec<String>,
    pub dump_after_bytes: u64,
}

impl Settings {
    pub fn from_config(config: &Value) -> Result<Settings, PartError> {
        let diff_doctypes = match config.get("diff_doctypes") {
            None | Some(Value::Null) => DEFAULT_DIFF_DOCTYPES.map(String::from).into(),
            Some(v) => doctype_list(v)?,
        };
        let dump_after_bytes = match config.get("dump_after_bytes") {
            None | Some(Value::Null) => DEFAULT_DUMP_AFTER,
            Some(v) => v
                .as_u64()
                .ok_or_else(|| PartError::invalid("dump_after_bytes must be a whole number"))?,
        };
        Ok(Settings {
            diff_doctypes,
            dump_after_bytes,
        })
    }
}

/// A list of doctype names (not empty strings, no `/`).
fn doctype_list(v: &Value) -> Result<Vec<String>, PartError> {
    let bad = || PartError::invalid("diff_doctypes must be a list of doctype names");
    let items = v.as_array().ok_or_else(bad)?;
    items
        .iter()
        .map(|d| match d.as_str() {
            Some(s) if !s.is_empty() && !s.contains('/') => Ok(s.to_string()),
            _ => Err(bad()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn instance_settings_default_and_validate() {
        let s = Settings::from_config(&json!({})).unwrap();
        assert_eq!(s.diff_doctypes.len(), 7);
        assert_eq!(s.diff_doctypes[3], "Purchase Order");
        assert_eq!(s.dump_after_bytes, DEFAULT_DUMP_AFTER);
        let s = Settings::from_config(&json!({"diff_doctypes": ["Item"], "dump_after_bytes": 9}));
        assert_eq!(s.unwrap().diff_doctypes, ["Item"]);
        for bad in [
            json!({"diff_doctypes": "Item"}),
            json!({"diff_doctypes": ["a/b"]}),
            json!({"diff_doctypes": [""]}),
            json!({"dump_after_bytes": -1}),
        ] {
            assert_eq!(
                Settings::from_config(&bad).unwrap_err().code,
                -32602,
                "{bad}"
            );
        }
    }

    #[test]
    fn stack_defaults_are_the_arena_install() {
        let d = StackConfig::default();
        assert_eq!((d.project.as_str(), d.port), ("arena_erpnext", 8180));
        assert_eq!(d.image_tag(), "v16.53.0");
        assert_eq!(d.server.container, "arena-erpnext-db");
        assert!(d
            .server
            .flags
            .iter()
            .any(|f| f.contains("utf8mb4_unicode_ci")));
        assert!(d.compose_file.ends_with("integrations/erpnext/compose.yml"));
        assert!(d.check().is_ok());
        let bad = StackConfig {
            image: "-x".into(),
            ..d
        };
        assert!(bad.check().is_err());
    }

    #[test]
    fn config_keys_override_the_stack() {
        let d = StackConfig::default();
        let c = d
            .with_config(
                &json!({"db_container": "db-t", "db_port": 3399, "db_volume": "v-t",
                                 "project": "arena_t", "http_port": 8181, "diff_doctypes": []}),
            )
            .unwrap();
        assert_eq!((c.server.container.as_str(), c.server.port), ("db-t", 3399));
        assert_eq!(
            (c.server.volume.as_str(), c.project.as_str(), c.port),
            ("v-t", "arena_t", 8181)
        );
        assert_eq!(d.with_config(&json!({})).unwrap(), d);
        for bad in [
            json!({"db_port": 70000}),
            json!({"db_container": "a b"}),
            json!({"project": "Arena"}),
            json!({"db_volume": 3}),
        ] {
            assert_eq!(d.with_config(&bad).unwrap_err().code, -32602, "{bad}");
        }
    }
}
