//! The config of a `sculptured` instance: the program to run and the team the tools use.
use std::path::PathBuf;

use arena_part::PartError;
use serde::Deserialize;
use serde_json::Value;

/// The team the tools use unless the config names another.
pub const DEFAULT_TEAM: &str = "arena";
/// The program looked up on `PATH` when the config names none.
const DEFAULT_BINARY: &str = "sculptured";
/// Team names Sculptured keeps for its own API paths.
const RESERVED: [&str; 3] = ["import", "import-file", "check-file"];

#[derive(Debug, Clone)]
pub struct Config {
    pub binary: PathBuf,
    /// The team's name, which is also its id in Sculptured.
    pub team: String,
}

/// The config as given: every field may be left out.
#[derive(Default, Deserialize)]
struct Raw {
    #[serde(default)]
    binary: Option<PathBuf>,
    #[serde(default)]
    team: Option<String>,
}

impl Config {
    pub fn parse(v: &Value) -> Result<Config, PartError> {
        let raw: Raw = match v {
            Value::Null => Raw::default(),
            v => serde_json::from_value(v.clone())
                .map_err(|e| PartError::invalid(format!("config: {e}")))?,
        };
        let binary = raw.binary.or_else(default_binary).ok_or_else(|| {
            PartError::invalid("config: binary is required (no sculptured on PATH)")
        })?;
        if !binary.is_file() {
            return Err(PartError::invalid(format!(
                "binary {binary:?} is not a file"
            )));
        }
        let team = raw.team.unwrap_or_else(|| DEFAULT_TEAM.into());
        if !valid_team(&team) {
            return Err(PartError::invalid(format!(
                "team {team:?} must be 1-40 of a-z and 0-9, with single dashes inside"
            )));
        }
        Ok(Config { binary, team })
    }
}

/// The sculptured program a config without `binary` runs: the first `sculptured` file on `PATH`
/// (None when there is none).
pub fn default_binary() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(DEFAULT_BINARY))
        .find(|f| f.is_file())
}

/// A name that Sculptured keeps as the team's id: lowercase letters, digits and `-`.
fn valid_team(t: &str) -> bool {
    let ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
    (1..=40).contains(&t.len())
        && t.bytes().all(ok)
        && !t.starts_with('-')
        && !t.ends_with('-')
        && !t.contains("--")
        && !RESERVED.contains(&t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn with_binary(team: Value) -> Value {
        json!({"binary": "/bin/sh", "team": team})
    }

    #[test]
    fn team_names() {
        assert_eq!(
            Config::parse(&json!({"binary": "/bin/sh"})).unwrap().team,
            "arena"
        );
        assert_eq!(
            Config::parse(&with_binary(json!("buying-2"))).unwrap().team,
            "buying-2"
        );
        for bad in [
            "",
            "Arena",
            "a b",
            "-a",
            "a-",
            "a--b",
            "import",
            &"x".repeat(41),
        ] {
            assert!(Config::parse(&with_binary(json!(bad))).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn binary_must_exist() {
        let e = Config::parse(&json!({"binary": "/nope/sculptured"})).unwrap_err();
        assert_eq!(e.code, -32602);
        assert!(Config::parse(&json!({"binary": "/bin/sh", "extra": 1})).is_ok());
    }
}
