//! Making the workspace a new Twenty server starts without: its sign-up, as the app's first screen
//! does it, then an API key with the Admin role. A workspace comes with demo records, which
//! `rows::purge_demo` empties afterwards.
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A workspace's sign-in, kept with the base. Every instance is a copy of its database, so it works
/// on all of them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Creds {
    pub api_key: String,
    pub admin_email: String,
    pub admin_password: String,
}

/// An API key needs an end date; this one outlives any save.
const KEY_EXPIRES: &str = "2099-01-01T00:00:00Z";
const ADMIN_EMAIL: &str = "admin@arena-twenty.example";
const ADMIN_PASSWORD: &str = "arena-twenty-local";
const NAME: &str = "Arena";

const SIGN_UP: &str = "mutation($e:String!,$p:String!){signUp(email:$e,password:$p){\
    tokens{accessOrWorkspaceAgnosticToken{token}}}}";
const NEW_WORKSPACE: &str =
    "mutation($i:SignUpInNewWorkspaceInput){signUpInNewWorkspace(input:$i){\
    loginToken{token}}}";
const LOG_IN: &str = "mutation($l:String!,$o:String!){getAuthTokensFromLoginToken(loginToken:$l,\
    origin:$o){tokens{accessOrWorkspaceAgnosticToken{token}}}}";
const ACTIVATE: &str = "mutation($d:ActivateWorkspaceInput!){activateWorkspace(data:$d){id}}";
const ROLES: &str = "{getRoles{id label}}";
const NEW_KEY: &str = "mutation($i:CreateApiKeyInput!){createApiKey(input:$i){id}}";
const KEY_TOKEN: &str =
    "mutation($i:UUID!,$e:String!){generateApiKeyToken(apiKeyId:$i,expiresAt:$e){\
    token}}";

struct Graphql {
    client: reqwest::Client,
    url: String,
    origin: String,
    token: Option<String>,
}

impl Graphql {
    async fn call(&self, query: &str, variables: Value) -> Result<Value> {
        let mut r = self
            .client
            .post(&self.url)
            .header("origin", &self.origin)
            .json(&json!({"query": query, "variables": variables}));
        if let Some(token) = &self.token {
            r = r.bearer_auth(token);
        }
        let answer: Value = r.send().await?.json().await.context("Twenty's answer")?;
        if let Some(errors) = answer.get("errors") {
            bail!("Twenty refused {query}: {errors}");
        }
        Ok(answer["data"].clone())
    }
}

/// The text at `pointer` in `data`.
fn text(data: &Value, pointer: &str) -> Result<String> {
    data.pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("no {pointer} in {data}"))
}

fn admin_role(roles: &Value) -> Result<String> {
    let list = roles["getRoles"].as_array().into_iter().flatten();
    let found = list.into_iter().find(|r| r["label"] == "Admin");
    let id = found.and_then(|r| r["id"].as_str());
    id.map(str::to_string)
        .ok_or_else(|| anyhow!("no Admin role in {roles}"))
}

/// Signs up on the empty server at `port`, makes the workspace and its API key.
pub async fn make(port: u16) -> Result<Creds> {
    let origin = format!("http://127.0.0.1:{port}");
    let mut gql = Graphql {
        client: reqwest::Client::new(),
        url: format!("{origin}/metadata"),
        origin,
        token: None,
    };
    let access = "/tokens/accessOrWorkspaceAgnosticToken/token";
    let signed = gql
        .call(SIGN_UP, json!({"e": ADMIN_EMAIL, "p": ADMIN_PASSWORD}))
        .await?;
    gql.token = Some(text(&signed, &format!("/signUp{access}"))?);
    let input = json!({"i": {"displayName": NAME}});
    let made = gql.call(NEW_WORKSPACE, input).await?;
    let login = text(&made, "/signUpInNewWorkspace/loginToken/token")?;
    let vars = json!({"l": login, "o": gql.origin});
    let logged = gql.call(LOG_IN, vars).await?;
    gql.token = Some(text(
        &logged,
        &format!("/getAuthTokensFromLoginToken{access}"),
    )?);
    gql.call(ACTIVATE, json!({"d": {"displayName": NAME}}))
        .await?;
    Ok(Creds {
        api_key: api_key(&gql).await?,
        admin_email: ADMIN_EMAIL.into(),
        admin_password: ADMIN_PASSWORD.into(),
    })
}

async fn api_key(gql: &Graphql) -> Result<String> {
    let role = admin_role(&gql.call(ROLES, json!({})).await?)?;
    let input = json!({"i": {"name": "arena", "expiresAt": KEY_EXPIRES, "roleId": role}});
    let key = gql.call(NEW_KEY, input).await?;
    let id = text(&key, "/createApiKey/id")?;
    let token = gql
        .call(KEY_TOKEN, json!({"i": id, "e": KEY_EXPIRES}))
        .await?;
    text(&token, "/generateApiKeyToken/token")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_admin_role_is_found_by_its_label() {
        let roles =
            json!({"getRoles": [{"id": "m", "label": "Member"}, {"id": "a", "label": "Admin"}]});
        assert_eq!(admin_role(&roles).unwrap(), "a");
        assert!(admin_role(&json!({"getRoles": []})).is_err());
    }

    #[test]
    fn text_names_what_is_missing() {
        let e = text(&json!({"a": 1}), "/b").unwrap_err().to_string();
        assert!(e.contains("/b"), "{e}");
    }
}
