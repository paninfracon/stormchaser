use crate::db;
use crate::{AppState, AuthClaims};
use axum::response::sse::Event;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use futures::StreamExt;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use tokio::sync::mpsc;
use tokio::time::sleep;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct LogsQuery {
    #[schema(example = 100)]
    pub limit: Option<usize>,
}

/// Format log event.
pub fn format_log_event(line: &str) -> Event {
    Event::default().event("log").data(line)
}

/// Stream step logs api.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{run_id}/steps/{step_id}/logs/stream",
    params(("run_id" = stormchaser_model::RunId, Path, description="Run ID"), ("step_id" = stormchaser_model::StepInstanceId, Path, description="Step instance ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "step"
)]
pub async fn stream_step_logs_api(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path((run_id, step_id)): Path<(RunId, StepInstanceId)>,
) -> Result<
    axum::response::sse::Sse<
        impl futures::stream::Stream<Item = Result<Event, std::convert::Infallible>>,
    >,
    StatusCode,
> {
    let log_backend = match &state.log_backend {
        Some(backend) => backend,
        None => return Err(StatusCode::NOT_IMPLEMENTED),
    };

    let instance = db::get_step_instance_by_id(&state.pool, run_id, step_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let rx = log_backend
        .stream_step_logs(
            &instance.step_name,
            stormchaser_model::StepId::new(step_id.into_inner()),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to stream logs: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).flat_map(|res| match res {
        Ok(log_line) => {
            let events: Vec<_> = log_line
                .lines()
                .map(|line| Ok(format_log_event(line)))
                .collect();
            tokio_stream::iter(events)
        }
        Err(e) => tokio_stream::iter(vec![Ok(Event::default()
            .event("error")
            .data(e.to_string()))]),
    });

    Ok(axum::response::sse::Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

/// Get step logs api.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{run_id}/steps/{step_id}/logs",
    params(
        ("run_id" = stormchaser_model::RunId, Path, description="Run ID"),
        ("step_id" = stormchaser_model::StepInstanceId, Path, description="Step instance ID"),
        ("limit" = Option<usize>, Query, description="Limit log lines")
    ),
    responses(
        (status = 200, description = "Success", body = Vec<String>),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "step"
)]
pub async fn get_step_logs_api(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path((run_id, step_id)): Path<(RunId, StepInstanceId)>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let log_backend = match &state.log_backend {
        Some(backend) => backend,
        None => return Err(StatusCode::NOT_IMPLEMENTED),
    };

    let instance = db::get_step_instance_by_id(&state.pool, run_id, step_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let logs = log_backend
        .fetch_step_logs(
            &instance.step_name,
            stormchaser_model::StepId::new(step_id.into_inner()),
            instance.started_at,
            instance.finished_at,
            query.limit,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch logs: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(logs))
}

/// Streams run logs.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{run_id}/logs/stream",
    params(("run_id" = stormchaser_model::RunId, Path, description="Run ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "step"
)]
pub async fn stream_run_logs_api(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
) -> Result<
    axum::response::sse::Sse<
        impl futures::stream::Stream<Item = Result<Event, std::convert::Infallible>>,
    >,
    StatusCode,
> {
    let log_backend = match &state.log_backend {
        Some(backend) => backend.clone(),
        None => return Err(StatusCode::NOT_IMPLEMENTED),
    };

    let (tx, rx) = mpsc::channel(100);

    let pool = state.pool.clone();
    tokio::spawn(async move {
        let mut seen_steps = std::collections::HashSet::new();
        tracing::debug!("Started run log stream task for run {}", run_id);

        loop {
            let status = db::get_workflow_run_status(&pool, run_id)
                .await
                .unwrap_or(None);

            let is_terminal = matches!(
                status.as_deref(),
                Some("succeeded") | Some("failed") | Some("cancelled")
            );

            let steps = db::get_step_names(&pool, run_id).await.unwrap_or_default();

            if !steps.is_empty() {
                tracing::trace!("Found {} steps for run {}", steps.len(), run_id);
            }

            for (step_id, step_name) in steps {
                if !seen_steps.contains(&step_id) {
                    seen_steps.insert(step_id);
                    tracing::debug!(
                        "Discovered new step {} ({}) for run log stream",
                        step_name,
                        step_id
                    );
                    let tx_clone = tx.clone();
                    let step_name_clone = step_name.clone();
                    let log_backend = log_backend.clone();

                    tokio::spawn(async move {
                        tracing::debug!(
                            "Starting sub-task to stream logs for step {}",
                            step_name_clone
                        );
                        if let Ok(mut step_rx) = log_backend
                            .stream_step_logs(
                                &step_name_clone,
                                stormchaser_model::StepId::new(step_id.into_inner()),
                            )
                            .await
                        {
                            tracing::debug!(
                                "Successfully connected to log stream for step {}",
                                step_name_clone
                            );
                            while let Some(log_res) = step_rx.recv().await {
                                match log_res {
                                    Ok(line) => {
                                        // SSE data cannot contain newlines. Split and send multiple events.
                                        for fragment in line.lines() {
                                            let prefixed =
                                                format!("[{}] {}", step_name_clone, fragment);
                                            if tx_clone.send(Ok(prefixed)).await.is_err() {
                                                return; // Receiver dropped
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Error in log stream for step {}: {:?}",
                                            step_name_clone,
                                            e
                                        );
                                        let _ = tx_clone.send(Err(e)).await;
                                        break;
                                    }
                                }
                            }
                            tracing::debug!("Log stream for step {} finished", step_name_clone);
                        } else {
                            tracing::error!(
                                "Failed to connect to log stream for step {}",
                                step_name_clone
                            );
                        }
                    });
                }
            }

            if is_terminal {
                // Allow some time for final logs to flush
                sleep(Duration::from_secs(5)).await;
                break;
            }

            sleep(Duration::from_secs(2)).await;
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).flat_map(|res| match res {
        Ok(log_line) => {
            let events: Vec<_> = log_line
                .lines()
                .map(|line| Ok(format_log_event(line)))
                .collect();
            tokio_stream::iter(events)
        }
        Err(e) => tokio_stream::iter(vec![Ok(Event::default()
            .event("error")
            .data(e.to_string()))]),
    });

    Ok(axum::response::sse::Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

#[utoipa::path(
    get,
    path = "/api/v1/runs/{run_id}/status/stream",
    params(
        ("run_id" = stormchaser_model::RunId, Path, description = "Run ID")
    ),
    responses(
        (status = 200, description = "Status stream (SSE)")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "step"
)]
/// Stream run status api.
pub async fn stream_run_status_api(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
) -> Result<
    axum::response::sse::Sse<
        impl futures::stream::Stream<Item = Result<Event, std::convert::Infallible>>,
    >,
    StatusCode,
> {
    let (tx, rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(100);
    let pool = state.pool.clone();

    tokio::spawn(async move {
        let mut last_run_status: Option<String> = None;
        let mut last_step_statuses: std::collections::HashMap<
            stormchaser_model::StepInstanceId,
            String,
        > = HashMap::new();

        loop {
            // Check workflow run status
            let current_run_status = db::get_combined_run_status(&pool, run_id)
                .await
                .unwrap_or(None);

            let is_terminal = matches!(
                current_run_status.as_deref(),
                Some("succeeded") | Some("failed") | Some("cancelled")
            );

            if current_run_status != last_run_status {
                if let Some(ref status) = current_run_status {
                    let data = serde_json::json!({ "status": status }).to_string();
                    let event = Event::default().event("run_status").data(data);
                    if tx.send(Ok(event)).await.is_err() {
                        break;
                    }
                }
                last_run_status = current_run_status.clone();
            }

            // Check step instances statuses
            let steps = db::get_combined_step_statuses(&pool, run_id)
                .await
                .unwrap_or_default();

            for (step_id, step_name, status) in steps {
                let should_emit = match last_step_statuses.get(&step_id) {
                    Some(last_status) => last_status != &status,
                    None => true,
                };

                if should_emit {
                    let data = serde_json::json!({
                        "step_id": step_id,
                        "step_name": step_name,
                        "status": status,
                    })
                    .to_string();
                    let event = Event::default().event("step_status").data(data);
                    if tx.send(Ok(event)).await.is_err() {
                        return; // Receiver dropped, break out of spawn
                    }
                    last_step_statuses.insert(step_id, status);
                }
            }

            if is_terminal {
                // Allow some time for final steps to be flushed and updated
                sleep(Duration::from_secs(2)).await;
                break;
            }

            sleep(Duration::from_secs(1)).await;
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|res| match res {
        Ok(event) => Ok(event),
        Err(_) => unreachable!(),
    });

    Ok(axum::response::sse::Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_log_event() {
        let line = "2026-04-23T19:17:03.318971Z INFO test log";
        let event = format_log_event(line);
        // The Debug representation of Event shows the internal fields. We verify it doesn't fail building
        // and stringifies correctly in typical SSE format.
        let stringified = format!("{:?}", event);
        assert!(stringified.contains("log"));
        assert!(stringified.contains("test log"));
    }
}
