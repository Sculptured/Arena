//! The generic `postgres` integration: each instance is a database, each save a copy of one.
//!
//! Databases are `pi_<ns><instance>` and `ps_<ns><save>`. `ns` is empty unless env `ARENA_PG_NS`
//! (or `?arena_ns=` on `ARENA_PG_URL`) sets it (tests use it to keep their databases apart from everyone else's). With an empty `ns`,
//! ids that name an existing legacy database (`snap_*`, `branch_*`, `gold_*`, `world_base`) are
//! used as they are, so runs recorded before saves were parts still open.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use arena_part::helpers::pg::{tokio_postgres, Templates};
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
    /// Instance database name -> whether `sql_*` tools are on. Instances started by this process.
    live: Mutex<HashMap<String, bool>>,
}

fn is_legacy(name: &str) -> bool {
    ["snap_", "branch_", "gold_"]
        .iter()
        .any(|p| name.starts_with(p))
        || name == "world_base"
}

fn internal(e: impl std::fmt::Display) -> PartError {
    PartError::internal(format!("{e:#}"))
}

impl Postgres {
    pub fn from_env() -> Postgres {
        let url = std::env::var("ARENA_PG_URL").unwrap_or_else(|_| DEFAULT_URL.into());
        // arenad passes `ARENA_PG_URL` on to programs but not other variables, so the namespace can
        // also ride on the URL as `?arena_ns=<ns>`.
        let (url, from_url) = match url.split_once("?arena_ns=") {
            Some((u, n)) => (u.to_string(), n.to_string()),
            None => (url, String::new()),
        };
        let ns = std::env::var("ARENA_PG_NS").unwrap_or(from_url);
        Postgres::new(&url, &ns)
    }

    pub fn new(url: &str, ns: &str) -> Postgres {
        Postgres {
            pg: Templates::new(url),
            ns: ns.into(),
            live: Mutex::new(HashMap::new()),
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

    /// The database of a live (or surviving) instance.
    async fn instance_db(&self, id: &str) -> Result<String, PartError> {
        let db = self.pi(id);
        if self.pg.exists(&db).await.map_err(internal)? {
            return Ok(db);
        }
        if self.legacy_ok(id) && self.pg.exists(id).await.map_err(internal)? {
            return Ok(id.into());
        }
        Err(PartError::unknown(format!("unknown instance {id}")))
    }

    async fn save_db(&self, id: &str) -> Result<String, PartError> {
        let db = self.ps(id);
        if self.pg.exists(&db).await.map_err(internal)? {
            return Ok(db);
        }
        if self.legacy_ok(id) && self.pg.exists(id).await.map_err(internal)? {
            return Ok(id.into());
        }
        Err(PartError::unknown(format!("unknown save {id}")))
    }

    async fn target_db(&self, t: &Target) -> Result<String, PartError> {
        match t {
            Target::Instance { instance } => self.instance_db(instance).await,
            Target::Save { save } => self.save_db(save).await,
        }
    }

    fn sql_tools_on(&self, db: &str) -> bool {
        self.live.lock().unwrap().get(db).copied().unwrap_or(false)
    }
}

fn tool(name: &str, description: &str, effect: Effect) -> ToolSchema {
    ToolSchema {
        name: name.into(),
        description: description.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "sql": {"type": "string"},
                "params": {"type": "array", "items": {}, "description": "values for $1, $2, .."}
            },
            "required": ["sql"]
        }),
        effect,
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

fn to_params(types: &[Type], vals: &[Value]) -> Result<Vec<Box<dyn ToSql + Sync + Send>>, String> {
    if types.len() != vals.len() {
        return Err(format!(
            "the statement takes {} params, got {}",
            types.len(),
            vals.len()
        ));
    }
    let mut out: Vec<Box<dyn ToSql + Sync + Send>> = vec![];
    for (i, (t, v)) in types.iter().zip(vals).enumerate() {
        let bad = || format!("param ${} ({v}) doesn't fit type {t}", i + 1);
        let b: Box<dyn ToSql + Sync + Send> = if v.is_null() {
            Box::new(None::<String>)
        } else if *t == Type::BOOL {
            Box::new(v.as_bool().ok_or_else(bad)?)
        } else if *t == Type::INT2 {
            Box::new(i16::try_from(v.as_i64().ok_or_else(bad)?).map_err(|_| bad())?)
        } else if *t == Type::INT4 {
            Box::new(i32::try_from(v.as_i64().ok_or_else(bad)?).map_err(|_| bad())?)
        } else if *t == Type::INT8 {
            Box::new(v.as_i64().ok_or_else(bad)?)
        } else if *t == Type::FLOAT4 {
            Box::new(v.as_f64().ok_or_else(bad)? as f32)
        } else if *t == Type::FLOAT8 {
            Box::new(v.as_f64().ok_or_else(bad)?)
        } else if *t == Type::JSON || *t == Type::JSONB {
            Box::new(v.clone())
        } else if *t == Type::TEXT || *t == Type::VARCHAR || *t == Type::BPCHAR || *t == Type::NAME
        {
            Box::new(match v {
                Value::String(s) => s.clone(),
                o => o.to_string(),
            })
        } else {
            return Err(format!("param ${} has unsupported type {t}", i + 1));
        };
        out.push(b);
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

async fn some_rows(c: &Client, keys: &[String]) -> anyhow::Result<Map<String, Value>> {
    let tables: HashMap<String, String> = pk_tables(c).await?.into_iter().collect();
    let mut out = Map::new();
    for k in keys {
        let doc = match k.split_once('/') {
            Some((t, id)) if tables.contains_key(t) => {
                let sql = format!(
                    "SELECT row_to_json(t) FROM {} t WHERE {}::text = $1",
                    q(t),
                    q(&tables[t])
                );
                match c.query_opt(&sql, &[&id]).await? {
                    Some(r) => r.get::<_, Value>(0),
                    None => Value::Null,
                }
            }
            _ => Value::Null,
        };
        out.insert(k.clone(), doc);
    }
    Ok(out)
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
            .map_err(internal)?;
        // describe has no config, so the tools are always listed; call_tool refuses them in
        // instances whose config didn't turn `sql_tools` on.
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
            tools: vec![
                tool(
                    "sql_query",
                    "Run a SELECT and get the rows as JSON objects.",
                    Effect::Read,
                ),
                tool(
                    "sql_execute",
                    "Run an INSERT, UPDATE, DELETE or other statement and get the rows affected.",
                    Effect::Write,
                ),
            ],
        })
    }

