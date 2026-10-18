//! Which handler errors are worth delivering the event again for.
//!
//! An error is transient when the same event could succeed later unchanged: the
//! database or broker was unreachable, a connection dropped, Postgres chose this
//! transaction as a serialization or deadlock victim. Everything else, from a bad
//! payload to a run in the wrong state, fails the same way every time.

use std::future::Future;
use std::time::Duration;

/// How many times an event is delivered before the engine gives up on it.
pub const MAX_DELIVERIES: i64 = 10;

/// Whether anything in the error's chain is transient.
pub fn is_transient(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        if let Some(e) = cause.downcast_ref::<sqlx::Error>() {
            return sqlx_is_transient(e);
        }
        if let Some(e) = cause.downcast_ref::<reqwest::Error>() {
            return e.is_connect() || e.is_timeout();
        }
        if let Some(e) = cause.downcast_ref::<std::io::Error>() {
            return io_is_transient(e);
        }
        cause.is::<async_nats::PublishError>()
            || cause.is::<async_nats::jetstream::context::PublishError>()
    })
}

/// A dropped or refused connection, not a missing file or a bad path
fn io_is_transient(error: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(
        error.kind(),
        ConnectionRefused
            | ConnectionReset
            | ConnectionAborted
            | NotConnected
            | BrokenPipe
            | TimedOut
            | Interrupted
            | UnexpectedEof
    )
}

fn sqlx_is_transient(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Io(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed => true,
        sqlx::Error::Database(e) => e.code().is_some_and(|code| {
            // Connection exceptions, insufficient resources, operator intervention
            // (the server shutting down), serialization failure, deadlock
            code.starts_with("08")
                || code.starts_with("53")
                || code.starts_with("57P")
                || code == "40001"
                || code == "40P01"
        }),
        _ => false,
    }
}

/// How long to wait before the next delivery, after `delivered` attempts: doubling
/// from 1 s, at most a minute.
pub fn backoff(delivered: i64) -> Duration {
    let exponent = delivered.clamp(1, 7) - 1;
    Duration::from_secs((1u64 << exponent).min(60))
}

/// How long a write that nothing else would retry is tried again while it fails
/// transiently: a built-in step's result, which no event will bring back
pub const PERSIST_FOR: Duration = Duration::from_secs(10 * 60);

/// Runs `attempt` until it succeeds, fails in a way another try would not change, or
/// [`PERSIST_FOR`] has passed, waiting longer between tries each time.
pub async fn persist<T, F, Fut>(what: &str, mut attempt: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    let deadline = tokio::time::Instant::now() + PERSIST_FOR;
    let mut tries = 1;
    loop {
        match attempt().await {
            Err(e) if is_transient(&e) && tokio::time::Instant::now() < deadline => {
                let delay = backoff(tries);
                tracing::warn!(
                    "Failed to {} (try {}), retrying in {:?}: {:#}",
                    what,
                    tries,
                    delay,
                    e
                );
                tokio::time::sleep(delay).await;
                tries += 1;
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_database_connection_is_transient() {
        let error = anyhow::Error::new(sqlx::Error::PoolTimedOut).context("Failed to persist");
        assert!(is_transient(&error));
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");
        assert!(is_transient(&anyhow::Error::new(sqlx::Error::Io(io))));
    }

    #[test]
    fn a_missing_row_or_a_bad_payload_is_not() {
        assert!(!is_transient(&anyhow::Error::new(sqlx::Error::RowNotFound)));
        assert!(!is_transient(&anyhow::anyhow!("Missing run_id")));
        let missing = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        assert!(!is_transient(&anyhow::Error::new(missing)));
        let json = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        assert!(!is_transient(
            &anyhow::Error::new(json).context("bad payload")
        ));
    }

    #[tokio::test]
    async fn persist_tries_again_while_failures_are_transient() {
        let mut tries = 0;
        let result = persist("test", || {
            tries += 1;
            let attempt = tries;
            async move {
                if attempt < 3 {
                    Err(anyhow::Error::new(sqlx::Error::PoolTimedOut))
                } else {
                    Ok(attempt)
                }
            }
        })
        .await;
        assert_eq!(result.unwrap(), 3);
    }

    #[tokio::test]
    async fn persist_gives_up_at_once_on_any_other_failure() {
        let mut tries = 0;
        let result: anyhow::Result<()> = persist("test", || {
            tries += 1;
            async { Err(anyhow::anyhow!("bad payload")) }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(tries, 1);
    }

    #[test]
    fn backoff_doubles_up_to_a_minute() {
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(2), Duration::from_secs(2));
        assert_eq!(backoff(4), Duration::from_secs(8));
        assert_eq!(backoff(7), Duration::from_secs(60));
        assert_eq!(backoff(100), Duration::from_secs(60));
    }
}
