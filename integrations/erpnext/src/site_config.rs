//! The JSON config files of the bench and of each site.
use arena_part::helpers::mariadb::Login;
use serde_json::{json, Map, Value};

/// `sites/apps.txt`: the apps the image has.
pub const APPS_TXT: &str = "frappe\nerpnext\n";

/// The database keys of a site's config, which differ between sites.
const DB_KEYS: [&str; 3] = ["db_name", "db_user", "db_password"];

/// `sites/common_site_config.json`: the database and Redis, by their names on the project's
/// network.
pub fn common_config() -> Value {
    json!({
        "db_host": "db",
        "db_port": 3306,
        "redis_cache": "redis://redis-cache:6379",
        "redis_queue": "redis://redis-queue:6379",
        "redis_socketio": "redis://redis-queue:6379",
        "socketio_port": 9000,
    })
}

/// What every site made from the base shares: the base site's config without its database
/// keys (it keeps `encryption_key`, which the stored API secret needs).
pub fn shared_part(site_config: &Value) -> Map<String, Value> {
    let mut out = site_config.as_object().cloned().unwrap_or_default();
    for k in DB_KEYS {
        out.remove(k);
    }
    out
}

/// A site's config: the shared part, its own database and account, and the scheduler paused,
/// so the site changes only through tool calls.
pub fn site_config(shared: &Map<String, Value>, db: &str, login: &Login) -> Value {
    let mut out = shared.clone();
    out.insert("db_name".into(), db.into());
    out.insert("db_user".into(), login.user.as_str().into());
    out.insert("db_password".into(), login.password.as_str().into());
    out.insert("db_type".into(), "mariadb".into());
    out.insert("pause_scheduler".into(), 1.into());
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_configs_share_all_but_the_database() {
        let base = json!({"db_name": "erpnext_base", "db_user": "erpnext_base",
            "db_password": "pw", "db_type": "mariadb", "encryption_key": "k=",
            "installed_apps": ["frappe", "erpnext"]});
        let shared = shared_part(&base);
        assert!(!shared.contains_key("db_password") && !shared.contains_key("db_name"));
        let login = Login {
            user: "eb_i1".into(),
            password: "secret".into(),
        };
        let c = site_config(&shared, "eb_i1", &login);
        assert_eq!(c["db_name"], "eb_i1");
        assert_eq!(c["db_user"], "eb_i1");
        assert_eq!(c["db_password"], "secret");
        assert_eq!(c["encryption_key"], "k=");
        assert_eq!(c["pause_scheduler"], 1);
        assert_eq!(c["installed_apps"], json!(["frappe", "erpnext"]));
        assert_eq!(common_config()["db_host"], "db");
    }
}