    async fn start(&self, p: StartParams) -> Result<(), PartError> {
        let db = self.pi(&p.instance);
        self.pg.drop(&db).await.map_err(internal)?;
        match &p.from {
            Some(f) => {
                let src = self.save_db(&f.save).await?;
                self.pg.create_from(&db, &src).await.map_err(internal)?;
            }
            None => {
                self.pg.create_empty(&db).await.map_err(internal)?;
                if let Some(sql) = p.config.get("init_sql").and_then(Value::as_str) {
                    let r = self
                        .pg
                        .with_db(&db, async |c| Ok(c.batch_execute(sql).await?))
                        .await;
                    if let Err(e) = r {
                        let _ = self.pg.drop(&db).await;
                        return Err(PartError::invalid(format!("init_sql failed: {e:#}")));
                    }
                }
            }
        }
        let tools = p
            .config
            .get("sql_tools")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.live.lock().unwrap().insert(db, tools);
        Ok(())
    }

    async fn call_tool(&self, p: CallToolParams) -> Result<ToolOutput, PartError> {
        let db = self.instance_db(&p.instance).await?;
        let fail = |m: String| {
            Ok(ToolOutput {
                error: Some(m),
                ..Default::default()
            })
        };
        let rows = match p.name.as_str() {
            "sql_query" => true,
            "sql_execute" => false,
            o => return fail(format!("unknown tool {o}")),
        };
        if !self.sql_tools_on(&db) {
            return fail("the sql tools are off in this world (config sql_tools)".into());
        }
        let Some(sql) = p.args.get("sql").and_then(Value::as_str) else {
            return fail("sql must be a string".into());
        };
        let params = match p.args.get("params") {
            None | Some(Value::Null) => vec![],
            Some(Value::Array(a)) => a.clone(),
            Some(_) => return fail("params must be a list".into()),
        };
        let res = self
            .pg
            .with_db(&db, async |c| Ok(run_sql(c, sql, &params, rows).await))
            .await
            .map_err(internal)?;
        match res {
            Err(e) => fail(e),
            Ok(v) if rows => Ok(ToolOutput {
                result: Some(v),
                reads: read_keys(sql),
                ..Default::default()
            }),
            Ok(v) => Ok(ToolOutput {
                result: Some(v),
                writes: write_keys(sql),
                ..Default::default()
            }),
        }
    }

