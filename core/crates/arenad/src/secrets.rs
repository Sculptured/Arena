//! Secrets a team carries (an `http` agent's request headers) that must never reach a log, an
//! answer or a saved run. A header value is put in this process's memory the moment it is read
//! and stands everywhere else as a reference, written `{"secret": "<ref>"}`. The reference is good
//! until arenad stops: a team stored before then names a secret that is gone, and says so.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Mutex, OnceLock},
};

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

fn vault() -> &'static Mutex<HashMap<String, String>> {
    static VAULT: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    VAULT.get_or_init(Default::default)
}

/// Keeps `value` in memory and gives back its reference.
fn keep(value: String) -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    vault().lock().unwrap().insert(id.clone(), value);
    id
}

/// The value `reference` stands for, while this process holds it.
fn reveal(reference: &str) -> Option<String> {
    vault().lock().unwrap().get(reference).cloned()
}

/// An `http` agent's request headers: each name with a reference to its value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers(BTreeMap<String, String>);

/// Why a header's value can't be had any more.
pub fn gone(name: &str) -> String {
    format!(
        "the value of header {name:?} was kept in memory by an arenad that has stopped since: connect the agent again"
    )
}

impl Headers {
    /// Headers with these plain values (kept in memory from here on).
    pub fn from_plain(plain: impl IntoIterator<Item = (String, String)>) -> Headers {
        Headers(plain.into_iter().map(|(k, v)| (k, keep(v))).collect())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// Each name with its value; `Err` names the first header whose value is gone.
    pub fn reveal(&self) -> Result<Vec<(String, String)>, String> {
        let one = |(k, r): (&String, &String)| reveal(r).map(|v| (k.clone(), v)).ok_or(gone(k));
        self.0.iter().map(one).collect()
    }
}

/// A header value as written: `{"secret": ref}`.
#[derive(Serialize, Deserialize)]
struct Ref {
    secret: String,
}

/// A header value as read: a plain value (kept from here on) or a reference.
#[derive(Deserialize)]
#[serde(untagged)]
enum ValueIn {
    Plain(String),
    Ref(Ref),
}

impl Serialize for Headers {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let refs = self.0.iter().map(|(k, r)| (k, Ref { secret: r.clone() }));
        s.collect_map(refs)
    }
}

impl<'de> Deserialize<'de> for Headers {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Headers, D::Error> {
        let raw = BTreeMap::<String, ValueIn>::deserialize(d)?;
        let mut out = BTreeMap::new();
        for (name, v) in raw {
            if name.trim().is_empty() {
                return Err(de::Error::custom("a header needs a name"));
            }
            let reference = match v {
                ValueIn::Plain(value) => keep(value),
                ValueIn::Ref(r) => r.secret,
            };
            out.insert(name, reference);
        }
        Ok(Headers(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_value_never_comes_back_out_but_its_reference_does() {
        let h: Headers = serde_json::from_value(json!({"Authorization": "Bearer s3cret"})).unwrap();
        let written = serde_json::to_string(&h).unwrap();
        assert!(!written.contains("s3cret"), "{written}");
        assert!(!format!("{h:?}").contains("s3cret"));
        // What was written reads back as the same secret.
        let again: Headers = serde_json::from_str(&written).unwrap();
        assert_eq!(again, h);
        assert_eq!(
            again.reveal().unwrap(),
            [("Authorization".to_string(), "Bearer s3cret".to_string())]
        );
    }

    #[test]
    fn a_reference_this_process_never_kept_is_gone() {
        let h: Headers = serde_json::from_value(json!({"X-Key": {"secret": "nope"}})).unwrap();
        let e = h.reveal().unwrap_err();
        assert!(
            e.contains("X-Key") && e.contains("connect the agent again"),
            "{e}"
        );
        assert!(serde_json::from_value::<Headers>(json!({" ": "v"})).is_err());
    }
}
