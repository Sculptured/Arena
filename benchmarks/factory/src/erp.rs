//! The ERP's REST API, as the seed uses it: one site, through the part's `connect` info.
use anyhow::{anyhow, bail, Result};
use reqwest::Method;
use serde_json::{json, Value};

pub struct Erp {
    client: reqwest::Client,
    base: String,
    site: String,
    auth: String,
}

fn field<'a>(erp: &'a Value, name: &str) -> Result<&'a str> {
    erp[name]
        .as_str()
        .ok_or_else(|| anyhow!("the erp part's connect info has no {name}"))
}

impl Erp {
    /// `info` is the part's `connect` info.
    pub fn connect(info: &Value) -> Result<Erp> {
        let auth = format!(
            "token {}:{}",
            field(info, "api_key")?,
            field(info, "api_secret")?
        );
        Ok(Erp {
            client: reqwest::Client::new(),
            base: field(info, "url")?.trim_end_matches('/').to_string(),
            site: field(info, "site")?.to_string(),
            auth,
        })
    }

    fn authed(&self, method: Method, url: String) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .header("Host", &self.site)
            .header("Authorization", &self.auth)
    }

    async fn send(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        let mut r = self.authed(method, format!("{}/api/{path}", self.base));
        if let Some(body) = body {
            r = r.json(body);
        }
        let answer = r.send().await?;
        let status = answer.status();
        let text = answer.text().await?;
        if !status.is_success() {
            bail!("{path}: {status}: {}", refusal(&text));
        }
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(v["data"].clone())
    }

    /// Writes `doc`; the document as saved.
    pub async fn insert(&self, doc: &Value) -> Result<Value> {
        let doctype = doc["doctype"].as_str().unwrap_or_default();
        let path = format!("resource/{doctype}");
        let saved = self.send(Method::POST, &path, Some(doc)).await;
        saved.map_err(|e| e.context(format!("writing {doc}")))
    }

    /// Writes `doc`, unless a document of that name exists.
    pub async fn insert_missing(&self, doc: &Value) -> Result<()> {
        let doctype = doc["doctype"].as_str().unwrap_or_default();
        let at = format!("{}/api/resource/{doctype}", self.base);
        let answer = self.authed(Method::POST, at).json(doc).send().await?;
        let status = answer.status();
        if status.is_success() || status == reqwest::StatusCode::CONFLICT {
            return Ok(());
        }
        bail!(
            "writing {doc}: {status}: {}",
            refusal(&answer.text().await?)
        )
    }

    pub async fn get(&self, doctype: &str, name: &str) -> Result<Value> {
        self.send(Method::GET, &format!("resource/{doctype}/{name}"), None)
            .await
    }

    /// Sets `fields` of a document (a single, such as Global Defaults, is named after itself).
    pub async fn update(&self, doctype: &str, name: &str, fields: &Value) -> Result<Value> {
        let path = format!("resource/{doctype}/{name}");
        self.send(Method::PUT, &path, Some(fields)).await
    }

    /// Writes `doc` and submits it (docstatus 1); the document as submitted.
    pub async fn submit_new(&self, doc: &Value) -> Result<Value> {
        let saved = self.insert(doc).await?;
        let doctype = doc["doctype"].as_str().unwrap_or_default();
        let name = name_of(&saved)?;
        self.update(doctype, name, &json!({"docstatus": 1})).await
    }

    /// The names of `doctype` matching `filters`.
    pub async fn names(&self, doctype: &str, filters: &Value) -> Result<Vec<String>> {
        let path = format!("resource/{doctype}?filters={filters}&limit_page_length=0");
        let found = self.send(Method::GET, &path, None).await?;
        let list = found.as_array().into_iter().flatten();
        let names = list.filter_map(|d| d["name"].as_str());
        Ok(names.map(str::to_string).collect())
    }
}

/// The name in a saved document.
pub fn name_of(doc: &Value) -> Result<&str> {
    doc["name"]
        .as_str()
        .ok_or_else(|| anyhow!("no name in {doc}"))
}

/// Frappe's message in an error's body, else the body.
fn refusal(text: &str) -> String {
    let body: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let said = body["exception"].as_str().or(body["message"].as_str());
    said.map_or_else(|| text.chars().take(600).collect(), str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_needs_the_parts_info() {
        assert!(Erp::connect(&json!({"url": "u"})).is_err());
        let info = json!({"url": "u/", "site": "s", "api_key": "k", "api_secret": "x"});
        let erp = Erp::connect(&info).unwrap();
        assert_eq!((erp.base.as_str(), erp.auth.as_str()), ("u", "token k:x"));
    }

    #[test]
    fn a_refusal_says_what_frappe_said() {
        assert_eq!(refusal(r#"{"exception": "no such item"}"#), "no such item");
        assert_eq!(refusal("plain"), "plain");
    }
}
