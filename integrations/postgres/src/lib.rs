//! The generic `postgres` integration: each instance is a database, each save a copy of one.
//!
//! Databases are `pi_<ns><instance>` and `ps_<ns><save>`. `ns` is empty unless env `ARENA_PG_NS`
//! (or `?arena_ns=` on `ARENA_PG_URL`) sets it (tests use it to keep their databases apart from everyone else's). With an empty `ns`,
//! ids that name an existing legacy database (`snap_*`, `branch_*`, `gold_*`, `world_base`) are
//! used as they are, so runs recorded before saves were parts still open.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use arena_part::helpers::pg::{tokio_postgres, Templates};
use arena_part::helpers::sql_tools::{
    lookup, schemas, sql_args, tool_error, tool_output, which, KeyLookup, SqlTool,
};
use arena_part::proto::*;
use arena_part::{Part, PartError};
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::Client;

const DEFAULT_URL: &str = "postgres://arena:arena@127.0.0.1:5544/postgres";

pub struct Postgres {
    pg: Templates,
    ns: String,
    /// `arena-postgres --sql-tools`: list and serve `sql_query` and `sql_execute`.
    sql_tools: bool,
    /// Instance databases started by this process.
    live: Mutex<HashSet<String>>,
}

/// Takes `arena_ns=<ns>` out of the URL's query string; other query parameters stay.
fn split_ns(url: &str) -> (String, String) {
    let Some((base, query)) = url.split_once('?') else {
        return (url.to_string(), String::new());
    };
    let mut ns = String::new();
    let mut rest = vec![];
    for kv in query.split('&') {
        match kv.strip_prefix("arena_ns=") {
            Some(n) => ns = n.to_string(),
            None => rest.push(kv),
        }
    }
    let rest: Vec<&str> = rest.into_iter().filter(|k| !k.is_empty()).collect();
    if rest.is_empty() {
        (base.to_string(), ns)
    } else {
        (format!("{base}?{}", rest.join("&")), ns)
    }
}

fn is_legacy(name: &str) -> bool {
    ["snap_", "branch_", "gold_"]
        .iter()
        .any(|p| name.starts_with(p))
        || name == "world_base"
}

/// Postgres database names are at most this long.
const MAX_DB_NAME: usize = 63;

impl Postgres {
    /// An id is a valid protocol id whose database names (`pi_<ns><id>`, `ps_<ns><id>`) fit.
    fn check_id(&self, id: &str) -> Result<(), PartError> {
        if !valid_id(id) {
            return Err(PartError::invalid(
                "instance and save ids must match [a-z0-9_]{1,63}",
            ));
        }
        if self.pi(id).len() > MAX_DB_NAME {
            return Err(PartError::invalid(format!(
                "the id {id} is too long for the namespace {:?} (database names are at most {MAX_DB_NAME} bytes)",
                self.ns
            )));
        }
        Ok(())
    }

    pub fn from_env() -> Postgres {
        let url = std::env::var("ARENA_PG_URL").unwrap_or_else(|_| DEFAULT_URL.into());
        // arenad passes `ARENA_PG_URL` on to programs but not other variables, so the namespace can
        // also ride on the URL as `?arena_ns=<ns>`.
        let (url, from_url) = split_ns(&url);
        let ns = std::env::var("ARENA_PG_NS").unwrap_or(from_url);
        Postgres::new(&url, &ns)
    }

    pub fn new(url: &str, ns: &str) -> Postgres {
        Postgres {
            pg: Templates::new(url),
            ns: ns.into(),
            sql_tools: false,
            live: Mutex::new(HashSet::new()),
        }
    }

    fn pi(&self, id: &str) -> String {
        format!("pi_{}{id}", self.ns)
    }

    fn ps(&self, id: &str) -> String {
        format!("ps_{}{id}", self.ns)
    }

    fn legacy_ok(&self, id: &str) -> bool {
        self.ns.is_empty() && is_legacy(id)
    }

