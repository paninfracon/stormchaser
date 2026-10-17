use crate::git_cache;
use crate::handler;
use crate::retry;
use async_nats::jetstream::message::Message;
use async_nats::jetstream::AckKind;
use serde_json::Value;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::auth;
use stormchaser_model::LogBackend;
use stormchaser_tls::TlsReloader;
use uuid::Uuid;

/// How often a handler that is still running tells the stream so. Well inside
/// JetStream's default 30 s ack wait, after which it would deliver the event again.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);

/// Handles an event in its own task. The event is acknowledged once handled, or once
/// it has failed in a way another try would not change; after a transient failure the
/// stream delivers it again, later.
fn spawn_handler<F>(message: Message, what: String, handler: F)
where
    F: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    tokio::spawn(async move {
        let result = with_progress(&message, handler).await;
        settle(&message, &what, result).await;
    });
}

async fn with_progress<F: Future>(message: &Message, handler: F) -> F::Output {
    tokio::pin!(handler);
    let mut ticks = tokio::time::interval(PROGRESS_INTERVAL);
    // The first tick is immediate
    ticks.tick().await;
    loop {
        tokio::select! {
            output = &mut handler => return output,
            _ = ticks.tick() => {
                let _ = message.ack_with(AckKind::Progress).await;
            }
        }
    }
}

fn deliveries(message: &Message) -> i64 {
    message
        .info()
        .map(|info| info.delivered)
        .unwrap_or(retry::MAX_DELIVERIES)
}

