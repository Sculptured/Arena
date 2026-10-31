//! Retail tools on Postgres: a port of `cs2680-gap-tests/harness/tool.py` (`Retail` and `call`),
//! which was verified to reproduce tau2-bench's gold databases. Each call runs in one
//! transaction; rows are `SELECT doc ... FOR UPDATE`d (write tools), mutated as JSON, and
//! written back only when the tool succeeds.

#![allow(clippy::option_map_unit_fn)]

use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::Result;
use serde_json::{json, Value};
use tokio_postgres::{Client, Transaction};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

pub const READ_TOOLS: &[&str] = &[
    "calculate",
    "find_user_id_by_name_zip",
    "find_user_id_by_email",
    "get_order_details",
    "get_product_details",
    "get_item_details",
    "get_user_details",
    "list_all_product_types",
    "transfer_to_human_agents",
];
pub const WRITE_TOOLS: &[&str] = &[
    "cancel_pending_order",
    "exchange_delivered_order_items",
    "modify_pending_order_address",
    "modify_pending_order_items",
    "modify_pending_order_payment",
    "modify_user_address",
    "return_delivered_order_items",
];

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub result: Option<Value>,
    pub error: Option<String>,
    /// Sorted.
    pub reads: Vec<String>,
    /// Sorted.
    pub writes: Vec<String>,
}

#[derive(Clone, Copy)]
enum Ty {
    Str,
    List,
}

const ADDR: &[(&str, Ty)] = &[
    ("address1", Ty::Str),
    ("address2", Ty::Str),
    ("city", Ty::Str),
    ("state", Ty::Str),
    ("country", Ty::Str),
    ("zip", Ty::Str),
];

fn spec(name: &str) -> Option<(&'static str, Vec<(&'static str, Ty)>)> {
    use Ty::*;
    let order_items = vec![
        ("order_id", Str),
        ("item_ids", List),
        ("new_item_ids", List),
        ("payment_method_id", Str),
    ];
    let with_addr = |first: (&'static str, Ty)| {
        let mut v = vec![first];
        v.extend_from_slice(ADDR);
        v
    };
    Some(match name {
        "calculate" => (
            "Evaluate an arithmetic expression and return the result rounded to 2 decimals.",
            vec![("expression", Str)],
        ),
        "find_user_id_by_name_zip" => (
            "Find a user id by first name, last name and zip code.",
            vec![("first_name", Str), ("last_name", Str), ("zip", Str)],
        ),
        "find_user_id_by_email" => ("Find a user id by email address.", vec![("email", Str)]),
        "get_order_details" => (
            "Get the details of an order (order ids start with '#').",
            vec![("order_id", Str)],
        ),
        "get_product_details" => (
            "Get the details of a product (product ids differ from item ids).",
            vec![("product_id", Str)],
        ),
        "get_item_details" => (
            "Get the details of a product item (variant) by item id.",
            vec![("item_id", Str)],
        ),
        "get_user_details" => (
            "Get the details of a user, including payment methods and orders.",
            vec![("user_id", Str)],
        ),
        "list_all_product_types" => ("List every product type with its product id.", vec![]),
        "transfer_to_human_agents" => (
            "Transfer the customer to a human agent with a summary of the issue.",
            vec![("summary", Str)],
        ),
        "cancel_pending_order" => (
            "Cancel a pending order; reason is 'no longer needed' or 'ordered by mistake'.",
            vec![("order_id", Str), ("reason", Str)],
        ),
        "exchange_delivered_order_items" => (
            "Request an exchange of delivered order items for new items of the same product.",
            order_items,
        ),
        "modify_pending_order_address" => (
            "Change the shipping address of a pending order.",
            with_addr(("order_id", Str)),
        ),
        "modify_pending_order_items" => (
            "Replace items of a pending order with other variants of the same product (one-time).",
            order_items,
        ),
        "modify_pending_order_payment" => (
            "Change the payment method of a pending order.",
            vec![("order_id", Str), ("payment_method_id", Str)],
        ),
        "modify_user_address" => (
            "Change the default address of a user.",
            with_addr(("user_id", Str)),
        ),
        "return_delivered_order_items" => (
            "Request a return of delivered order items.",
            vec![
                ("order_id", Str),
                ("item_ids", List),
                ("payment_method_id", Str),
            ],
        ),
        _ => return None,
    })
}

