//! Waiting for Sculptured to get somewhere: a check run every 100 ms until it gives an answer.
use std::future::Future;
use std::time::Duration;

use anyhow::{bail, Result};
use tokio::time::{sleep, Instant};

/// How long the integration waits for any one thing (a server up, a change recorded).
pub const LIMIT: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_millis(100);

/// Runs `check` until it gives `Some` or fails; after `LIMIT`, an error naming `what`.
pub async fn until<T, F, Fut>(what: &str, mut check: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<T>>>,
{
    let deadline = Instant::now() + LIMIT;
    loop {
        if let Some(v) = check().await? {
            return Ok(v);
        }
        if Instant::now() >= deadline {
            bail!(
                "Sculptured did not get there in {} s: {what}",
                LIMIT.as_secs()
            );
        }
        sleep(EVERY).await;
    }
}
