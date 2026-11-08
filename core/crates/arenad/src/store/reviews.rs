//! AI reviews in the metadata database: one row per reviewed branch (`branch:<id>`), pair of
//! branches (`compare:<a>:<b>`) or pair of suites (`suites:<a>:<b>`), kept until asked again.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use tokio_postgres::Row;

use super::Store;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reviews (
    key text PRIMARY KEY,
    kind text NOT NULL,
    status text NOT NULL,
    model text NOT NULL,
    review jsonb,
    issues jsonb NOT NULL DEFAULT '[]',
    error text,
    cost_usd double precision,
    cost_source text,
    tokens bigint,
    seconds double precision,
    digest_bytes bigint,
    created_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz
);
"#;

/// Reviews that were running when arenad stopped won't finish.
pub(super) const INTERRUPT: &str = "UPDATE reviews SET status = 'error', finished_at = now(), \
     error = 'arenad stopped before the review finished' WHERE status = 'running'";

const COLS: &str =
    "key, kind, status, model, review, issues, error, cost_usd, cost_source, tokens, \
     seconds, digest_bytes, created_at, finished_at";

/// A stored review: `running`, `done` (with `review`) or `error` (with `error`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoredReview {
    pub key: String,
    /// `run`, `compare` or `suites`.
    pub kind: String,
    pub status: String,
    /// The reviewer, as `claude_code:<model>` or `<provider>:<model>`.
    pub model: String,
    pub review: Option<Value>,
    /// What checking the answer found: citations dropped, fields read leniently.
    pub issues: Value,
    pub error: Option<String>,
    pub cost_usd: Option<f64>,
    pub cost_source: Option<String>,
    pub tokens: Option<i64>,
    pub seconds: Option<f64>,
    /// The size of the digest sent, in bytes.
    pub digest_bytes: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// A finished review's result.
pub struct ReviewDone<'a> {
    pub review: &'a Value,
    pub issues: &'a [String],
    pub cost_usd: Option<f64>,
    pub cost_source: Option<&'a str>,
    pub tokens: Option<i64>,
    pub seconds: f64,
    pub digest_bytes: usize,
}

fn review_from(r: &Row) -> StoredReview {
    StoredReview {
        key: r.get("key"),
        kind: r.get("kind"),
        status: r.get("status"),
        model: r.get("model"),
        review: r.get("review"),
        issues: r.get("issues"),
        error: r.get("error"),
        cost_usd: r.get("cost_usd"),
        cost_source: r.get("cost_source"),
        tokens: r.get("tokens"),
        seconds: r.get("seconds"),
        digest_bytes: r.get("digest_bytes"),
        created_at: r.get("created_at"),
        finished_at: r.get("finished_at"),
    }
}

impl Store {
    pub async fn review(&self, key: &str) -> Result<Option<StoredReview>> {
        let sql = format!("SELECT {COLS} FROM reviews WHERE key = $1");
        let row = self.client.query_opt(&sql, &[&key]).await?;
        Ok(row.as_ref().map(review_from))
    }

    /// Starts a review of `key` with `model`, replacing a finished one: the new row, or `None`
    /// when one is running already.
    pub async fn start_review(
        &self,
        key: &str,
        kind: &str,
        model: &str,
    ) -> Result<Option<StoredReview>> {
        let sql = format!(
            "INSERT INTO reviews (key, kind, status, model) VALUES ($1, $2, 'running', $3)
             ON CONFLICT (key) DO UPDATE SET kind = $2, status = 'running', model = $3,
                 review = NULL, issues = '[]', error = NULL, cost_usd = NULL, cost_source = NULL,
                 tokens = NULL, seconds = NULL, digest_bytes = NULL, created_at = now(),
                 finished_at = NULL
             WHERE reviews.status <> 'running'
             RETURNING {COLS}"
        );
        let row = self.client.query_opt(&sql, &[&key, &kind, &model]).await?;
        Ok(row.as_ref().map(review_from))
    }

    pub async fn finish_review(&self, key: &str, done: ReviewDone<'_>) -> Result<()> {
        let issues = serde_json::to_value(done.issues)?;
        let bytes = done.digest_bytes as i64;
        self.client
            .execute(
                "UPDATE reviews SET status = 'done', review = $2, issues = $3, cost_usd = $4,
                     cost_source = $5, tokens = $6, seconds = $7, digest_bytes = $8,
                     finished_at = now()
                 WHERE key = $1",
                &[
                    &key,
                    done.review,
                    &issues,
                    &done.cost_usd,
                    &done.cost_source,
                    &done.tokens,
                    &done.seconds,
                    &bytes,
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn fail_review(&self, key: &str, error: &str) -> Result<()> {
        self.client
            .execute(
                "UPDATE reviews SET status = 'error', error = $2, finished_at = now() WHERE key = $1",
                &[&key, &error],
            )
            .await?;
        Ok(())
    }
}
