//! What every provider adapter shares on the wire: where a path under a provider's base is,
//! and sending a request to get back its status, JSON body and `retry-after`.

use serde_json::{json, Value};

/// The URL of `path` under `base` (one `/` between them).
pub(super) fn url(base: &str, path: &str) -> String {
    format!("{}/{path}", base.trim_end_matches('/'))
}

/// A provider's answer as it came.
pub(super) struct Answer {
    pub status: u16,
    /// Its JSON body, `{"raw": text}` when it isn't JSON.
    pub body: Value,
    /// The seconds its `retry-after` header asks to wait, when it gives a number.
    pub retry_after: Option<u64>,
}

/// Sends `req`; `Err` when no answer came. Messages never carry the URL, so never a credential
/// in it.
pub(super) async fn exchange(req: reqwest::RequestBuilder) -> Result<Answer, String> {
    let r = req
        .send()
        .await
        .map_err(|e| format!("upstream request failed: {}", e.without_url()))?;
    let status = r.status().as_u16();
    let retry_after = retry_after(r.headers());
    let text = r
        .text()
        .await
        .map_err(|e| format!("upstream response unreadable: {}", e.without_url()))?;
    let body = serde_json::from_str(&text).unwrap_or_else(|_| json!({"raw": text}));
    Ok(Answer {
        status,
        body,
        retry_after,
    })
}

/// The whole seconds of a `retry-after` header (its HTTP-date form is not read).
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let v = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    v.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

    #[test]
    fn paths_join_their_base_once() {
        assert_eq!(url("http://a/v1/", "models"), "http://a/v1/models");
        assert_eq!(url("http://a/v1", "messages"), "http://a/v1/messages");
    }

    #[test]
    fn retry_after_is_read_in_seconds() {
        let mut h = HeaderMap::new();
        assert_eq!(retry_after(&h), None);
        h.insert(RETRY_AFTER, HeaderValue::from_static(" 7"));
        assert_eq!(retry_after(&h), Some(7));
        h.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT"),
        );
        assert_eq!(retry_after(&h), None);
    }
}
