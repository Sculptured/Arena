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

    /// Posts `body` to `/rest/<path>`; the answer. Twenty limits an API key to 100 requests a
    /// minute, so a 429 waits as long as it says and is sent again.
    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let at = format!("{}/rest/{path}", self.base);
        loop {
            let answer = self
                .client
                .post(&at)
                .bearer_auth(&self.key)
                .json(body)
                .send();
            let answer = answer.await?;
            let status = answer.status();
            let text = answer.text().await?;
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if status.as_u16() == 429 {
                let wait = v["retryAfterSeconds"].as_u64().unwrap_or(5).max(1);
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                continue;
            }
            if !status.is_success() {
                bail!("writing to {path}: {status}: {text}");
            }
            return Ok(v);
        }
    }

    /// Writes `body` to `/rest/<path>`; the new record's id, from `create<made>`.
    async fn create(&self, path: &str, made: &str, body: &Value) -> Result<String> {
        let v = self.post(path, body).await?;
        let id = v["data"][format!("create{made}")]["id"].as_str();
        id.map(str::to_string)
            .ok_or_else(|| anyhow!("no id in {v}"))
    }

    /// Writes `bodies` through `/rest/batch/<path>`, [`BATCH`] to a request.
    async fn create_all(&self, path: &str, bodies: &[Value]) -> Result<()> {
        for chunk in bodies.chunks(BATCH) {
            self.post(&format!("batch/{path}"), &json!(chunk)).await?;
        }
        Ok(())
    }
}

/// The most records Twenty's batch endpoint takes in one request.
const BATCH: usize = 60;

/// The account of customer `name`, with what the CRM keeps about the company when the data says.
pub fn company(name: &str, account: &Value) -> Value {
    let mut c = json!({"name": name, "domainName": {"primaryLinkUrl": account["domain"]}});
    if let Some(m) = account["revenue_musd"].as_u64() {
        c["annualRevenue"] = json!({"amountMicros": m * 1_000_000_000_000, "currencyCode": "USD"});
    }
    if let Some(domain) = account["domain"].as_str() {
        let handle = domain
            .trim_start_matches("https://")
            .trim_end_matches(".example");
        c["linkedinLink"] = json!({"primaryLinkUrl": format!("https://linkedin.example/{handle}")});
    }
    if let Some(city) = account["city"].as_str() {
        c["address"] = json!({"addressCity": city, "addressState": account["state"],
                              "addressCountry": "United States"});
    }
    c
}

/// Writes one customer's account, its contacts and an opportunity per order.
async fn seed_account(crm: &Crm, c: &Company<'_>, customer: &Value) -> Result<()> {
    let name = customer["name"].as_str().unwrap_or_default();
    let account = &c.extra["accounts"][name];
    if account.is_null() {
        bail!("data/company.json has no account for {name}");
    }
    let id = crm
        .create("companies", "Company", &company(name, account))
        .await?;
    let people: Vec<Value> = list(account, "contacts").map(|p| person(p, &id)).collect();
    crm.create_all("people", &people).await?;
    let deals: Result<Vec<Value>> = list(c.data, "orders")
        .filter(|o| o["customer"] == name)
        .map(|o| opportunity(c, o, &id))
        .collect();
    crm.create_all("opportunities", &deals?).await
}

/// Writes the accounts, contacts and opportunities (`info` is the part's `connect` info).
pub async fn seed(info: &Value, c: &Company<'_>) -> Result<()> {
    let crm = Crm::connect(info)?;
    for customer in list(c.data, "customers") {
        seed_account(&crm, c, customer).await?;
    }
    Ok(())
}
