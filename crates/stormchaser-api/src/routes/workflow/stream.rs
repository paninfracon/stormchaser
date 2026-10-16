use crate::authz::{self, Caller, Kind};
use crate::db;
use crate::AppState;
use axum::response::sse::Event;
use axum::{extract::State, http::StatusCode};
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Bound on the run ids one stream remembers as unreadable.
const MAX_UNREADABLE_RUNS: usize = 10_000;

#[utoipa::path(
    get,
    path = "/api/v1/runs/stream",
    responses(
        (status = 200, description = "Workflow runs stream (SSE)")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "workflow"
)]
/// Stream workflow runs api.
pub async fn stream_workflow_runs_api(
    caller: Caller,
    State(state): State<AppState>,
) -> Result<
    axum::response::sse::Sse<
        impl futures::stream::Stream<Item = Result<Event, std::convert::Infallible>>,
    >,
    StatusCode,
> {
    let (tx, rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(100);

    let nats = state.nats.clone();
    let pool = state.pool.clone();

    tokio::spawn(async move {
        // Only runs the caller may read are streamed to them
        let mut filter = authz::ReadFilter::new(&state, &caller, Kind::Run);

        let legacy_subscriber = match nats.subscribe("stormchaser.v1.run.>").await {
            Ok(sub) => sub,
            Err(e) => {
                tracing::error!("Failed to subscribe to NATS for workflow runs: {:?}", e);
                return;
            }
        };
        let sharded_subscriber = match nats.subscribe("stormchaser.v1.*.run.>").await {
            Ok(sub) => sub,
            Err(e) => {
                tracing::error!(
                    "Failed to subscribe to sharded NATS subject for workflow runs: {:?}",
                    e
                );
                return;
            }
        };
        let mut subscriber = futures::stream::select(legacy_subscriber, sharded_subscriber);
        // Runs this caller may not read, so their later events cost no database query
        let mut unreadable = std::collections::HashSet::new();

        loop {
            // A caller who may read none of the events never sends, so the client's
            // disconnect must end the task here rather than at a failed send
            let msg = tokio::select! {
                _ = tx.closed() => break,
                msg = subscriber.next() => match msg {
                    Some(msg) => msg,
                    None => break,
                },
            };
            let ce: cloudevents::Event = match serde_json::from_slice(&msg.payload) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let payload: Value = if let Some(cloudevents::Data::Json(v)) = ce.data() {
                v.clone()
            } else {
                continue;
            };

            if let Some(run_id_str) = payload.get("run_id").and_then(|id| id.as_str()) {
                if let Ok(run_id) = Uuid::parse_str(run_id_str) {
                    if unreadable.contains(&run_id) {
                        continue;
                    }
                    // Fetch full detail for the run
                    let detail =
                        db::get_workflow_run_detail(&pool, stormchaser_model::RunId::new(run_id))
                            .await
                            .unwrap_or(None);

                    if let Some(mut run) = detail {
                        match filter.can_read(Some(&run.initiating_user)).await {
                            Ok(true) => {}
                            Ok(false) => {
                                if unreadable.len() >= MAX_UNREADABLE_RUNS {
                                    unreadable.clear();
                                }
                                unreadable.insert(run_id);
                                continue;
                            }
                            Err(_) => continue,
                        }
                        if let Some(status_str) = payload.get("status").and_then(|s| s.as_str()) {
                            if let Ok(status) = serde_json::from_value(serde_json::Value::String(
                                status_str.to_string(),
                            )) {
                                run.status = status;
                            }
                        }

                        // Withheld rather than sent unscrubbed if the secrets cannot be read
                        let Ok(error) = crate::redact::RunRedactor::scrub_run_error(
                            &pool,
                            run.id,
                            run.error.take(),
                        )
                        .await
                        else {
                            continue;
                        };
                        run.error = error;

                        let data = serde_json::to_string(&run).unwrap_or_default();
                        let event = Event::default().event("workflow_run").data(data);
                        if tx.send(Ok(event)).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|res| match res {
        Ok(event) => Ok(event),
        Err(_) => unreachable!(),
    });

    Ok(axum::response::sse::Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}
