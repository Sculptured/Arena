//! The CRM's starting records: the same customers as the ERP, as accounts with their contacts, and
//! an open opportunity per sales order, due when the order is. An opportunity's name starts with
//! the order's id, by which a notice about the order finds it.
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::company::{list, Company};

/// The opportunity of a sales order.
pub fn opportunity_name(order: &Value) -> String {
    format!(
        "{} {} x {}",
        order["id"].as_str().unwrap_or_default(),
        order["item"].as_str().unwrap_or_default(),
        order["qty"]
    )
}

fn price_of<'a>(data: &'a Value, code: &Value) -> Result<&'a Value> {
    list(data, "items")
        .find(|i| i["code"] == *code)
        .map(|i| &i["price"])
        .ok_or_else(|| anyhow!("the data has no item {code}"))
}

/// `order` as an opportunity of the account `account_id`.
pub fn opportunity(c: &Company, order: &Value, account_id: &str) -> Result<Value> {
    let price = price_of(c.data, &order["item"])?.as_f64().unwrap_or(0.0);
    let qty = order["qty"].as_f64().unwrap_or(0.0);
    let micros = (qty * price * 1e6).round() as u64;
    Ok(
        json!({"name": opportunity_name(order), "stage": "PROPOSAL", "companyId": account_id,
              "closeDate": c.due(order).iso(),
              "amount": {"amountMicros": micros, "currencyCode": c.data["currency"]}}),
    )
}

/// `contact` as a person at the account `account_id`.
pub fn person(contact: &Value, account_id: &str) -> Value {
    json!({"name": {"firstName": contact["first"], "lastName": contact["last"]},
           "emails": {"primaryEmail": contact["email"]}, "jobTitle": contact["title"],
           "companyId": account_id})
}

struct Crm {
    client: reqwest::Client,
    base: String,
    key: String,
}

impl Crm {
    fn connect(info: &Value) -> Result<Crm> {
        let text = |name: &str| {
            info[name]
                .as_str()
                .ok_or_else(|| anyhow!("the crm part's connect info has no {name}"))
        };
        Ok(Crm {
            client: reqwest::Client::new(),
            base: text("url")?.trim_end_matches('/').to_string(),
            key: text("api_key")?.to_string(),
        })
    }

    /// Writes `body` to `/rest/<path>`; the new record's id, from `create<made>`.
    async fn create(&self, path: &str, made: &str, body: &Value) -> Result<String> {
        let at = format!("{}/rest/{path}", self.base);
        let answer = self
            .client
            .post(at)
            .bearer_auth(&self.key)
            .json(body)
            .send()
            .await?;
        let status = answer.status();
        let text = answer.text().await?;
        if !status.is_success() {
            bail!("writing {body}: {status}: {text}");
        }
        let v: Value = serde_json::from_str(&text)?;
        let id = v["data"][format!("create{made}")]["id"].as_str();
        id.map(str::to_string)
            .ok_or_else(|| anyhow!("no id in {text}"))
    }
}

/// Writes the accounts, contacts and opportunities (`info` is the part's `connect` info).
pub async fn seed(info: &Value, c: &Company<'_>) -> Result<()> {
    let crm = Crm::connect(info)?;
    for customer in list(c.data, "customers") {
        let name = customer["name"].as_str().unwrap_or_default();
        let account = &c.extra["accounts"][name];
        if account.is_null() {
            bail!("data/company.json has no account for {name}");
        }
        let company = json!({"name": name,
                             "domainName": {"primaryLinkUrl": account["domain"]}});
        let id = crm.create("companies", "Company", &company).await?;
        for contact in list(account, "contacts") {
            crm.create("people", "Person", &person(contact, &id))
                .await?;
        }
        for order in list(c.data, "orders").filter(|o| o["customer"] == name) {
            let deal = opportunity(c, order, &id)?;
            crm.create("opportunities", "Opportunity", &deal).await?;
        }
    }
    Ok(())
}