/// MCP-style tool entries `{name, description, inputSchema}`.
pub fn tool_schemas() -> Vec<Value> {
    READ_TOOLS
        .iter()
        .chain(WRITE_TOOLS)
        .map(|name| {
            let (desc, args) = spec(name).expect("every tool has a spec");
            let mut props = serde_json::Map::new();
            for (a, ty) in &args {
                props.insert(
                    a.to_string(),
                    match ty {
                        Ty::Str => json!({"type": "string"}),
                        Ty::List => json!({"type": "array", "items": {"type": "string"}}),
                    },
                );
            }
            let required: Vec<&str> = args.iter().map(|(a, _)| *a).collect();
            json!({"name": name, "description": desc,
                   "inputSchema": {"type": "object", "properties": props, "required": required}})
        })
        .collect()
}

enum Fail {
    Tool(String),
    Db(anyhow::Error),
}
impl From<tokio_postgres::Error> for Fail {
    fn from(e: tokio_postgres::Error) -> Self {
        Fail::Db(e.into())
    }
}
fn bad<T>(msg: impl Into<String>) -> Result<T, Fail> {
    Err(Fail::Tool(msg.into()))
}
type R<T> = Result<T, Fail>;

pub fn fold(text: &str) -> String {
    text.trim()
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

fn round2(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}
fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}
fn st(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}
fn is_gift_card(method: &Value) -> bool {
    method["source"] == "gift_card"
}
fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(st).collect())
        .unwrap_or_default()
}
fn count(v: &[String], x: &str) -> usize {
    v.iter().filter(|y| *y == x).count()
}
/// `round(0, 2)` and friends stay integers in Python when nothing was added.
fn money(x: f64, any: bool) -> Value {
    if any {
        json!(x)
    } else {
        json!(0)
    }
}

