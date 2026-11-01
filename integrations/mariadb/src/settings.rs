//! An instance's settings: the world config's keys, else env `ARENA_MARIADB_*`, else defaults.
use arena_part::helpers::mariadb::{valid_docker_name, ServerConfig};
use arena_part::PartError;
use serde_json::Value;

/// Restores read at most this many binlog bytes before a save writes a dump instead.
pub const DEFAULT_DUMP_AFTER: u64 = 64 << 20;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub server: ServerConfig,
    pub init_sql: Option<String>,
    pub dump_after_bytes: u64,
}

/// Config key, env var.
const CONTAINER: (&str, &str) = ("container", "ARENA_MARIADB_CONTAINER");
const PORT: (&str, &str) = ("port", "ARENA_MARIADB_PORT");
const VOLUME: (&str, &str) = ("volume", "ARENA_MARIADB_VOLUME");
const PASSWORD: (&str, &str) = ("root_password", "ARENA_MARIADB_PASSWORD");
const DUMP_AFTER: (&str, &str) = ("dump_after_bytes", "ARENA_MARIADB_DUMP_AFTER_BYTES");

/// The value of a setting as text: the config key's, else the env var's.
fn lookup(config: &Value, (key, var): (&str, &str)) -> Option<String> {
    match config.get(key) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => std::env::var(var).ok(),
    }
}

fn text(config: &Value, setting: (&str, &str), default: String) -> String {
    lookup(config, setting).unwrap_or(default)
}

fn number<T: std::str::FromStr>(
    config: &Value,
    setting: (&str, &str),
    default: T,
) -> Result<T, PartError> {
    match lookup(config, setting) {
        None => Ok(default),
        Some(s) => s.parse().map_err(|_| {
            PartError::invalid(format!("{} must be a whole number, got {s}", setting.0))
        }),
    }
}

impl Settings {
    pub fn from_config(config: &Value) -> Result<Settings, PartError> {
        let d = ServerConfig::default();
        let server = ServerConfig {
            container: text(config, CONTAINER, d.container),
            port: number(config, PORT, d.port)?,
            volume: text(config, VOLUME, d.volume),
            root_password: text(config, PASSWORD, d.root_password),
            flags: d.flags,
        };
        if !valid_docker_name(&server.container) || !valid_docker_name(&server.volume) {
            return Err(PartError::invalid(
                "container and volume must match [A-Za-z0-9][A-Za-z0-9_.-]{0,62}",
            ));
        }
        Ok(Settings {
            server,
            init_sql: config
                .get("init_sql")
                .and_then(Value::as_str)
                .map(String::from),
            dump_after_bytes: number(config, DUMP_AFTER, DEFAULT_DUMP_AFTER)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn config_keys_win_and_bad_values_are_invalid() {
        let s = Settings::from_config(&json!({
            "container": "arena-mariadb-x", "port": 3399, "volume": "v1",
            "dump_after_bytes": 10, "init_sql": "CREATE TABLE a(id int)"
        }))
        .unwrap();
        assert_eq!(s.server.container, "arena-mariadb-x");
        assert_eq!(s.server.port, 3399);
        assert_eq!(s.server.volume, "v1");
        assert_eq!(s.dump_after_bytes, 10);
        assert_eq!(s.init_sql.as_deref(), Some("CREATE TABLE a(id int)"));
        let e = Settings::from_config(&json!({"port": 70000})).unwrap_err();
        assert_eq!(e.code, -32602);
        let e = Settings::from_config(&json!({"container": "a b"})).unwrap_err();
        assert_eq!(e.code, -32602);
    }
}
