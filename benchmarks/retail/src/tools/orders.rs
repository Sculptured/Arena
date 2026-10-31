//! The tools that change orders and users, and the checks they share.

use serde_json::{json, Value};

use super::fail::{bad, R};
use super::retail::Retail;
use super::value::{count, is_gift_card, money, num, round2, st};

/// The `items` of an order.
fn items_of(order: &Value) -> Vec<Value> {
    order["items"].as_array().cloned().unwrap_or_default()
}

fn item_ids_of(items: &[Value]) -> Vec<String> {
    items.iter().map(|i| st(&i["item_id"])).collect()
}

/// Errors with `msg(id)` for the first id asked for more often than the order has it.
fn check_counts(asked: &[String], items: &[Value], msg: impl Fn(&str) -> String) -> R<()> {
    let have = item_ids_of(items);
    match asked.iter().find(|id| count(asked, id) > count(&have, id)) {
        Some(id) => bad(msg(id)),
        None => Ok(()),
    }
}

fn sorted(v: &[String]) -> Vec<String> {
    let mut v = v.to_vec();
    v.sort();
    v
}

fn set_address(target: &mut Value, a: &Value) {
    target["address"] = json!({"address1": a["address1"], "address2": a["address2"], "city": a["city"],
                               "country": a["country"], "state": a["state"], "zip": a["zip"]});
}

fn push_payment(order: &mut Value, entry: Value) {
    if let Some(h) = order["payment_history"].as_array_mut() {
        h.push(entry);
    }
}

fn history_entry(kind: &str, amount: Value, pm: &str) -> Value {
    json!({"transaction_type": kind, "amount": amount, "payment_method_id": pm})
}

/// What swapping items for new variants comes to.
struct Swaps {
    /// Sum of the price differences.
    diff: f64,
    /// The last new variant, whose price and options upstream applies to every item.
    last_variant: Option<Value>,
}