struct Retail<'a> {
    tx: &'a Transaction<'a>,
    lock: bool,
    cache: HashMap<String, Value>,
    all_loaded: HashSet<&'static str>,
    reads: BTreeSet<String>,
    writes: BTreeSet<String>,
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

    fn put(&mut self, table: &str, id: &str, doc: Value) {
        self.cache.insert(format!("{table}/{id}"), doc);
    }

    async fn order(&mut self, id: &str) -> R<Value> {
        self.reads.insert(format!("orders/{id}"));
        match self.row("orders", id).await? {
            Some(v) => Ok(v),
            None => bad("Order not found"),
        }
    }
    async fn user(&mut self, id: &str) -> R<Value> {
        self.reads.insert(format!("users/{id}"));
        match self.row("users", id).await? {
            Some(v) => Ok(v),
            None => bad("User not found"),
        }
    }
    async fn product(&mut self, id: &str) -> R<Value> {
        self.reads.insert(format!("products/{id}"));
        match self.row("products", id).await? {
            Some(v) => Ok(v),
            None => bad("Product not found"),
        }
    }
    async fn variant(&mut self, product_id: &str, variant_id: &str) -> R<Value> {
        let product = self.product(product_id).await?;
        match product["variants"].get(variant_id) {
            Some(v) => Ok(v.clone()),
            None => bad("Variant not found"),
        }
    }
    async fn payment(&mut self, user_id: &str, pm: &str) -> R<Value> {
        let user = self.user(user_id).await?;
        match user["payment_methods"].get(pm) {
            Some(v) => Ok(v.clone()),
            None => bad("Payment method not found"),
        }
    }
    /// In-place balance update on the (cached) user, as Python mutates the method dict.
    fn set_balance(&mut self, user_id: &str, pm: &str, balance: f64) {
        let key = format!("users/{user_id}");
        if let Some(u) = self.cache.get_mut(&key) {
            u["payment_methods"][pm]["balance"] = json!(balance);
        }
        self.writes.insert(key);
    }

    async fn run(&mut self, name: &str, a: &Value) -> R<Value> {
        let s = |k: &str| st(&a[k]);
        match name {
            "calculate" => self.calculate(&s("expression")),
            "find_user_id_by_name_zip" => {
                self.reads.insert("users/*".into());
                let (f, l, z) = (fold(&s("first_name")), fold(&s("last_name")), s("zip"));
                let m: Vec<String> = self
                    .all("users")
                    .await?
                    .into_iter()
                    .filter(|(_, u)| {
                        fold(u["name"]["first_name"].as_str().unwrap_or("")) == f
                            && fold(u["name"]["last_name"].as_str().unwrap_or("")) == l
                            && u["address"]["zip"].as_str() == Some(z.as_str())
                    })
                    .map(|(id, _)| id)
                    .collect();
                single(m)
            }
            "find_user_id_by_email" => {
                self.reads.insert("users/*".into());
                let e = fold(&s("email"));
                let m: Vec<String> = self
                    .all("users")
                    .await?
                    .into_iter()
                    .filter(|(_, u)| fold(u["email"].as_str().unwrap_or("")) == e)
                    .map(|(id, _)| id)
                    .collect();
                single(m)
            }
            "get_order_details" => self.order(&s("order_id")).await,
            "get_product_details" => self.product(&s("product_id")).await,
            "get_item_details" => {
                self.reads.insert("products/*".into());
                let item = s("item_id");
                for (_, p) in self.all("products").await? {
                    if let Some(v) = p["variants"].get(&item) {
                        return Ok(v.clone());
                    }
                }
                bad("Item not found")
            }
            "get_user_details" => self.user(&s("user_id")).await,
            "list_all_product_types" => {
                self.reads.insert("products/*".into());
                let mut names = std::collections::BTreeMap::new();
                for (_, p) in self.all("products").await? {
                    names.insert(st(&p["name"]), st(&p["product_id"]));
                }
                let body: Vec<String> = names
                    .iter()
                    .map(|(k, v)| format!("{}: {}", py_json_str(k), py_json_str(v)))
                    .collect();
                Ok(json!(format!("{{{}}}", body.join(", "))))
            }
            "transfer_to_human_agents" => Ok(json!("Transfer successful")),
            "cancel_pending_order" => {
                self.cancel_pending_order(&s("order_id"), &s("reason"))
                    .await
            }
            "exchange_delivered_order_items" => {
                self.exchange(
                    &s("order_id"),
                    strs(&a["item_ids"]),
                    strs(&a["new_item_ids"]),
                    &s("payment_method_id"),
                )
                .await
            }
            "modify_pending_order_address" => {
                let mut order = self.order(&s("order_id")).await?;
                if !st(&order["status"]).contains("pending") {
                    return bad("Non-pending order cannot be modified");
                }
                set_address(&mut order, a);
                self.finish_order(&s("order_id"), order)
            }
            "modify_pending_order_items" => {
                self.modify_items(
                    &s("order_id"),
                    strs(&a["item_ids"]),
                    strs(&a["new_item_ids"]),
                    &s("payment_method_id"),
                )
                .await
            }
            "modify_pending_order_payment" => {
                self.modify_payment(&s("order_id"), &s("payment_method_id"))
                    .await
            }
            "modify_user_address" => {
                let mut user = self.user(&s("user_id")).await?;
                set_address(&mut user, a);
                self.put("users", &s("user_id"), user.clone());
                self.writes.insert(format!("users/{}", s("user_id")));
                Ok(user)
            }
            "return_delivered_order_items" => {
                let oid = s("order_id");
                let pm = s("payment_method_id");
                let item_ids = strs(&a["item_ids"]);
                let mut order = self.order(&oid).await?;
                if order["status"] != "delivered" {
                    return bad("Non-delivered order cannot be returned");
                }
                let method = self.payment(&st(&order["user_id"]), &pm).await?;
                if !is_gift_card(&method)
                    && Value::String(pm.clone()) != order["payment_history"][0]["payment_method_id"]
                {
                    return bad("Payment method should be the original payment method");
                }
                let all_ids: Vec<String> = order["items"]
                    .as_array()
                    .map(|i| i.iter().map(|x| st(&x["item_id"])).collect())
                    .unwrap_or_default();
                for id in &item_ids {
                    if count(&item_ids, id) > count(&all_ids, id) {
                        return bad("Some item not found");
                    }
                }
                let mut sorted = item_ids.clone();
                sorted.sort();
                order["status"] = json!("return requested");
                order["return_items"] = json!(sorted);
                order["return_payment_method_id"] = json!(pm);
                self.finish_order(&oid, order)
            }
            _ => unreachable!("validated by call"),
        }
    }

    fn finish_order(&mut self, oid: &str, order: Value) -> R<Value> {
        self.put("orders", oid, order.clone());
        self.writes.insert(format!("orders/{oid}"));
        Ok(order)
    }

    fn calculate(&mut self, expr: &str) -> R<Value> {
        if !expr.chars().all(|c| "0123456789+-*/(). ".contains(c)) {
            return bad("Invalid characters in expression");
        }
        let mut p = Parser {
            s: expr.chars().filter(|c| *c != ' ').collect(),
            i: 0,
        };
        let v = p.expr()?;
        if p.i != p.s.len() || !v.is_finite() {
            return bad("Invalid expression");
        }
        Ok(json!(format!("{:?}", round2(v))))
    }

    async fn cancel_pending_order(&mut self, oid: &str, reason: &str) -> R<Value> {
        let mut order = self.order(oid).await?;
        if order["status"] != "pending" {
            return bad("Non-pending order cannot be cancelled");
        }
        if reason != "no longer needed" && reason != "ordered by mistake" {
            return bad("Invalid reason");
        }
        let uid = st(&order["user_id"]);
        let mut refunds = vec![];
        for payment in order["payment_history"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let pid = st(&payment["payment_method_id"]);
            refunds.push(json!({"transaction_type": "refund", "amount": payment["amount"], "payment_method_id": pid}));
            let method = self.payment(&uid, &pid).await?;
            if is_gift_card(&method) {
                self.set_balance(
                    &uid,
                    &pid,
                    round2(num(&method["balance"]) + num(&payment["amount"])),
                );
            }
        }
        order["status"] = json!("cancelled");
        order["cancel_reason"] = json!(reason);
        if let Some(h) = order["payment_history"].as_array_mut() {
            h.extend(refunds);
        }
        self.finish_order(oid, order)
    }

    async fn exchange(
        &mut self,
        oid: &str,
        item_ids: Vec<String>,
        new_ids: Vec<String>,
        pm: &str,
    ) -> R<Value> {
        let mut order = self.order(oid).await?;
        if order["status"] != "delivered" {
            return bad("Non-delivered order cannot be exchanged");
        }
        let items = order["items"].as_array().cloned().unwrap_or_default();
        let all_ids: Vec<String> = items.iter().map(|i| st(&i["item_id"])).collect();
        for id in &item_ids {
            if count(&item_ids, id) > count(&all_ids, id) {
                return bad(format!("Number of {id} not found."));
            }
        }
        if item_ids.len() != new_ids.len() {
            return bad("The number of items to be exchanged should match.");
        }
        let mut diff = 0.0;
        for (id, new_id) in item_ids.iter().zip(&new_ids) {
            let Some(item) = items.iter().find(|i| i["item_id"] == id.as_str()) else {
                return bad(format!("Item {id} not found"));
            };
            let variant = self.variant(&st(&item["product_id"]), new_id).await?;
            if !variant["available"].as_bool().unwrap_or(false) {
                return bad(format!("New item {new_id} not found or available"));
            }
            diff += num(&variant["price"]) - num(&item["price"]);
        }
        let diff_v = money(round2(diff), !item_ids.is_empty());
        let diff = round2(diff);
        let method = self.payment(&st(&order["user_id"]), pm).await?;
        if is_gift_card(&method) && num(&method["balance"]) < diff {
            return bad("Insufficient gift card balance to pay for the price difference");
        }
        let (mut a, mut b) = (item_ids.clone(), new_ids.clone());
        a.sort();
        b.sort();
        order["status"] = json!("exchange requested");
        order["exchange_items"] = json!(a);
        order["exchange_new_items"] = json!(b);
        order["exchange_payment_method_id"] = json!(pm);
        order["exchange_price_difference"] = diff_v;
        self.finish_order(oid, order)
    }

    async fn modify_items(
        &mut self,
        oid: &str,
        item_ids: Vec<String>,
        new_ids: Vec<String>,
        pm: &str,
    ) -> R<Value> {
        let mut order = self.order(oid).await?;
        if order["status"] != "pending" {
            return bad("Non-pending order cannot be modified");
        }
        let items = order["items"].as_array().cloned().unwrap_or_default();
        let all_ids: Vec<String> = items.iter().map(|i| st(&i["item_id"])).collect();
        for id in &item_ids {
            if count(&item_ids, id) > count(&all_ids, id) {
                return bad(format!("{id} not found"));
            }
        }
        if item_ids.len() != new_ids.len() {
            return bad("The number of items to be exchanged should match");
        }
        let mut diff = 0.0;
        let mut variant: Option<Value> = None;
        for (id, new_id) in item_ids.iter().zip(&new_ids) {
            if id == new_id {
                return bad("The new item id should be different from the old item id");
            }
            let Some(item) = items.iter().find(|i| i["item_id"] == id.as_str()) else {
                return bad(format!("Item {id} not found"));
            };
            let v = self.variant(&st(&item["product_id"]), new_id).await?;
            if !v["available"].as_bool().unwrap_or(false) {
                return bad(format!("New item {new_id} not found or available"));
            }
            diff += num(&v["price"]) - num(&item["price"]);
            variant = Some(v);
        }
        let uid = st(&order["user_id"]);
        let method = self.payment(&uid, pm).await?;
        let gift = is_gift_card(&method);
        if gift && num(&method["balance"]) < diff {
            return bad("Insufficient gift card balance to pay for the new item");
        }
        let any = !item_ids.is_empty();
        order["payment_history"].as_array_mut().map(|h| {
            h.push(json!({
                "transaction_type": if diff > 0.0 { "payment" } else { "refund" },
                "amount": money(diff.abs(), any), "payment_method_id": pm,
            }))
        });
        if gift {
            self.set_balance(&uid, pm, round2(num(&method["balance"]) - diff));
        }
        // Mirrors upstream, which applies the last variant's price and options to every item.
        if let Some(variant) = variant {
            for (id, new_id) in item_ids.iter().zip(&new_ids) {
                if let Some(item) = order["items"]
                    .as_array_mut()
                    .and_then(|a| a.iter_mut().find(|i| i["item_id"] == id.as_str()))
                {
                    item["item_id"] = json!(new_id);
                    item["price"] = variant["price"].clone();
                    item["options"] = variant["options"].clone();
                }
            }
        }
        order["status"] = json!("pending (item modified)");
        self.finish_order(oid, order)
    }

    async fn modify_payment(&mut self, oid: &str, pm: &str) -> R<Value> {
        let mut order = self.order(oid).await?;
        if !st(&order["status"]).contains("pending") {
            return bad("Non-pending order cannot be modified");
        }
        let uid = st(&order["user_id"]);
        let method = self.payment(&uid, pm).await?;
        let history = order["payment_history"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if history.len() != 1 || history[0]["transaction_type"] != "payment" {
            return bad("There should be exactly one payment for a pending order");
        }
        if history[0]["payment_method_id"] == pm {
            return bad("The new payment method should be different from the current one");
        }
        let amount = num(&history[0]["amount"]);
        if is_gift_card(&method) && num(&method["balance"]) < amount {
            return bad("Insufficient gift card balance to pay for the order");
        }
        let old_id = st(&history[0]["payment_method_id"]);
        if let Some(h) = order["payment_history"].as_array_mut() {
            h.push(json!({"transaction_type": "payment", "amount": history[0]["amount"], "payment_method_id": pm}));
            h.push(json!({"transaction_type": "refund", "amount": history[0]["amount"], "payment_method_id": old_id}));
        }
        if is_gift_card(&method) {
            self.set_balance(&uid, pm, round2(num(&method["balance"]) - amount));
        }
        let old_method = self.payment(&uid, &old_id).await?;
        if is_gift_card(&old_method) {
            self.set_balance(&uid, &old_id, round2(num(&old_method["balance"]) + amount));
        }
        self.finish_order(oid, order)
    }
}

