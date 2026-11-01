//! Names: a site and its database, and the keys of documents.

/// A site and the MariaDB database (and account) behind it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Site {
    /// The site's name, which requests name in their `Host` header.
    pub name: String,
    pub db: String,
}

/// Instance databases start with this.
pub const INSTANCE_PREFIX: &str = "eb_";
/// Scratch databases, for reading saves, start with this.
pub const VIEW_PREFIX: &str = "ev_";
/// The base site's database, while the base is being made (no instance or view prefix).
pub const BASE_DB: &str = "erpnext_base";
/// The base site, while it is being made (with `-`, so no instance's site).
pub const BASE_SITE: &str = "arena-base.localhost";

impl Site {
    /// The site of instance `id` (an id is `[a-z0-9_]{1,63}`): `<id>.localhost` on `eb_<id>`.
    /// `None` when the database name would pass MariaDB's 64 characters.
    pub fn instance(id: &str) -> Option<Site> {
        let db = format!("{INSTANCE_PREFIX}{id}");
        (db.len() <= 64).then(|| Site {
            name: format!("{id}.localhost"),
            db,
        })
    }

    /// The `n`th scratch site of this process. Its name has `-`, which no instance id has.
    pub fn view(n: u64) -> Site {
        let pid = std::process::id();
        Site {
            name: format!("view-{pid}-{n}.localhost"),
            db: format!("{VIEW_PREFIX}{pid}_{n}"),
        }
    }

    /// The base site while it is made.
    pub fn base() -> Site {
        Site {
            name: BASE_SITE.into(),
            db: BASE_DB.into(),
        }
    }
}

/// The key of a document: `<Doctype>/<name>`.
pub fn key(doctype: &str, name: &str) -> String {
    format!("{doctype}/{name}")
}

/// A key's doctype and name (a doctype has no `/`; a name may).
pub fn parse_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('/')
        .filter(|(d, n)| !d.is_empty() && !n.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_of_instances_and_views_never_clash() {
        let s = Site::instance("chk_a").unwrap();
        assert_eq!(
            (s.name.as_str(), s.db.as_str()),
            ("chk_a.localhost", "eb_chk_a")
        );
        assert!(Site::instance(&"a".repeat(61)).is_some());
        assert!(Site::instance(&"a".repeat(62)).is_none());
        let v = Site::view(3);
        assert!(v.name.starts_with("view-") && v.name.ends_with("-3.localhost"));
        assert!(v.db.starts_with(VIEW_PREFIX));
        let b = Site::base();
        assert!(!b.db.starts_with(VIEW_PREFIX) && !b.db.starts_with(INSTANCE_PREFIX));
        assert!(b.name.contains('-') && !b.name.starts_with("view-"));
    }

    #[test]
    fn keys_split_at_the_first_slash() {
        assert_eq!(key("Sales Order", "SO-1"), "Sales Order/SO-1");
        assert_eq!(parse_key("Item/A/B"), Some(("Item", "A/B")));
        assert_eq!(parse_key("Item"), None);
        assert_eq!(parse_key("/x"), None);
        assert_eq!(parse_key("Item/"), None);
    }
}
