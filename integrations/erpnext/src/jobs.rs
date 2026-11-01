//! Waiting for a site's background jobs, so a save sees their effects.
use std::time::{Duration, Instant};

use arena_part::PartError;
use reqwest::Method;
use serde_json::{json, Value};

use crate::rest::{Reply, Rest};

/// How long a save waits for a site's jobs.
pub const JOBS_TIMEOUT: Duration = Duration::from_secs(60);

/// The site's queued or running RQ jobs (Frappe lists only the jobs of the site asked).
fn pending_query() -> Value {
    json!({
        "doctype": "RQ Job",
        "filters": [["status", "in", ["queued", "started"]]],
        "fields": ["name"],
        "order_by": "creation desc",
        "limit_page_length": 1,
    })
}

async fn has_pending(rest: &Rest, site: &str) -> Result<bool, PartError> {
    let path = ["method", "frappe.client.get_list"];
    match rest
        .send(Method::POST, site, &path, &pending_query())
        .await?
    {
        Reply::Ok(v) => Ok(v.as_array().is_some_and(|a| !a.is_empty())),
        Reply::Refused(e) => Err(PartError::internal(format!("listing jobs of {site}: {e}"))),
    }
}

/// Waits until `site` has no queued or running background job; -32001 after `timeout`.
pub async fn drain(rest: &Rest, site: &str, timeout: Duration) -> Result<(), PartError> {
    let deadline = Instant::now() + timeout;
    while has_pending(rest, site).await? {
        if Instant::now() > deadline {
            return Err(PartError::cant_restore(format!(
                "background jobs still running for {site}"
            )));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}
