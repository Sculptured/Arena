//! The transaction view over the rows (cached, with read and write keys recorded), and the read
//! tools.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::{json, Value};
use tokio_postgres::Transaction;

use super::calc::calculate;
use super::fail::{bad, R};
use super::schema::is_write;
use super::value::{fold, py_json_str, st, strs};

pub(super) struct Retail<'a> {
    pub(super) tx: &'a Transaction<'a>,
    pub(super) lock: bool,
    pub(super) cache: HashMap<String, Value>,
    pub(super) all_loaded: HashSet<&'static str>,
    pub(super) reads: BTreeSet<String>,
    pub(super) writes: BTreeSet<String>,
}

impl<'a> Retail<'a> {
    async fn row(&mut self, table: &'static str, id: &str) -> R<Option<Value>> {
        let key = format!("{table}/{id}");
        if let Some(v) = self.cache.get(&key) {
            return Ok(Some(v.clone()));
        }
        if self.all_loaded.contains(table) {
            return Ok(None);
        }
        let q = format!(
            "SELECT doc FROM {table} WHERE id = $1{}",
            if self.lock { " FOR UPDATE" } else { "" }
        );
        match self.tx.query_opt(&q, &[&id]).await? {
            Some(r) => {
                let doc: Value = r.get(0);
                self.cache.insert(key, doc.clone());
                Ok(Some(doc))
            }
            None => Ok(None),
        }
    }