    async fn save(&self, p: SaveParams) -> Result<Saved, PartError> {
        let db = self.instance_db(&p.instance).await?;
        let ps = self.ps(&p.save);
        self.pg.drop(&ps).await.map_err(internal)?;
        self.pg.create_from(&ps, &db).await.map_err(internal)?;
        let bytes = self.pg.size(&ps).await.map_err(internal)?.unwrap_or(0);
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
                    Ok(all_rows(c).await?.into_iter().collect())
                } else {
                    some_rows(c, &p.keys).await
                }
            })
            .await
            .map_err(internal)?;
        Ok(Docs { docs })
    }

    async fn diff(&self, p: DiffParams) -> Result<Differing, PartError> {
        let (a, b) = (self.target_db(&p.a).await?, self.target_db(&p.b).await?);
        // One database at a time: never two connections held at once.
        let ra = self.pg.with_db(&a, async |c| all_rows(c).await).await;
        let rb = self.pg.with_db(&b, async |c| all_rows(c).await).await;
        let (ra, rb) = (ra.map_err(internal)?, rb.map_err(internal)?);
        let keys: HashSet<&String> = ra.keys().chain(rb.keys()).collect();
        let mut differing: Vec<String> = keys
            .into_iter()
            .filter(|k| ra.get(*k) != rb.get(*k))
            .cloned()
            .collect();
        differing.sort();
        Ok(Differing { differing })
    }

    async fn drop_save(&self, p: SaveId) -> Result<(), PartError> {
        let ps = self.ps(&p.save);
        if self.pg.exists(&ps).await.map_err(internal)? {
            return self.pg.drop(&ps).await.map_err(internal);
        }
        if self.legacy_ok(&p.save) && p.save.starts_with("snap_") {
            return self.pg.drop(&p.save).await.map_err(internal);
        }
        Ok(())
    }

    async fn stop(&self, p: IdParams) -> Result<(), PartError> {
        // The database stays (inspect after a run); `gc` drops it once nobody asks for it.
        self.live.lock().unwrap().remove(&self.pi(&p.instance));
        Ok(())
    }

    async fn usage(&self) -> Result<Usage, PartError> {
        // Saves only: live instances are working copies, and the conformance check requires the
        // figure to fall back once its saves are dropped while its instances still run.
        let mut prefixes = vec![format!("ps_{}", self.ns)];
        if self.ns.is_empty() {
            prefixes.push("snap_".into());
        }
        let mut bytes = 0i64;
        for p in prefixes {
            for (_, s) in self.pg.list(&p).await.map_err(internal)? {
                bytes += s;
            }
        }
        Ok(Usage {
            bytes: bytes.max(0) as u64,
        })
    }

    async fn gc(&self, p: GcParams) -> Result<GcResult, PartError> {
        let keep: HashSet<&str> = p.keep.iter().map(String::as_str).collect();
        let live: HashSet<String> = self.live.lock().unwrap().keys().cloned().collect();
        let mut out = GcResult::default();
        let mut doomed = vec![];
        // (prefix, is instance)
        let mut kinds = vec![
            (format!("ps_{}", self.ns), false),
            (format!("pi_{}", self.ns), true),
        ];
        if self.ns.is_empty() {
            kinds.push(("snap_".into(), false));
            kinds.push(("branch_".into(), true));
        }
        for (prefix, instance) in kinds {
            for (name, size) in self.pg.list(&prefix).await.map_err(internal)? {
                // `pi_`/`ps_` ids drop the prefix; legacy names are their own id.
                let id = if prefix.starts_with("p") {
                    &name[prefix.len()..]
                } else {
                    &name[..]
                };
                if keep.contains(id) || (instance && live.contains(&name)) {
                    continue;
                }
                doomed.push((name, size));
            }
        }
        for (name, size) in doomed {
            self.pg.drop(&name).await.map_err(internal)?;
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
