//! The CRM's starting records: the ERP's customers as accounts with their contacts, and an open
//! opportunity per sales order, closing when the order is due. An opportunity's name starts with
//! its order's customer PO number, by which the checks find it.
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::data::{Contact, Customer, Data, Order};

pub fn opportunity_name(o: &Order) -> String {
    format!("{} Corvexa x {}", o.po, o.qty)
}

pub fn opportunity(d: &Data, o: &Order, account: &str) -> Value {
    let micros = (o.qty as f64 * d.product.price * 1e6).round() as u64;
    json!({"name": opportunity_name(o), "stage": "PROPOSAL", "companyId": account, "closeDate": o.due,
           "amount": {"amountMicros": micros, "currencyCode": d.currency}})
}

pub fn person(c: &Contact, account: &str) -> Value {
    json!({"name": {"firstName": c.first, "lastName": c.last}, "emails": {"primaryEmail": c.email},
           "jobTitle": c.title, "companyId": account})
}

/// The account of a customer; the address names a state only where the customer has one.
pub fn company(c: &Customer) -> Value {
    let country = match c.region.as_str() {
        "US" => "United States",
        _ => c.country.as_deref().unwrap_or_default(),
    };
    let mut address = json!({"addressCity": c.city, "addressCountry": country});
    if let Some(state) = &c.state {
        address["addressState"] = json!(state);
    }
    json!({"name": c.name, "domainName": {"primaryLinkUrl": format!("https://{}", c.domain)},
           "address": address})
}

struct Crm {
    client: reqwest::Client,
    base: String,
    key: String,
}

/// The most records Twenty's batch endpoint takes in one request.
const BATCH: usize = 60;

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

    /// Posts `body` to `/rest/<path>`. Twenty limits an API key to 100 requests a minute, so a
    /// 429 waits as long as it says and is sent again.
    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let at = format!("{}/rest/{path}", self.base);
        loop {
            let answer = self
                .client
                .post(&at)
                .bearer_auth(&self.key)
                .json(body)
                .send()
                .await?;
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

    async fn create(&self, path: &str, made: &str, body: &Value) -> Result<String> {
        let v = self.post(path, body).await?;
        let id = v["data"][format!("create{made}")]["id"].as_str();
        id.map(str::to_string)
            .ok_or_else(|| anyhow!("no id in {v}"))
    }

    async fn create_all(&self, path: &str, bodies: &[Value]) -> Result<()> {
        for chunk in bodies.chunks(BATCH) {
            self.post(&format!("batch/{path}"), &json!(chunk)).await?;
        }
        Ok(())
    }
}

async fn seed_account(crm: &Crm, d: &Data, c: &Customer) -> Result<()> {
    let id = crm.create("companies", "Company", &company(c)).await?;
    let people: Vec<Value> = c.contacts.iter().map(|p| person(p, &id)).collect();
    crm.create_all("people", &people).await?;
    let deals: Vec<Value> = (d.orders.iter())
        .filter(|o| o.customer == c.name)
        .map(|o| opportunity(d, o, &id))
        .collect();
    crm.create_all("opportunities", &deals).await
}

/// Writes the accounts, contacts and opportunities (`info` is the crm part's `connect` info).
pub async fn seed(info: &Value, d: &Data) -> Result<()> {
    let crm = Crm::connect(info)?;
    for c in &d.customers {
        seed_account(&crm, d, c).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::data;

    #[test]
    fn an_opportunity_is_named_by_its_order_and_closes_when_due() {
        let d = data();
        let o = d.order("PO-SBMC-4471").unwrap();
        let deal = opportunity(d, o, "acc");
        assert_eq!(deal["name"], "PO-SBMC-4471 Corvexa x 400");
        assert_eq!(deal["closeDate"], "2026-11-10");
        assert_eq!(deal["amount"]["amountMicros"], 496_000_000_000u64);
        let eu = company(d.customer("Hôpital Saint-Aubin").unwrap());
        assert_eq!(eu["address"]["addressCountry"], "France");
        assert!(eu["address"].get("addressState").is_none());
    }
}