    async fn all(&mut self, table: &'static str) -> R<Vec<(String, Value)>> {
        if self.all_loaded.insert(table) {
            for r in self
                .tx
                .query(&format!("SELECT id, doc FROM {table}"), &[])
                .await?
            {
                let id: String = r.get(0);
                self.cache
                    .entry(format!("{table}/{id}"))
                    .or_insert_with(|| r.get(1));
            }
        }
        let prefix = format!("{table}/");
        let mut v: Vec<(String, Value)> = self
            .cache
            .iter()
            .filter_map(|(k, d)| {
                k.strip_prefix(&prefix)
                    .map(|id| (id.to_string(), d.clone()))
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(v)
    }

    /// The tool's result: `doc`, written back as `table/id` when the tool succeeds.
    pub(super) fn finish(&mut self, table: &str, id: &str, doc: Value) -> R<Value> {
        let key = format!("{table}/{id}");
        self.cache.insert(key.clone(), doc.clone());
        self.writes.insert(key);
        Ok(doc)
    }

    /// A document that must exist; `missing` is the error when it doesn't.
    async fn must(&mut self, table: &'static str, missing: &str, id: &str) -> R<Value> {
        self.reads.insert(format!("{table}/{id}"));
        match self.row(table, id).await? {
            Some(v) => Ok(v),
            None => bad(missing),
        }
    }

    pub(super) async fn order(&mut self, id: &str) -> R<Value> {
        self.must("orders", "Order not found", id).await
    }

    pub(super) async fn user(&mut self, id: &str) -> R<Value> {
        self.must("users", "User not found", id).await
    }

    async fn product(&mut self, id: &str) -> R<Value> {
        self.must("products", "Product not found", id).await
    }

    pub(super) async fn variant(&mut self, product_id: &str, variant_id: &str) -> R<Value> {
        let product = self.product(product_id).await?;
        member(&product["variants"], variant_id, "Variant not found")
    }

    pub(super) async fn payment(&mut self, user_id: &str, pm: &str) -> R<Value> {
        let user = self.user(user_id).await?;
        member(&user["payment_methods"], pm, "Payment method not found")
    }

    /// In-place balance update on the (cached) user, as Python mutates the method dict.
    pub(super) fn set_balance(&mut self, user_id: &str, pm: &str, balance: f64) {
        let key = format!("users/{user_id}");
        if let Some(u) = self.cache.get_mut(&key) {
            u["payment_methods"][pm]["balance"] = json!(balance);
        }
        self.writes.insert(key);
    }

    /// Runs the named tool with its (already checked) arguments.
    pub(super) async fn run(&mut self, name: &str, a: &Value) -> R<Value> {
        if !is_write(name) {
            self.run_read(name, a).await
        } else {
            self.run_write(name, a).await
        }
    }

    async fn run_read(&mut self, name: &str, a: &Value) -> R<Value> {
        let s = |k: &str| st(&a[k]);
        match name {
            "calculate" => calculate(&s("expression")),
            "find_user_id_by_name_zip" => {
                self.find_user_by_name_zip(&s("first_name"), &s("last_name"), &s("zip"))
                    .await
            }
            "find_user_id_by_email" => self.find_user_by_email(&s("email")).await,
            "get_order_details" => self.order(&s("order_id")).await,
            "get_product_details" => self.product(&s("product_id")).await,
            "get_item_details" => self.item_details(&s("item_id")).await,
            "get_user_details" => self.user(&s("user_id")).await,
            "list_all_product_types" => self.list_product_types().await,
            "transfer_to_human_agents" => Ok(json!("Transfer successful")),
            _ => unreachable!("validated by call"),
        }
    }

    async fn run_write(&mut self, name: &str, a: &Value) -> R<Value> {
        let s = |k: &str| st(&a[k]);
        let (item_ids, new_ids) = (strs(&a["item_ids"]), strs(&a["new_item_ids"]));
        match name {
            "cancel_pending_order" => {
                self.cancel_pending_order(&s("order_id"), &s("reason"))
                    .await
            }
            "exchange_delivered_order_items" => {
                self.exchange(&s("order_id"), item_ids, new_ids, &s("payment_method_id"))
                    .await
            }
            "modify_pending_order_address" => self.modify_order_address(&s("order_id"), a).await,
            "modify_pending_order_items" => {
                self.modify_items(&s("order_id"), item_ids, new_ids, &s("payment_method_id"))
                    .await
            }
            "modify_pending_order_payment" => {
                self.modify_payment(&s("order_id"), &s("payment_method_id"))
                    .await
            }
            "modify_user_address" => self.modify_user_address(&s("user_id"), a).await,
            "return_delivered_order_items" => {
                self.return_items(&s("order_id"), item_ids, &s("payment_method_id"))
                    .await
            }
            _ => unreachable!("validated by call"),
        }
    }

    /// The id of the one user that `matches`; an error if there is none or several.
    async fn find_user(&mut self, matches: impl Fn(&Value) -> bool) -> R<Value> {
        self.reads.insert("users/*".into());
        let ids: Vec<String> = self
            .all("users")
            .await?
            .into_iter()
            .filter(|(_, u)| matches(u))
            .map(|(id, _)| id)
            .collect();
        match ids.len() {
            1 => Ok(json!(ids[0])),
            0 => bad("User not found"),
            _ => bad("User lookup is ambiguous"),
        }
    }

    async fn find_user_by_name_zip(&mut self, first: &str, last: &str, zip: &str) -> R<Value> {
        let (f, l) = (fold(first), fold(last));
        self.find_user(|u| {
            fold(u["name"]["first_name"].as_str().unwrap_or("")) == f
                && fold(u["name"]["last_name"].as_str().unwrap_or("")) == l
                && u["address"]["zip"].as_str() == Some(zip)
        })
        .await
    }

    async fn find_user_by_email(&mut self, email: &str) -> R<Value> {
        let e = fold(email);
        self.find_user(|u| fold(u["email"].as_str().unwrap_or("")) == e)
            .await
    }

    async fn item_details(&mut self, item: &str) -> R<Value> {
        self.reads.insert("products/*".into());
        for (_, p) in self.all("products").await? {
            if let Some(v) = p["variants"].get(item) {
                return Ok(v.clone());
            }
        }
        bad("Item not found")
    }

    /// Python's `json.dumps` of `{product name: product id}`, sorted by name.
    async fn list_product_types(&mut self) -> R<Value> {
        self.reads.insert("products/*".into());
        let mut names = BTreeMap::new();
        for (_, p) in self.all("products").await? {
            names.insert(st(&p["name"]), st(&p["product_id"]));
        }
        let body: Vec<String> = names
            .iter()
            .map(|(k, v)| format!("{}: {}", py_json_str(k), py_json_str(v)))
            .collect();
        Ok(json!(format!("{{{}}}", body.join(", "))))
    }
}

/// `table[key]`, or the error `missing`.
fn member(table: &Value, key: &str, missing: &str) -> R<Value> {
    match table.get(key) {
        Some(v) => Ok(v.clone()),
        None => bad(missing),
    }
}