    /// The database named `db`, or the legacy one named `id`; `what` names the thing in errors.
    async fn existing_db(&self, what: &str, id: &str, db: String) -> Result<String, PartError> {
        self.check_id(id)?;
        if self
            .pg
            .exists(&db)
            .await
            .map_err(PartError::internal_from)?
        {
            return Ok(db);
        }
        if self.legacy_ok(id) && self.pg.exists(id).await.map_err(PartError::internal_from)? {
            return Ok(id.into());
        }
        Err(PartError::unknown(format!("unknown {what} {id}")))
    }

    /// The database of a live (or surviving) instance.
    async fn instance_db(&self, id: &str) -> Result<String, PartError> {
        self.existing_db("instance", id, self.pi(id)).await
    }

    async fn save_db(&self, id: &str) -> Result<String, PartError> {
        self.existing_db("save", id, self.ps(id)).await
    }

    /// A new empty instance database, running the config's `init_sql` in it.
    async fn create_fresh(&self, db: &str, config: &Value) -> Result<(), PartError> {
        self.pg
            .create_empty(db)
            .await
            .map_err(PartError::internal_from)?;
        let Some(sql) = config.get("init_sql").and_then(Value::as_str) else {
            return Ok(());
        };
        let r = self
            .pg
            .with_db(db, async |c| Ok(c.batch_execute(sql).await?))
            .await;
        if let Err(e) = r {
            let _ = self.pg.drop(db).await;
            return Err(PartError::invalid(format!("init_sql failed: {e:#}")));
        }
        Ok(())
    }

    /// The kinds of database this part owns (plus the legacy kinds, with no namespace).
    fn gc_kinds(&self) -> Vec<GcKind> {
        let kind = |prefix: String, instance, strip| GcKind {
            prefix,
            instance,
            strip,
        };
        let mut kinds = vec![
            kind(format!("ps_{}", self.ns), false, true),
            kind(format!("pi_{}", self.ns), true, true),
        ];
        if self.ns.is_empty() {
            kinds.push(kind("snap_".into(), false, false));
            kinds.push(kind("branch_".into(), true, false));
        }
        kinds
    }

    async fn target_db(&self, t: &Target) -> Result<String, PartError> {
        match t {
            Target::Instance { instance } => self.instance_db(instance).await,
            Target::Save { save } => self.save_db(save).await,
        }
    }

    /// Turns on the `sql_query` and `sql_execute` tools (for the conformance check and simple worlds).
    pub fn with_sql_tools(mut self) -> Postgres {
        self.sql_tools = true;
        self
    }
}

// ---- parsing sql for read and write keys ----