impl Retail<'_> {
    fn finish_order(&mut self, oid: &str, order: Value) -> R<Value> {
        self.put("orders", oid, order.clone());
        self.writes.insert(format!("orders/{oid}"));
        Ok(order)
    }

    pub(super) async fn modify_order_address(&mut self, oid: &str, a: &Value) -> R<Value> {
        let mut order = self.order(oid).await?;
        if !st(&order["status"]).contains("pending") {
            return bad("Non-pending order cannot be modified");
        }
        set_address(&mut order, a);
        self.finish_order(oid, order)
    }

    pub(super) async fn modify_user_address(&mut self, uid: &str, a: &Value) -> R<Value> {
        let mut user = self.user(uid).await?;
        set_address(&mut user, a);
        self.put("users", uid, user.clone());
        self.writes.insert(format!("users/{uid}"));
        Ok(user)
    }

    pub(super) async fn return_items(
        &mut self,
        oid: &str,
        item_ids: Vec<String>,
        pm: &str,
    ) -> R<Value> {
        let mut order = self.order(oid).await?;
        if order["status"] != "delivered" {
            return bad("Non-delivered order cannot be returned");
        }
        let method = self.payment(&st(&order["user_id"]), pm).await?;
        if !is_gift_card(&method)
            && order["payment_history"][0]["payment_method_id"] != Value::String(pm.into())
        {
            return bad("Payment method should be the original payment method");
        }
        check_counts(&item_ids, &items_of(&order), |_| {
            "Some item not found".into()
        })?;
        order["status"] = json!("return requested");
        order["return_items"] = json!(sorted(&item_ids));
        order["return_payment_method_id"] = json!(pm);
        self.finish_order(oid, order)
    }

    pub(super) async fn cancel_pending_order(&mut self, oid: &str, reason: &str) -> R<Value> {
        let mut order = self.order(oid).await?;
        if order["status"] != "pending" {
            return bad("Non-pending order cannot be cancelled");
        }
        if reason != "no longer needed" && reason != "ordered by mistake" {
            return bad("Invalid reason");
        }
        let refunds = self.refund_payments(&order).await?;
        order["status"] = json!("cancelled");
        order["cancel_reason"] = json!(reason);
        if let Some(h) = order["payment_history"].as_array_mut() {
            h.extend(refunds);
        }
        self.finish_order(oid, order)
    }

    /// Refund entries for every payment of the order; gift cards get their money back.
    async fn refund_payments(&mut self, order: &Value) -> R<Vec<Value>> {
        let uid = st(&order["user_id"]);
        let mut refunds = vec![];
        for payment in order["payment_history"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let pid = st(&payment["payment_method_id"]);
            refunds.push(history_entry("refund", payment["amount"].clone(), &pid));
            let method = self.payment(&uid, &pid).await?;
            if is_gift_card(&method) {
                let balance = round2(num(&method["balance"]) + num(&payment["amount"]));
                self.set_balance(&uid, &pid, balance);
            }
        }
        Ok(refunds)
    }

    /// Looks up each new variant (it must be available) and sums the price differences.
    async fn price_swaps(
        &mut self,
        items: &[Value],
        item_ids: &[String],
        new_ids: &[String],
        reject_same: bool,
    ) -> R<Swaps> {
        let mut swaps = Swaps {
            diff: 0.0,
            last_variant: None,
        };
        for (id, new_id) in item_ids.iter().zip(new_ids) {
            if reject_same && id == new_id {
                return bad("The new item id should be different from the old item id");
            }
            let Some(item) = items.iter().find(|i| i["item_id"] == id.as_str()) else {
                return bad(format!("Item {id} not found"));
            };
            let variant = self.variant(&st(&item["product_id"]), new_id).await?;
            if !variant["available"].as_bool().unwrap_or(false) {
                return bad(format!("New item {new_id} not found or available"));
            }
            swaps.diff += num(&variant["price"]) - num(&item["price"]);
            swaps.last_variant = Some(variant);
        }
        Ok(swaps)
    }

    pub(super) async fn exchange(
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
        let items = items_of(&order);
        check_counts(&item_ids, &items, |id| format!("Number of {id} not found."))?;
        if item_ids.len() != new_ids.len() {
            return bad("The number of items to be exchanged should match.");
        }
        let swaps = self.price_swaps(&items, &item_ids, &new_ids, false).await?;
        let diff_v = money(round2(swaps.diff), !item_ids.is_empty());
        let diff = round2(swaps.diff);
        let method = self.payment(&st(&order["user_id"]), pm).await?;
        if is_gift_card(&method) && num(&method["balance"]) < diff {
            return bad("Insufficient gift card balance to pay for the price difference");
        }
        order["status"] = json!("exchange requested");
        order["exchange_items"] = json!(sorted(&item_ids));
        order["exchange_new_items"] = json!(sorted(&new_ids));
        order["exchange_payment_method_id"] = json!(pm);
        order["exchange_price_difference"] = diff_v;
        self.finish_order(oid, order)
    }

    pub(super) async fn modify_items(
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
        let items = items_of(&order);
        check_counts(&item_ids, &items, |id| format!("{id} not found"))?;
        if item_ids.len() != new_ids.len() {
            return bad("The number of items to be exchanged should match");
        }
        let swaps = self.price_swaps(&items, &item_ids, &new_ids, true).await?;
        self.charge_difference(&mut order, pm, swaps.diff, !item_ids.is_empty())
            .await?;
        if let Some(variant) = swaps.last_variant {
            apply_variant(&mut order, &item_ids, &new_ids, &variant);
        }
        order["status"] = json!("pending (item modified)");
        self.finish_order(oid, order)
    }

    /// Records the price difference of modified items as a payment or refund; gift cards pay.
    async fn charge_difference(
        &mut self,
        order: &mut Value,
        pm: &str,
        diff: f64,
        any: bool,
    ) -> R<()> {
        let uid = st(&order["user_id"]);
        let method = self.payment(&uid, pm).await?;
        let gift = is_gift_card(&method);
        if gift && num(&method["balance"]) < diff {
            return bad("Insufficient gift card balance to pay for the new item");
        }
        let kind = if diff > 0.0 { "payment" } else { "refund" };
        push_payment(order, history_entry(kind, money(diff.abs(), any), pm));
        if gift {
            self.set_balance(&uid, pm, round2(num(&method["balance"]) - diff));
        }
        Ok(())
    }

    pub(super) async fn modify_payment(&mut self, oid: &str, pm: &str) -> R<Value> {
        let mut order = self.order(oid).await?;
        if !st(&order["status"]).contains("pending") {
            return bad("Non-pending order cannot be modified");
        }
        let uid = st(&order["user_id"]);
        let method = self.payment(&uid, pm).await?;
        let paid = single_payment(&order, pm)?;
        let amount = num(&paid["amount"]);
        if is_gift_card(&method) && num(&method["balance"]) < amount {
            return bad("Insufficient gift card balance to pay for the order");
        }
        let old_id = st(&paid["payment_method_id"]);
        push_payment(
            &mut order,
            history_entry("payment", paid["amount"].clone(), pm),
        );
        push_payment(
            &mut order,
            history_entry("refund", paid["amount"].clone(), &old_id),
        );
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

/// The one payment of a pending order, if it is not already made with `pm`.
fn single_payment(order: &Value, pm: &str) -> R<Value> {
    let history = order["payment_history"]
        .as_array()
        .map_or(&[][..], |h| h.as_slice());
    if history.len() != 1 || history[0]["transaction_type"] != "payment" {
        return bad("There should be exactly one payment for a pending order");
    }
    if history[0]["payment_method_id"] == pm {
        return bad("The new payment method should be different from the current one");
    }
    Ok(history[0].clone())
}

/// Mirrors upstream, which applies the last variant's price and options to every item.
fn apply_variant(order: &mut Value, item_ids: &[String], new_ids: &[String], variant: &Value) {
    for (id, new_id) in item_ids.iter().zip(new_ids) {
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
