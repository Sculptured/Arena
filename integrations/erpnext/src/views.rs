//! `inspect` and `diff` through REST: an instance's site is read in place; a save is read
//! through a scratch site made from it and removed after.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use arena_part::proto::{DiffParams, Differing, Docs, InspectParams, Target};
use arena_part::PartError;
use serde_json::{json, Map, Value};

use crate::instances::close_site;
use crate::names::{key, parse_key, Site};
use crate::rest::{Reply, Rest};
use crate::saves::SaveMeta;
use crate::Erpnext;

/// A doctype's documents by key, every field of each.
type DocMap = BTreeMap<String, Value>;

fn next_view() -> Site {
    static N: AtomicU64 = AtomicU64::new(0);
    Site::view(N.fetch_add(1, Ordering::Relaxed))
}

impl Erpnext {
    /// The doctypes `diff` compares for `t`: its instance's settings, or what its save recorded.
    fn doctypes_of(&self, t: &Target) -> Result<Vec<String>, PartError> {
        match t {
            Target::Instance { instance } => Ok(self.instance(instance)?.settings.diff_doctypes),
            Target::Save { save } => {
                let dir = self.dirs.find(save);
                let dir = dir.ok_or_else(|| PartError::unknown(format!("unknown save {save}")))?;
                let meta = SaveMeta::read(&dir);
                Ok(meta
                    .map_err(|e| PartError::internal(format!("save {save}: {e}")))?
                    .diff_doctypes)
            }
        }
    }

    /// Runs `f` with the REST client and the site to read `t` from.
    async fn read<T>(
        &self,
        t: &Target,
        f: impl AsyncFnOnce(&Rest, &str) -> Result<T, PartError>,
    ) -> Result<T, PartError> {
        let ready = self.ready().await?;
        let save = match t {
            Target::Instance { instance } => {
                return f(&ready.rest, &self.instance(instance)?.site.name).await;
            }
            Target::Save { save } => save,
        };
        let site = next_view();
        let _busy = self.busy.mark(&site.db);
        let source = self.source(ready, Some(save))?;
        self.open_site(ready, &source, &site).await?;
        let out = f(&ready.rest, &site.name).await;
        close_site(&ready.stack, &site).await?;
        out
    }

    pub(crate) async fn inspect_target(&self, p: InspectParams) -> Result<Docs, PartError> {
        let all = match p.keys.is_empty() {
            true => self.doctypes_of(&p.target)?,
            false => vec![],
        };
        let docs = self
            .read(&p.target, async |rest, site| match p.keys.is_empty() {
                true => Ok(dump_docs(rest, site, &all).await?.into_iter().collect()),
                false => some_docs(rest, site, &p.keys).await,
            })
            .await?;
        Ok(Docs { docs })
    }

    pub(crate) async fn diff_targets(&self, p: DiffParams) -> Result<Differing, PartError> {
        let mut doctypes: BTreeSet<String> = self.doctypes_of(&p.a)?.into_iter().collect();
        doctypes.extend(self.doctypes_of(&p.b)?);
        if p.a == p.b {
            return Ok(Differing { differing: vec![] });
        }
        let doctypes: Vec<String> = doctypes.into_iter().collect();
        let a = self
            .read(&p.a, async |r, s| dump_docs(r, s, &doctypes).await)
            .await?;
        let b = self
            .read(&p.b, async |r, s| dump_docs(r, s, &doctypes).await)
            .await?;
        Ok(Differing::between(&a, &b))
    }
}

/// Every document of each doctype, every field.
async fn dump_docs(rest: &Rest, site: &str, doctypes: &[String]) -> Result<DocMap, PartError> {
    let query = [
        ("fields", json!(["*"]).to_string()),
        ("limit_page_length", "0".into()),
    ];
    let mut out = DocMap::new();
    for doctype in doctypes {
        let rows = match rest.get(site, &["resource", doctype], &query).await? {
            Reply::Ok(Value::Array(rows)) => rows,
            Reply::Ok(other) => {
                return Err(PartError::internal(format!("listing {doctype}: {other}")))
            }
            Reply::Refused(e) => {
                return Err(PartError::invalid(format!("can't list {doctype}: {e}")))
            }
        };
        for row in rows {
            if let Some(name) = row.get("name").and_then(Value::as_str) {
                out.insert(key(doctype, name), row.clone());
            }
        }
    }
    Ok(out)
}

/// The documents of `keys` (null for one there is not).
async fn some_docs(
    rest: &Rest,
    site: &str,
    keys: &[String],
) -> Result<Map<String, Value>, PartError> {
    let mut out = Map::new();
    for k in keys {
        let (doctype, name) = parse_key(k)
            .ok_or_else(|| PartError::invalid(format!("{k:?} is not a key (<Doctype>/<name>)")))?;
        let doc = match rest.get(site, &["resource", doctype, name], &[]).await? {
            Reply::Ok(doc) => doc,
            Reply::Refused(_) => Value::Null,
        };
        out.insert(k.clone(), doc);
    }
    Ok(out)
}
