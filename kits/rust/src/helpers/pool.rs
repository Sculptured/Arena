//! Running blocking work (files, Docker, child processes) off the async runtime.

/// Runs `f` on tokio's blocking pool; a panic in `f` comes back as an error.
pub async fn blocking<T, E, F>(f: F) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: From<tokio::task::JoinError> + Send + 'static,
{
    tokio::task::spawn_blocking(f).await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PartError;

    #[tokio::test]
    async fn results_and_panics_come_back() {
        let ok: Result<i32, PartError> = blocking(|| Ok(2)).await;
        assert_eq!(ok, Ok(2));
        let panicked: Result<(), PartError> = blocking(|| panic!("boom")).await;
        assert_eq!(panicked.unwrap_err().code, -32603);
        let err: anyhow::Result<()> = blocking(|| anyhow::bail!("no")).await;
        assert_eq!(err.unwrap_err().to_string(), "no");
    }
}
