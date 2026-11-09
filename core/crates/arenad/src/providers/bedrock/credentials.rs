//! The AWS credentials a Bedrock call signs with, from the user's own setup; arenad never stores
//! them. In order:
//!
//! 1. `AWS_BEARER_TOKEN_BEDROCK`: a Bedrock API key, sent as a bearer token;
//! 2. `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` (and `AWS_SESSION_TOKEN`);
//! 3. whatever the AWS CLI resolves (`aws configure export-credentials`, honouring `AWS_PROFILE`):
//!    the shared files, IAM Identity Center, `aws login`, assumed roles, instance roles. These are
//!    kept in memory until five minutes before they expire.

use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use crate::providers::Secret;

/// Long-term or temporary access keys.
#[derive(Debug, Clone)]
pub(crate) struct Keys {
    pub access_key: String,
    pub secret: Secret,
    pub session_token: Option<Secret>,
    pub expires: Option<DateTime<Utc>>,
}

/// How a request proves who sends it.
#[derive(Debug, Clone)]
pub(crate) enum Auth {
    Bearer(Secret),
    Keys(Keys),
}

const BEARER_ENV: &str = "AWS_BEARER_TOKEN_BEDROCK";
/// What to do when no credentials are found.
pub(crate) const NO_CREDENTIALS: &str = "no AWS credentials: set AWS_BEARER_TOKEN_BEDROCK, or AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY, or sign in with the aws CLI";

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// The credentials in the environment, if any.
fn from_env() -> Option<Auth> {
    if let Some(token) = env(BEARER_ENV) {
        return Some(Auth::Bearer(Secret::new(token)));
    }
    let access_key = env("AWS_ACCESS_KEY_ID")?;
    let secret = env("AWS_SECRET_ACCESS_KEY")?;
    Some(Auth::Keys(Keys {
        access_key,
        secret: Secret::new(secret),
        session_token: env("AWS_SESSION_TOKEN").map(Secret::new),
        expires: None,
    }))
}

/// True when some source of credentials is there, judged without asking the AWS CLI.
pub(crate) fn some_source() -> bool {
    from_env().is_some() || crate::api::on_path("aws")
}

/// Keys from `aws configure export-credentials --format process` output.
pub(crate) fn parse_process_output(stdout: &str) -> Result<Keys, String> {
    let v: Value = serde_json::from_str(stdout.trim())
        .map_err(|_| "the aws CLI gave credentials arenad could not read".to_string())?;
    let text = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let (Some(access_key), Some(secret)) = (text("AccessKeyId"), text("SecretAccessKey")) else {
        return Err("the aws CLI gave no access key".into());
    };
    let expires = text("Expiration")
        .and_then(|e| DateTime::parse_from_rfc3339(&e).ok())
        .map(|e| e.with_timezone(&Utc));
    Ok(Keys {
        access_key,
        secret: Secret::new(secret),
        session_token: text("SessionToken").map(Secret::new),
        expires,
    })
}

/// The first line of what the aws CLI said on failure (it says what to do, e.g. sign in again).
fn cli_failure(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("no message");
    let line: String = line.chars().take(200).collect();
    format!(
        "no usable AWS credentials: the aws CLI says: {}",
        line.trim()
    )
}

async fn from_cli() -> Result<Keys, String> {
    let mut cmd = tokio::process::Command::new("aws");
    cmd.args(["configure", "export-credentials", "--format", "process"])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let run = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output());
    let out = match run.await {
        Err(_) => return Err("the aws CLI took longer than 30 s to give credentials".into()),
        Ok(Err(_)) => return Err(NO_CREDENTIALS.into()),
        Ok(Ok(out)) => out,
    };
    if !out.status.success() {
        return Err(cli_failure(&out.stderr));
    }
    parse_process_output(&String::from_utf8_lossy(&out.stdout))
}

fn cache() -> &'static Mutex<Option<Keys>> {
    static CACHE: Mutex<Option<Keys>> = Mutex::new(None);
    &CACHE
}

/// True while `keys` are good for five more minutes (keys without an expiry always are).
fn fresh(keys: &Keys, now: DateTime<Utc>) -> bool {
    keys.expires.is_none_or(|e| e - Duration::minutes(5) > now)
}

/// The credentials to sign the next call with.
pub(crate) async fn resolve() -> Result<Auth, String> {
    if let Some(a) = from_env() {
        return Ok(a);
    }
    let cached = cache().lock().unwrap().clone();
    if let Some(k) = cached.filter(|k| fresh(k, Utc::now())) {
        return Ok(Auth::Keys(k));
    }
    let keys = from_cli().await?;
    *cache().lock().unwrap() = Some(keys.clone());
    Ok(Auth::Keys(keys))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_cli_process_format() {
        let out = r#"{"Version": 1, "AccessKeyId": "AKIA1", "SecretAccessKey": "topsecret",
                      "SessionToken": "st", "Expiration": "2026-11-08T12:00:00+00:00"}"#;
        let k = parse_process_output(out).unwrap();
        assert_eq!(k.access_key, "AKIA1");
        assert_eq!(k.session_token.as_ref().map(Secret::expose), Some("st"));
        let at = |h| DateTime::parse_from_rfc3339(h).unwrap().with_timezone(&Utc);
        assert!(fresh(&k, at("2026-11-08T11:50:00Z")));
        assert!(!fresh(&k, at("2026-11-08T11:56:00Z")));
        assert!(parse_process_output(r#"{"AccessKeyId": ""}"#).is_err());
        assert!(!format!("{k:?}").contains("topsecret"));
    }

    #[test]
    fn a_cli_failure_says_its_first_line_only() {
        let e =
            cli_failure(b"\naws: [ERROR]: Your session has expired. Please reauthenticate.\nmore");
        assert_eq!(
            e,
            "no usable AWS credentials: the aws CLI says: aws: [ERROR]: Your session has expired. Please reauthenticate."
        );
    }
}
