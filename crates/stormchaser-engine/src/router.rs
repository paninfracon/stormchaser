use serde_json::Value;
use std::sync::Arc;
use stormchaser_engine::git_cache;
use stormchaser_engine::handler;
use stormchaser_model::auth;
use stormchaser_model::LogBackend;
use stormchaser_tls::TlsReloader;
use uuid::Uuid;

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
            tokio::spawn(async move {
                if let Err(e) = handler::handle_workflow_queued(
                    stormchaser_model::RunId::new(run_id),
                    pool,
                    git_cache,
                    opa_client,
                    nats_client,
                    tls_reloader,
                )
                .await
                {
                    tracing::error!(
                        "Failed to handle workflow queued event for {}: {:?}",
                        run_id,
                        e
                    );
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.v1.run.direct" => {
            tokio::spawn(async move {
                if let Err(e) =
                    handler::handle_workflow_direct(payload, pool, opa_client, nats_client).await
                {
                    tracing::error!(
                        "Failed to handle workflow direct event for {}: {:?}",
                        run_id,
                        e
                    );
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.v1.run.start_pending" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_workflow_start_pending(
                    stormchaser_model::RunId::new(run_id),
                    pool,
                    nats_client,
                    tls_reloader,
                )
                .await
                {
                    tracing::error!(
                        "Failed to handle workflow start_pending event for {}: {:?}",
                        run_id,
                        e
                    );
                }
                let _ = message.double_ack().await;
            });
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
            tokio::spawn(async move {
                if let Err(e) = handler::handle_runner_registration(payload, pool).await {
                    tracing::error!("Failed to handle runner registration: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.v1.runner.heartbeat" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_runner_heartbeat(payload, pool).await {
                    tracing::error!("Failed to handle runner heartbeat: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.v1.runner.offline" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_runner_offline(payload, pool).await {
                    tracing::error!("Failed to handle runner offline: {:?}", e);
                }
                let _ = message.double_ack().await;
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
            tokio::spawn(async move {
                if let Err(e) = handler::handle_wasm_registration(payload, pool).await {
                    tracing::error!("Failed to handle WASM step registration: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.v1.step.unpacking_sfs" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_step_unpacking_sfs(payload, pool).await {
                    tracing::error!("Failed to handle step unpacking_sfs event: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.v1.step.packing_sfs" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_step_packing_sfs(payload, pool).await {
                    tracing::error!("Failed to handle step packing_sfs event: {:?}", e);
                }
                let _ = message.double_ack().await;
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
            tokio::spawn(async move {
                if let Err(e) = handler::handle_step_running(event, pool).await {
                    tracing::error!("Failed to handle step running event: {:?}", e);
                }
                let _ = message.double_ack().await;
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
            tokio::spawn(async move {
                if let Err(e) = handler::handle_step_completed(
                    event,
                    pool,
                    nats_client,
                    log_backend,
                    tls_reloader,
                )
                .await
                {
                    tracing::error!("Failed to handle step completed event: {:?}", e);
                }
                let _ = message.double_ack().await;
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
            tokio::spawn(async move {
                if let Err(e) =
                    handler::handle_step_failed(event, pool, nats_client, tls_reloader).await
                {
                    tracing::error!("Failed to handle step failed event: {:?}", e);
                }
                let _ = message.double_ack().await;
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
        let _ = message.double_ack().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_routing_helpers() {
        let _a = handle_run_events;
        let _b = handle_runner_events;
        let _c = handle_step_events;
    }
}