fn single(m: Vec<String>) -> R<Value> {
    match m.len() {
        1 => Ok(json!(m[0])),
        0 => bad("User not found"),
        _ => bad("User lookup is ambiguous"),
    }
}

fn set_address(target: &mut Value, a: &Value) {
    target["address"] = json!({"address1": a["address1"], "address2": a["address2"], "city": a["city"],
                               "country": a["country"], "state": a["state"], "zip": a["zip"]});
}

/// `json.dumps(s)` with Python's default `ensure_ascii`.
fn py_json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut b = [0u16; 2];
                for u in c.encode_utf16(&mut b) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Arithmetic as Python would evaluate it for the characters `calculate` allows.
struct Parser {
    s: Vec<char>,
    i: usize,
}
impl Parser {
    fn peek(&self, off: usize) -> Option<char> {
        self.s.get(self.i + off).copied()
    }
    fn expr(&mut self) -> R<f64> {
        let mut v = self.term()?;
        while let Some(c @ ('+' | '-')) = self.peek(0) {
            self.i += 1;
            let r = self.term()?;
            v = if c == '+' { v + r } else { v - r };
        }
        Ok(v)
    }
    fn term(&mut self) -> R<f64> {
        let mut v = self.unary()?;
        loop {
            match (self.peek(0), self.peek(1)) {
                (Some('/'), Some('/')) => {
                    self.i += 2;
                    let r = self.unary()?;
                    if r == 0.0 {
                        return bad("division by zero");
                    }
                    v = (v / r).floor();
                }
                (Some('/'), _) => {
                    self.i += 1;
                    let r = self.unary()?;
                    if r == 0.0 {
                        return bad("division by zero");
                    }
                    v /= r;
                }
                (Some('*'), next) if next != Some('*') => {
                    self.i += 1;
                    v *= self.unary()?;
                }
                _ => return Ok(v),
            }
        }
    }
    fn unary(&mut self) -> R<f64> {
        match self.peek(0) {
            Some('-') => {
                self.i += 1;
                Ok(-self.unary()?)
            }
            Some('+') => {
                self.i += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }
    fn power(&mut self) -> R<f64> {
        let base = self.atom()?;
        if self.peek(0) == Some('*') && self.peek(1) == Some('*') {
            self.i += 2;
            let e = self.unary()?;
            return Ok(base.powf(e));
        }
        Ok(base)
    }
    fn atom(&mut self) -> R<f64> {
        match self.peek(0) {
            Some('(') => {
                self.i += 1;
                let v = self.expr()?;
                if self.peek(0) != Some(')') {
                    return bad("Invalid expression");
                }
                self.i += 1;
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() || c == '.' => {
                let start = self.i;
                while self.peek(0).is_some_and(|c| c.is_ascii_digit() || c == '.') {
                    self.i += 1;
                }
                let t: String = self.s[start..self.i].iter().collect();
                t.parse().or_else(|_| bad("Invalid expression"))
            }
            _ => bad("Invalid expression"),
        }
    }
}

/// Runs one tool in one transaction. `Err` only for database failures; tool failures,
/// unknown tools and bad arguments come back in `ToolOutcome.error`.
pub async fn call(client: &mut Client, name: &str, args: &Value) -> Result<ToolOutcome> {
    let fail = |msg: String| ToolOutcome {
        result: None,
        error: Some(msg),
        reads: vec![],
        writes: vec![],
    };
    let Some((_, params)) = spec(name) else {
        return Ok(fail(format!("Unknown tool {name}")));
    };
    let Some(obj) = args.as_object() else {
        return Ok(fail(format!("{name}() arguments must be a JSON object")));
    };
    for (k, ty) in &params {
        match (obj.get(*k), ty) {
            (None, _) => return Ok(fail(format!("{name}() missing required argument: '{k}'"))),
            (Some(Value::String(_)), Ty::Str) => {}
            (Some(Value::Array(a)), Ty::List) if a.iter().all(Value::is_string) => {}
            (Some(_), Ty::Str) => {
                return Ok(fail(format!("{name}() argument '{k}' must be a string")))
            }
            (Some(_), Ty::List) => {
                return Ok(fail(format!(
                    "{name}() argument '{k}' must be a list of strings"
                )))
            }
        }
    }
    if let Some(k) = obj.keys().find(|k| !params.iter().any(|(p, _)| p == k)) {
        return Ok(fail(format!(
            "{name}() got an unexpected keyword argument '{k}'"
        )));
    }

    let tx = client.transaction().await?;
    let mut retail = Retail {
        tx: &tx,
        lock: WRITE_TOOLS.contains(&name),
        cache: HashMap::new(),
        all_loaded: HashSet::new(),
        reads: BTreeSet::new(),
        writes: BTreeSet::new(),
    };
    let out = retail.run(name, args).await;
    let (reads, writes) = (
        retail.reads.iter().cloned().collect(),
        retail.writes.iter().cloned().collect(),
    );
    match out {
        Ok(result) => {
            for key in &retail.writes {
                let (table, id) = key.split_once('/').expect("write keys are table/id");
                let doc = &retail.cache[key];
                tx.execute(
                    &format!("UPDATE {table} SET doc = $1 WHERE id = $2"),
                    &[doc, &id],
                )
                .await?;
            }
            tx.commit().await?;
            Ok(ToolOutcome {
                result: Some(result),
                error: None,
                reads,
                writes,
            })
        }
        Err(Fail::Tool(msg)) => {
            // Nothing is persisted on failure (as tool.py's main); sets report what was attempted.
            tx.rollback().await?;
            Ok(ToolOutcome {
                result: None,
                error: Some(msg),
                reads,
                writes,
            })
        }
        Err(Fail::Db(e)) => Err(e),
    }
}
