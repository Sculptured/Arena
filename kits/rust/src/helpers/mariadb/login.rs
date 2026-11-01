//! MariaDB accounts: root, and the accounts limited to one database that instances and replays
//! use.
use std::io::{self, Read};

use crate::helpers::folder::hex;

use super::server::ServerConfig;

/// What a database account may do in its one database. No views, triggers, routines or events:
/// their `DEFINER` names an account, which would not survive a restore under another name.
pub const DB_GRANTS: &str = "SELECT, INSERT, UPDATE, DELETE, CREATE, DROP, ALTER, INDEX, \
                             CREATE TEMPORARY TABLES, LOCK TABLES";

/// What a replay account may do besides: replay `BINLOG` events and turn its own binlogging off.
pub const REPLAY_GRANTS: &str = "BINLOG REPLAY, BINLOG ADMIN";

/// A user name and password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub user: String,
    pub password: String,
}

impl Login {
    pub fn root(cfg: &ServerConfig) -> Login {
        Login {
            user: "root".into(),
            password: cfg.root_password.clone(),
        }
    }

    /// `user` with a new random password.
    pub fn new_random(user: &str) -> io::Result<Login> {
        let mut bytes = [0u8; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        Ok(Login {
            user: user.into(),
            password: hex(&bytes),
        })
    }
}

/// The account that replays saves into database `db`.
pub fn replay_user(db: &str) -> String {
    format!("rp_{db}")
}

/// `s` with every byte outside `[A-Za-z0-9-._~]` percent-encoded (for a URL's user info).
pub fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
