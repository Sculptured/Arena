//! Names (databases, folders) in use right now, which `gc` must leave alone.
use std::collections::HashSet;
use std::sync::Mutex;

#[derive(Default)]
pub struct BusySet(Mutex<HashSet<String>>);

/// Marks a database busy until dropped.
pub struct Busy<'a> {
    set: &'a BusySet,
    db: String,
}

impl BusySet {
    pub fn mark(&self, db: &str) -> Busy<'_> {
        self.0.lock().expect("busy lock").insert(db.to_string());
        Busy {
            set: self,
            db: db.to_string(),
        }
    }

    /// Marks `db` busy, unless it is busy already (`None`).
    pub fn try_mark(&self, db: &str) -> Option<Busy<'_>> {
        let fresh = self.0.lock().expect("busy lock").insert(db.to_string());
        fresh.then(|| Busy {
            set: self,
            db: db.to_string(),
        })
    }

    pub fn snapshot(&self) -> HashSet<String> {
        self.0.lock().expect("busy lock").clone()
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.set.0.lock().expect("busy lock").remove(&self.db);
    }
}
