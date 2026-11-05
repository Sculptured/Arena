//! `inspect` and `diff` through REST: an instance's site is read in place; a save is read
//! through a scratch site made from it and removed after. Building a scratch site takes tens of
//! seconds, so `diff` reads both sides at once and keeps the differing documents of saves for the
//! `inspect` of them that follows (Arena's compare asks for exactly those).
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{oneshot, watch};

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
        if let Err(e) = close_site(&ready.stack, &site).await {
            eprintln!("arena-erpnext: removing scratch site {}: {e}", site.name);
        }
        out
    }

    pub(crate) async fn inspect_target(&self, p: InspectParams) -> Result<Docs, PartError> {
        if let (Target::Save { save }, false) = (&p.target, p.keys.is_empty()) {
            if let Some(docs) = self.docs.get(save, &p.keys) {
                return Ok(Docs { docs });
            }
        }
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
        if let Some(differing) = self.kept_diff(&p) {
            return Ok(Differing { differing });
        }
        let doctypes: Vec<String> = doctypes.into_iter().collect();
        let (keys_tx, keys_rx) = watch::channel(None);
        let ((a_tx, a_rx), (b_tx, b_rx)) = (oneshot::channel(), oneshot::channel());
        let decide = async move {
            let (a, b): (DocMap, DocMap) = (a_rx.await.ok()?, b_rx.await.ok()?);
            let d = Differing::between(&a, &b);
            let _ = keys_tx.send(Some(Arc::new(d.differing.clone())));
            Some(d)
        };
        let (da, db, d) = tokio::join!(
            self.read_side(&p.a, &doctypes, a_tx, keys_rx.clone()),
            self.read_side(&p.b, &doctypes, b_tx, keys_rx),
            decide
        );
        let (da, db) = (da?, db?);
        let d = d.ok_or_else(|| PartError::internal("diff: a side was not read"))?;
        self.keep(&p, [da, db], &d.differing);
        Ok(d)
    }

    /// The diff of two saves made before (saves never change); none for an instance.
    fn kept_diff(&self, p: &DiffParams) -> Option<Vec<String>> {
        match (&p.a, &p.b) {
            (Target::Save { save: a }, Target::Save { save: b }) => self.docs.diff(a, b),
            _ => None,
        }
    }

    /// One side of a diff: its documents go to `dumped`; once the differing keys are known, it
    /// reads their documents before its scratch site goes (none, if that fails: the diff stands).
    async fn read_side(
        &self,
        t: &Target,
        doctypes: &[String],
        dumped: oneshot::Sender<DocMap>,
        mut keys: watch::Receiver<Option<Arc<Vec<String>>>>,
    ) -> Result<Map<String, Value>, PartError> {
        self.read(t, async move |rest, site| {
            let _ = dumped.send(dump_docs(rest, site, doctypes).await?);
            let known = keys.wait_for(Option::is_some).await.map(|k| k.clone());
            let known = known.map_err(|_| PartError::internal("diff: the other side failed"))?;
            let keys: Arc<Vec<String>> = known.unwrap_or_default();
            Ok(some_docs(rest, site, &keys).await.unwrap_or_default())
        })
        .await
    }

    /// Keeps what a diff read from saves: each side's documents and, for two saves, the diff
    /// (an instance changes, so nothing of one is kept).
    fn keep(&self, p: &DiffParams, docs: [Map<String, Value>; 2], differing: &[String]) {
        for (t, d) in [&p.a, &p.b].into_iter().zip(docs) {
            if let Target::Save { save } = t {
                self.docs.put(save, d);
            }
        }
        if let (Target::Save { save: a }, Target::Save { save: b }) = (&p.a, &p.b) {
            self.docs.put_diff(a, b, differing);
        }
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
            Reply::Refused { message: e, .. } => {
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
            Reply::Refused { status: 404, .. } => Value::Null,
            Reply::Refused { message, .. } => {
                if doctype_exists(rest, site, doctype).await? {
                    return Err(PartError::internal(format!("reading {k}: {message}")));
                }
                Value::Null
            }
        };
        out.insert(k.clone(), doc);
    }
    Ok(out)
}

/// False when Frappe has no such doctype: it answers a read of one with a server error, not 404,
/// and a key of a doctype that doesn't exist names no document (arenad probes saves with one).
async fn doctype_exists(rest: &Rest, site: &str, doctype: &str) -> Result<bool, PartError> {
    let path = ["resource", "DocType", doctype];
    let reply = rest.get(site, &path, &[]).await?;
    Ok(!matches!(reply, Reply::Refused { status: 404, .. }))
}