async fn settle(message: &Message, what: &str, result: anyhow::Result<()>) {
    let Err(e) = result else {
        let _ = message.double_ack().await;
        return;
    };
    let delivered = deliveries(message);
    if retry::is_transient(&e) && delivered < retry::MAX_DELIVERIES {
        let delay = retry::backoff(delivered);
        tracing::warn!(
            "Failed to handle {} (delivery {}), retrying in {:?}: {:#}",
            what,
            delivered,
            delay,
            e
        );
        let _ = message.ack_with(AckKind::Nak(Some(delay))).await;
    } else {
        tracing::error!("Failed to handle {}: {:?}", what, e);
        let _ = message.double_ack().await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_run_events(
    subject: &str,
    payload: Value,
    message: async_nats::jetstream::message::Message,
    pool: sqlx::PgPool,
    git_cache: Arc<git_cache::GitCache>,
    opa_client: Arc<auth::OpaClient>,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) {
    let run_id_str = match payload["run_id"].as_str() {
        Some(id) => id,
        None => {
            let _ = message.double_ack().await;
            return;
        }
    };
    let run_id = match Uuid::parse_str(run_id_str) {
        Ok(id) => id,
        Err(_) => {
            let _ = message.double_ack().await;
            return;
        }
    };

    match subject {
        "stormchaser.v1.run.queued" => {
            spawn_handler(
                message,
                format!("workflow queued event for {}", run_id),
                async move {
                    handler::handle_workflow_queued(
                        stormchaser_model::RunId::new(run_id),
                        payload,
                        pool,
                        git_cache,
                        opa_client,
                        nats_client,
                        tls_reloader,
                    )
                    .await
                },
            );
        }
        "stormchaser.v1.run.direct" => {
            spawn_handler(
                message,
                format!("workflow direct event for {}", run_id),
                async move {
                    handler::handle_workflow_direct(payload, pool, opa_client, nats_client).await
                },
            );
        }
        "stormchaser.v1.run.start_pending" => {
            spawn_handler(
                message,
                format!("workflow start_pending event for {}", run_id),
                async move {
                    handler::handle_workflow_start_pending(
                        stormchaser_model::RunId::new(run_id),
                        pool,
                        nats_client,
                        tls_reloader,
                    )
                    .await
                },
            );
        }
        _ => {
            let _ = message.double_ack().await;
        }
    }
}

async fn handle_runner_events(
    subject: &str,
    payload: Value,
    message: async_nats::jetstream::message::Message,
    pool: sqlx::PgPool,
) {
    match subject {
        "stormchaser.v1.runner.register" => {
            spawn_handler(message, "runner registration".to_string(), async move {
                handler::handle_runner_registration(payload, pool).await
            });
        }
        "stormchaser.v1.runner.heartbeat" => {
            spawn_handler(message, "runner heartbeat".to_string(), async move {
                handler::handle_runner_heartbeat(payload, pool).await
            });
        }
        "stormchaser.v1.runner.offline" => {
            spawn_handler(message, "runner offline".to_string(), async move {
                handler::handle_runner_offline(payload, pool).await
            });
        }
        _ => {
            let _ = message.double_ack().await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_step_events(
    subject: &str,
    payload: Value,
    message: async_nats::jetstream::message::Message,
    pool: sqlx::PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
    log_backend: Arc<Option<LogBackend>>,
) {
    match subject {
        "stormchaser.v1.step.register_wasm" => {
            spawn_handler(message, "WASM step registration".to_string(), async move {
                handler::handle_wasm_registration(payload, pool).await
            });
        }
        "stormchaser.v1.step.unpacking_sfs" => {
            spawn_handler(
                message,
                "step unpacking_sfs event".to_string(),
                async move { handler::handle_step_unpacking_sfs(payload, pool).await },
            );
        }
        "stormchaser.v1.step.packing_sfs" => {
            spawn_handler(message, "step packing_sfs event".to_string(), async move {
                handler::handle_step_packing_sfs(payload, pool).await
            });
        }
        "stormchaser.v1.step.initializing" => {
            let event: stormchaser_model::events::StepInitializingEvent =
                match serde_json::from_value(payload) {
                    Ok(e) => e,
                    Err(err) => {
                        tracing::error!("Failed to parse StepInitializingEvent: {}", err);
                        let _ = message.double_ack().await;
                        return;
                    }
                };
            spawn_handler(message, "step initializing event".to_string(), async move {
                handler::handle_step_initializing(event, pool).await
            });
        }
        "stormchaser.v1.step.running" => {
            let event: stormchaser_model::events::StepRunningEvent =
                match serde_json::from_value(payload) {
                    Ok(e) => e,
                    Err(err) => {
                        tracing::error!("Failed to parse StepRunningEvent: {}", err);
                        let _ = message.double_ack().await;
                        return;
                    }
                };
            spawn_handler(message, "step running event".to_string(), async move {
                handler::handle_step_running(event, pool).await
            });
        }
        "stormchaser.v1.step.completed" => {
            let event: stormchaser_model::events::StepCompletedEvent =
                match serde_json::from_value(payload) {
                    Ok(e) => e,
                    Err(err) => {
                        tracing::error!("Failed to parse StepCompletedEvent: {}", err);
                        let _ = message.double_ack().await;
                        return;
                    }
                };
            spawn_handler(message, "step completed event".to_string(), async move {
                handler::handle_step_completed(event, pool, nats_client, log_backend, tls_reloader)
                    .await
            });
        }
        "stormchaser.v1.step.failed" => {
            let event: stormchaser_model::events::StepFailedEvent =
                match serde_json::from_value(payload) {
                    Ok(e) => e,
                    Err(err) => {
                        tracing::error!("Failed to parse StepFailedEvent: {}", err);
                        let _ = message.double_ack().await;
                        return;
                    }
                };
            spawn_handler(message, "step failed event".to_string(), async move {
                handler::handle_step_failed(event, pool, nats_client, tls_reloader).await
            });
        }
        _ => {
            let _ = message.double_ack().await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_message(
    subject: &str,
    payload: Value,
    message: async_nats::jetstream::message::Message,
    pool: sqlx::PgPool,
    git_cache: Arc<git_cache::GitCache>,
    opa_client: Arc<auth::OpaClient>,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
    log_backend: Arc<Option<LogBackend>>,
) {
    let normalized_subject = stormchaser_model::runner_auth::normalize_subject(subject);
    let subject = normalized_subject.as_str();

    // An event delivered this often keeps failing without saying so: its handler panics,
    // or the engine dies handling it
    let delivered = deliveries(&message);
    if delivered > retry::MAX_DELIVERIES {
        tracing::error!(
            "Giving up on {} after {} deliveries: {}",
            subject,
            delivered,
            payload
        );
        let _ = message.double_ack().await;
        return;
    }

    // What a runner reports must be signed with the key the engine issued it, for the
    // subject it arrived on. Its signature is timed against when the stream received
    // it, not now: the stream keeps reports while the engine is down.
    if let Some(sender) = crate::runner_auth::sender_of(subject) {
        let published_at = message
            .info()
            .map(|info| info.published.unix_timestamp())
            .unwrap_or_else(|_| chrono::Utc::now().timestamp());
        match crate::runner_auth::refusal(&pool, sender, subject, &payload, published_at).await {
            Ok(None) => {}
            Ok(Some(reason)) => {
                tracing::warn!("Ignoring {}: {}", subject, reason);
                let _ = message.double_ack().await;
                return;
            }
            // Not known to be bad: the stream delivers it again
            Err(e) => {
                tracing::warn!("Could not check {}, retrying later: {:#}", subject, e);
                let _ = message
                    .ack_with(async_nats::jetstream::AckKind::Nak(Some(
                        std::time::Duration::from_secs(5),
                    )))
                    .await;
                return;
            }
        }
    }

    if subject.starts_with("stormchaser.v1.run.") {
        handle_run_events(
            subject,
            payload,
            message,
            pool,
            git_cache,
            opa_client,
            nats_client,
            tls_reloader,
        )
        .await;
    } else if subject.starts_with("stormchaser.v1.runner.") {
        handle_runner_events(subject, payload, message, pool).await;
    } else if subject.starts_with("stormchaser.v1.step.") {
        handle_step_events(
            subject,
            payload,
            message,
            pool,
            nats_client,
            tls_reloader,
            log_backend,
        )
        .await;
    } else {
        tracing::warn!(
            "Unhandled subject '{}', acknowledging to prevent redelivery",
            subject
        );
        let _ = message.double_ack().await;
    }
}