fn words(sql: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for ch in sql.chars() {
        if ch == '"' {
            quoted = !quoted;
            cur.push(ch);
        } else if quoted || ch.is_alphanumeric() || ch == '_' || ch == '.' {
            cur.push(ch);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if ch == '(' {
                out.push("(".into());
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `public."T"` -> `T`; `None` if it is not a plain name.
fn table_name(w: &str) -> Option<String> {
    let last = w.rsplit('.').next()?;
    let last = last.trim_matches('"');
    (!last.is_empty() && last != "(" && last.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .then(|| last.to_string())
}

fn table_keys(tables: impl IntoIterator<Item = String>) -> Vec<String> {
    let set: std::collections::BTreeSet<_> = tables.into_iter().collect();
    set.into_iter().map(|t| format!("{t}/*")).collect()
}

/// `<table>/*` for each table named after FROM or JOIN; `[]` if none can be read out.
pub fn read_keys(sql: &str) -> Vec<String> {
    let w = words(sql);
    let mut t = vec![];
    for i in 0..w.len().saturating_sub(1) {
        if w[i].eq_ignore_ascii_case("from") || w[i].eq_ignore_ascii_case("join") {
            if let Some(n) = table_name(&w[i + 1]) {
                t.push(n);
            }
        }
    }
    table_keys(t)
}

/// `<table>/*` for the INSERT, UPDATE or DELETE target; `["*"]` if it can't be parsed.
pub fn write_keys(sql: &str) -> Vec<String> {
    let w = words(sql);
    let is = |i: usize, s: &str| w.get(i).is_some_and(|x| x.eq_ignore_ascii_case(s));
    let target = if is(0, "insert") && is(1, "into") || is(0, "delete") && is(1, "from") {
        w.get(2)
    } else if is(0, "update") {
        w.get(1)
    } else {
        None
    };
    match target.and_then(|t| table_name(t)) {
        Some(t) => vec![format!("{t}/*")],
        None => vec!["*".into()],
    }
}

// ---- params ----

type Param = Box<dyn ToSql + Sync + Send>;

/// The parameter types a statement can take.
#[derive(Clone, Copy)]
enum Kind {
    Bool,
    Int2,
    Int4,
    Int8,
    Float4,
    Float8,
    Json,
    Text,
}

fn kind_of(t: &Type) -> Option<Kind> {
    Some(if *t == Type::BOOL {
        Kind::Bool
    } else if *t == Type::INT2 {
        Kind::Int2
    } else if *t == Type::INT4 {
        Kind::Int4
    } else if *t == Type::INT8 {
        Kind::Int8
    } else if *t == Type::FLOAT4 {
        Kind::Float4
    } else if *t == Type::FLOAT8 {
        Kind::Float8
    } else if *t == Type::JSON || *t == Type::JSONB {
        Kind::Json
    } else if *t == Type::TEXT || *t == Type::VARCHAR || *t == Type::BPCHAR || *t == Type::NAME {
        Kind::Text
    } else {
        return None;
    })
}

fn null_param(kind: Kind) -> Param {
    match kind {
        Kind::Bool => Box::new(None::<bool>),
        Kind::Int2 => Box::new(None::<i16>),
        Kind::Int4 => Box::new(None::<i32>),
        Kind::Int8 => Box::new(None::<i64>),
        Kind::Float4 => Box::new(None::<f32>),
        Kind::Float8 => Box::new(None::<f64>),
        Kind::Json => Box::new(None::<Value>),
        Kind::Text => Box::new(None::<String>),
    }
}

/// `None` when the (non-null) value doesn't fit the kind.
fn value_param(kind: Kind, v: &Value) -> Option<Param> {
    Some(match kind {
        Kind::Bool => Box::new(v.as_bool()?),
        Kind::Int2 => Box::new(i16::try_from(v.as_i64()?).ok()?),
        Kind::Int4 => Box::new(i32::try_from(v.as_i64()?).ok()?),
        Kind::Int8 => Box::new(v.as_i64()?),
        Kind::Float4 => Box::new(v.as_f64()? as f32),
        Kind::Float8 => Box::new(v.as_f64()?),
        Kind::Json => Box::new(v.clone()),
        Kind::Text => Box::new(match v {
            Value::String(s) => s.clone(),
            o => o.to_string(),
        }),
    })
}

fn to_params(types: &[Type], vals: &[Value]) -> Result<Vec<Param>, String> {
    if types.len() != vals.len() {
        return Err(format!(
            "the statement takes {} params, got {}",
            types.len(),
            vals.len()
        ));
    }
    let mut out = vec![];
    for (i, (t, v)) in types.iter().zip(vals).enumerate() {
        let n = i + 1;
        let kind = kind_of(t).ok_or_else(|| format!("param ${n} has unsupported type {t}"))?;
        out.push(if v.is_null() {
            null_param(kind)
        } else {
            value_param(kind, v).ok_or_else(|| format!("param ${n} ({v}) doesn't fit type {t}"))?
        });
    }
    Ok(out)
}

fn db_err(e: tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

async fn run_sql(c: &Client, sql: &str, params: &[Value], rows: bool) -> Result<Value, String> {
    let sql = sql.trim().trim_end_matches(';');
    let full = if rows {
        format!("SELECT row_to_json(q) FROM ({sql}) q")
    } else {
        sql.to_string()
    };
    let st = c.prepare(&full).await.map_err(db_err)?;
    let owned = to_params(st.params(), params)?;
    let refs: Vec<&(dyn ToSql + Sync)> =
        owned.iter().map(|b| &**b as &(dyn ToSql + Sync)).collect();
    if rows {
        let rs = c.query(&st, &refs).await.map_err(db_err)?;
        Ok(Value::Array(
            rs.iter().map(|r| r.get::<_, Value>(0)).collect(),
        ))
    } else {
        let n = c.execute(&st, &refs).await.map_err(db_err)?;
        Ok(json!({"rows_affected": n}))
    }
}

// ---- reading rows for inspect and diff ----

const PK_TABLES: &str = "SELECT c.relname::text, a.attname::text FROM pg_index i \
    JOIN pg_class c ON c.oid = i.indrelid \
    JOIN pg_namespace n ON n.oid = c.relnamespace \
    JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = i.indkey[0] \
    WHERE i.indisprimary AND i.indnkeyatts = 1 AND n.nspname = 'public' \
    AND c.relkind IN ('r', 'p') ORDER BY 1";

fn q(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

async fn pk_tables(c: &Client) -> anyhow::Result<Vec<(String, String)>> {
    let rows = c.query(PK_TABLES, &[]).await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// Every row of every table with a one-column primary key, keyed `<table>/<pk>`.
async fn all_rows(c: &Client) -> anyhow::Result<BTreeMap<String, Value>> {
    let mut out = BTreeMap::new();
    for (t, pk) in pk_tables(c).await? {
        let sql = format!("SELECT {}::text, row_to_json(t) FROM {} t", q(&pk), q(&t));
        for r in c.query(&sql, &[]).await? {
            let id: String = r.get(0);
            out.insert(format!("{t}/{id}"), r.get::<_, Value>(1));
        }
    }
    Ok(out)
}

/// A fingerprint of every table the rows-by-key view doesn't cover (no one-column primary key, or
/// not in schema `public`), keyed `<table>/*` or `<schema>.<table>/*`.
async fn whole_tables(c: &Client) -> anyhow::Result<BTreeMap<String, String>> {
    let pk: HashSet<String> = pk_tables(c).await?.into_iter().map(|x| x.0).collect();
    let mut out = BTreeMap::new();
    for (schema, t) in user_tables(c).await? {
        if schema == "public" && pk.contains(&t) {
            continue;
        }
        let sql = format!(
            "SELECT md5(coalesce(string_agg(r::text, E'\\n' ORDER BY r::text), '')) FROM {}.{} r",
            q(&schema),
            q(&t)
        );
        let h: String = c.query_one(&sql, &[]).await?.get(0);
        let key = if schema == "public" {
            format!("{t}/*")
        } else {
            format!("{schema}.{t}/*")
        };
        out.insert(key, h);
    }
    Ok(out)
}

async fn user_tables(c: &Client) -> anyhow::Result<Vec<(String, String)>> {
    let rows = c
        .query(
            "SELECT n.nspname::text, c.relname::text FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r', 'p') AND NOT c.relispartition \
             AND n.nspname <> 'information_schema' AND n.nspname NOT LIKE 'pg\\_%' ORDER BY 1, 2",
            &[],
        )
        .await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// `Ok(Err(message))` when a key names a public table that has no one-column primary key.
async fn some_rows(
    c: &Client,
    keys: &[String],
) -> anyhow::Result<Result<Map<String, Value>, String>> {
    let pks: HashMap<String, String> = pk_tables(c).await?.into_iter().collect();
    let all: Vec<(String, String)> = user_tables(c).await?;
    let is_table = |t: &str| all.iter().any(|(s, n)| s == "public" && n == t);
    let mut out = Map::new();
    for k in keys {
        let doc = match lookup(k, &pks, is_table) {
            Err(m) => return Ok(Err(m)),
            Ok(KeyLookup::Missing) => Value::Null,
            Ok(KeyLookup::Row { table, pk, id }) => {
                let sql = format!(
                    "SELECT row_to_json(t) FROM {} t WHERE {}::text = $1",
                    q(table),
                    q(pk)
                );
                match c.query_opt(&sql, &[&id]).await? {
                    Some(r) => r.get::<_, Value>(0),
                    None => Value::Null,
                }
            }
        };
        out.insert(k.clone(), doc);
    }
    Ok(Ok(out))
}

/// The names of the databases a gc may drop, and how to read the id out of a name.
struct GcKind {
    prefix: String,
    instance: bool,
    /// Whether the prefix is not part of the id (`pi_`/`ps_`); legacy names are their own id.
    strip: bool,
}

impl GcKind {
    /// Kept when `keep` names its id (an entry ending in `*` is a prefix), or a live instance.
    fn is_kept(&self, name: &str, keep: &[String], live: &HashSet<String>) -> bool {
        let id = if self.strip {
            &name[self.prefix.len()..]
        } else {
            name
        };
        kept(keep, id) || (self.instance && live.contains(name))
    }
}

/// Both sides of a diff: every row, plus a fingerprint of each table without a one-column key.
async fn comparable_rows(c: &mut Client) -> anyhow::Result<BTreeMap<String, Value>> {
    let mut rows = all_rows(c).await?;
    rows.extend(
        whole_tables(c)
            .await?
            .into_iter()
            .map(|(k, h)| (k, json!(h))),
    );
    Ok(rows)
}

#[async_trait]
impl Part for Postgres {
    async fn describe(&self) -> Result<Describe, PartError> {
        let version = self
            .pg
            .with_db("postgres", async |c| {
                Ok(c.query_one("SHOW server_version", &[])
                    .await?
                    .get::<_, String>(0))
            })
            .await
            .map_err(PartError::internal_from)?;
        Ok(Describe {
            name: "postgres".into(),
            version: "0.1.0".into(),
            protocol: PROTOCOL,
            restore: Restore {
                exact: ExactKind::Save,
                approximate: false,
            },
            uses_clock: false,
            fingerprint: json!({"server_version": version}),
            tools: if self.sql_tools {
                schemas("values for $1, $2, ..")
            } else {
                vec![]
            },
        })
    }

    async fn start(&self, p: StartParams) -> Result<(), PartError> {
        self.check_id(&p.instance)?;
        let db = self.pi(&p.instance);
        self.pg.drop(&db).await.map_err(PartError::internal_from)?;
        match &p.from {
            Some(f) => {
                let src = self.save_db(&f.save).await?;
                self.pg
                    .create_from(&db, &src)
                    .await
                    .map_err(PartError::internal_from)?;
            }
            None => self.create_fresh(&db, &p.config).await?,
        }
        self.live.lock().unwrap().insert(db);
        Ok(())
    }

    async fn call_tool(&self, p: CallToolParams) -> Result<ToolOutput, PartError> {
        let db = self.instance_db(&p.instance).await?;
        let tool = match which(&p.name, self.sql_tools) {
            Ok(t) => t,
            Err(out) => return Ok(out),
        };
        let (sql, params) = match sql_args(&p.args) {
            Ok(a) => a,
            Err(m) => return Ok(tool_error(m)),
        };
        let rows = tool == SqlTool::Query;
        let res = self
            .pg
            .with_db(&db, async |c| Ok(run_sql(c, sql, params, rows).await))
            .await
            .map_err(PartError::internal_from)?;
        let keys = if rows {
            read_keys(sql)
        } else {
            write_keys(sql)
        };
        Ok(tool_output(tool, res, keys))
    }

    async fn save(&self, p: SaveParams) -> Result<Saved, PartError> {
        self.check_id(&p.save)?;
        let db = self.instance_db(&p.instance).await?;
        let ps = self.ps(&p.save);
        self.pg.drop(&ps).await.map_err(PartError::internal_from)?;
        // a benchmark may hold its own connection to the instance: close it for the copy
        self.pg
            .create_from_exclusive(&ps, &db)
            .await
            .map_err(PartError::internal_from)?;
        let bytes = self
            .pg
            .size(&ps)
            .await
            .map_err(PartError::internal_from)?
            .unwrap_or(0);
        Ok(Saved {
            bytes: bytes.max(0) as u64,
            exact: true,
        })
    }

    async fn inspect(&self, p: InspectParams) -> Result<Docs, PartError> {
        let db = self.target_db(&p.target).await?;
        let docs = self
            .pg
            .with_db(&db, async |c| {
                if p.keys.is_empty() {
                    Ok(Ok(all_rows(c).await?.into_iter().collect()))
                } else {
                    some_rows(c, &p.keys).await
                }
            })
            .await
            .map_err(PartError::internal_from)?
            .map_err(PartError::invalid)?;
        Ok(Docs { docs })
    }

    async fn diff(&self, p: DiffParams) -> Result<Differing, PartError> {
        let (a, b) = (self.target_db(&p.a).await?, self.target_db(&p.b).await?);
        // One database at a time: never two connections held at once.
        let ra = self
            .pg
            .with_db(&a, async |c| comparable_rows(c).await)
            .await;
        let rb = self
            .pg
            .with_db(&b, async |c| comparable_rows(c).await)
            .await;
        let (ra, rb) = (
            ra.map_err(PartError::internal_from)?,
            rb.map_err(PartError::internal_from)?,
        );
        Ok(Differing::between(&ra, &rb))
    }

    /// Drops the save's database; an unknown save is -32002. An old run's snapshot or final branch
    /// database is dropped too, never `gold_*` or `world_base` (a no-op for those).
    async fn drop_save(&self, p: SaveId) -> Result<(), PartError> {
        self.check_id(&p.save)?;
        let exists = async |db: &str| self.pg.exists(db).await.map_err(PartError::internal_from);
        let ps = self.ps(&p.save);
        let db = if exists(&ps).await? {
            ps
        } else if self.legacy_ok(&p.save) && exists(&p.save).await? {
            p.save.clone()
        } else {
            return Err(PartError::unknown(format!("unknown save {}", p.save)));
        };
        if db.starts_with("gold_") || db == "world_base" {
            return Ok(());
        }
        self.pg.drop(&db).await.map_err(PartError::internal_from)
    }

    async fn stop(&self, p: IdParams) -> Result<(), PartError> {
        self.check_id(&p.instance)?;
        // arenad saves what it wants to keep before it stops an instance.
        let db = self.pi(&p.instance);
        self.live.lock().unwrap().remove(&db);
        self.pg.drop(&db).await.map_err(PartError::internal_from)
    }

    async fn usage(&self) -> Result<Usage, PartError> {
        let mut bytes = 0i64;
        for kind in self.gc_kinds() {
            for (_, size) in self
                .pg
                .list(&kind.prefix)
                .await
                .map_err(PartError::internal_from)?
            {
                bytes += size;
            }
        }
        Ok(Usage {
            bytes: bytes.max(0) as u64,
        })
    }

    async fn gc(&self, p: GcParams) -> Result<GcResult, PartError> {
        let live: HashSet<String> = self.live.lock().unwrap().clone();
        let mut doomed = vec![];
        for kind in self.gc_kinds() {
            for (name, size) in self
                .pg
                .list(&kind.prefix)
                .await
                .map_err(PartError::internal_from)?
            {
                if !kind.is_kept(&name, &p.keep, &live) {
                    doomed.push((name, size));
                }
            }
        }
        let mut out = GcResult::default();
        for (name, size) in doomed {
            self.pg
                .drop(&name)
                .await
                .map_err(PartError::internal_from)?;
            out.dropped += 1;
            out.bytes += size.max(0) as u64;
        }
        Ok(out)
    }

    async fn connect(&self, p: IdParams) -> Result<Value, PartError> {
        let db = self.instance_db(&p.instance).await?;
        Ok(json!({"url": self.pg.db_url(&db)}))
    }
}

#[cfg(test)]
mod tests {
    use super::split_ns;

    #[test]
    fn ns_stops_at_ampersand_and_keeps_other_params() {
        let u = "postgres://a:b@h:1/postgres";
        assert_eq!(split_ns(u), (u.into(), String::new()));
        assert_eq!(
            split_ns(&format!("{u}?arena_ns=x_")),
            (u.into(), "x_".into())
        );
        assert_eq!(
            split_ns(&format!("{u}?arena_ns=x_&sslmode=disable")),
            (format!("{u}?sslmode=disable"), "x_".into())
        );
        assert_eq!(
            split_ns(&format!("{u}?sslmode=disable&arena_ns=y_")),
            (format!("{u}?sslmode=disable"), "y_".into())
        );
    }
}
