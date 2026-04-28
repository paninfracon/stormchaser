use super::{
    DirectRunRequest, EnqueueRequest, EnqueueResponse, ListRunsQuery, StepDetail,
    WorkflowRunDetail, WorkflowRunFullDetail,
};
use crate::{AppState, AuthClaims, RUNS_ENQUEUED};
use axum::response::sse::Event;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use futures::StreamExt;
use serde_json::Value;
use stormchaser_model::workflow::RunStatus;
use tokio::sync::mpsc;
use uuid::Uuid;

#[utoipa::path(
    post,
    path = "/api/v1/runs",
    request_body = EnqueueRequest,
    responses(
        (status = 200, description = "Workflow enqueued", body = EnqueueResponse),
        (status = 500, description = "Internal Server Error")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "stormchaser"
)]
#[tracing::instrument(skip(state, claims), fields(run_id = tracing::field::Empty, initiating_user = tracing::field::Empty))]
pub async fn enqueue_workflow(
    AuthClaims(claims): AuthClaims,
    State(state): State<AppState>,
    Json(payload): Json<EnqueueRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let run_id = Uuid::new_v4();
    let user_id = claims.email.clone().unwrap_or(claims.sub.clone());

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("initiating_user", tracing::field::display(&user_id));

    tracing::info!("Enqueuing workflow: {:?}", payload.workflow_name);
    let fencing_token = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Create WorkflowRun with initiating_user
    crate::db::insert_workflow_run(
        &mut tx,
        run_id,
        &payload.workflow_name,
        &user_id,
        &payload.repo_url,
        &payload.workflow_path,
        &payload.git_ref,
        RunStatus::Queued,
        fencing_token,
    )
    .await
    .inspect_err(|e| tracing::error!(run_id = %run_id, "Database error inserting run: {:?}", e))
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Create RunContext (placeholder for dsl_version and workflow_definition)
    crate::db::insert_run_context(
        &mut tx,
        run_id,
        "v1",
        serde_json::json!({}),
        "",
        &payload.inputs,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 3. Create RunQuotas (always, with defaults + overrides)
    let timeout = payload
        .overrides
        .as_ref()
        .and_then(|o| o.timeout.clone())
        .unwrap_or_else(|| "1h".to_string());

    crate::db::insert_run_quotas(&mut tx, run_id, 10, "1", "4Gi", "10Gi", &timeout)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    tx.commit()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Publish to NATS
    let event = serde_json::json!({
        "run_id": run_id,
        "event_type": "workflow_queued",
        "timestamp": chrono::Utc::now(),
    });

    state
        .nats
        .publish("stormchaser.run.queued", event.to_string().into())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    RUNS_ENQUEUED.add(
        1,
        &[
            opentelemetry::KeyValue::new("workflow_name", payload.workflow_name.clone()),
            opentelemetry::KeyValue::new("initiating_user", user_id.clone()),
        ],
    );

    Ok(Json(EnqueueResponse {
        run_id,
        status: "queued".to_string(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/runs",
    params(
        ListRunsQuery
    ),
    responses(
        (status = 200, description = "List of workflow runs", body = [WorkflowRunDetail]),
        (status = 500, description = "Internal Server Error")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "stormchaser"
)]
pub async fn list_workflow_runs(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Query(params): Query<ListRunsQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let limit = params.limit.unwrap_or(20).min(100);
    let offset = params.offset.unwrap_or(0);

    let mut query = sqlx::QueryBuilder::new(
        r#"
        WITH combined_runs AS (
            SELECT
                wr.id, wr.workflow_name, wr.initiating_user, wr.repo_url, wr.workflow_path, wr.git_ref,
                wr.status::run_status as "status", wr.version, wr.created_at, wr.updated_at, wr.started_resolving_at, wr.started_at, wr.finished_at, wr.error,
                rc.inputs, rc.secrets, rc.source_code, rc.dsl_version
            FROM workflow_runs wr
            JOIN run_contexts rc ON wr.id = rc.run_id
            UNION ALL
            SELECT
                wr.id, wr.workflow_name, wr.initiating_user, wr.repo_url, wr.workflow_path, wr.git_ref,
                wr.status::run_status as "status", wr.version, wr.created_at, wr.updated_at, wr.started_resolving_at, wr.started_at, wr.finished_at, wr.error,
                rc.inputs, rc.secrets, rc.source_code, rc.dsl_version
            FROM archived_workflow_runs wr
            JOIN archived_run_contexts rc ON wr.id = rc.run_id
        )
        SELECT * FROM combined_runs wr WHERE 1=1
        "#,
    );

    if let Some(name) = params.workflow_name {
        query.push(" AND wr.workflow_name LIKE ");
        query.push_bind(format!("%{}%", name));
    }

    if let Some(status) = params.status {
        query.push(" AND wr.status = ");
        query.push_bind(status);
    }

    if let Some(user) = params.initiating_user {
        query.push(" AND wr.initiating_user = ");
        query.push_bind(user);
    }

    if let Some(repo) = params.repo_url {
        query.push(" AND wr.repo_url = ");
        query.push_bind(repo);
    }

    if let Some(path) = params.workflow_path {
        query.push(" AND wr.workflow_path = ");
        query.push_bind(path);
    }

    if let Some(after) = params.created_after {
        query.push(" AND wr.created_at >= ");
        query.push_bind(after);
    }

    if let Some(before) = params.created_before {
        query.push(" AND wr.created_at <= ");
        query.push_bind(before);
    }

    query.push(" ORDER BY wr.created_at DESC LIMIT ");
    query.push_bind(limit as i64);
    query.push(" OFFSET ");
    query.push_bind(offset as i64);

    let runs: Vec<WorkflowRunDetail> = query
        .build_query_as()
        .fetch_all(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch workflow runs: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(runs))
}

#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}",
    params(
        ("id" = Uuid, Path, description = "Run ID")
    ),
    responses(
        (status = 200, description = "Workflow run details", body = WorkflowRunFullDetail),
        (status = 404, description = "Run not found")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "stormchaser"
)]
pub async fn get_workflow_run(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, StatusCode> {
    // 1. Fetch the workflow run detail
    let detail: WorkflowRunDetail =
        sqlx::query_as("SELECT * FROM combined_run_details WHERE id = $1")
            .bind(run_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to fetch workflow run detail for {}: {:?}",
                    run_id,
                    e
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?
            .ok_or(StatusCode::NOT_FOUND)?;

    // 2. Fetch all step instances for this run (active or archived)
    let instances = crate::db::get_step_instances(&state.pool, run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch step instances for {}: {:?}", run_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut steps = Vec::new();
    for instance in instances {
        // Fetch outputs for this step (active or archived)
        let outputs = crate::db::get_step_outputs(&state.pool, instance.id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Fetch status history
        let history = crate::db::get_step_status_history(&state.pool, instance.id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let mut logs = if let Some(backend) = &state.log_backend {
            backend
                .fetch_step_logs(
                    &instance.step_name,
                    instance.id,
                    instance.started_at,
                    instance.finished_at,
                )
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!("Failed to fetch logs for step {}: {:?}", instance.id, e);
                    vec![format!("Error fetching logs: {}", e)]
                })
        } else {
            vec!["Log backend not configured".to_string()]
        };

        if logs.len() > 1000 {
            logs = logs.split_off(logs.len() - 1000);
        }

        steps.push(StepDetail {
            instance,
            outputs,
            history,
            logs,
        });
    }

    let artifacts = crate::db::list_run_artifacts(&state.pool, run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch run artifacts for {}: {:?}", run_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let test_summaries = crate::db::list_run_test_summaries(&state.pool, run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch test summaries for {}: {:?}", run_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let test_cases = crate::db::list_run_test_cases(&state.pool, run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch test cases for {}: {:?}", run_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(WorkflowRunFullDetail {
        detail,
        steps,
        artifacts,
        test_summaries,
        test_cases,
    }))
}

pub async fn delete_workflow_run_api(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, StatusCode> {
    crate::db::delete_workflow_run(&state.pool, run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete workflow run {}: {:?}", run_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}

#[tracing::instrument(skip(state, claims), fields(run_id = tracing::field::Empty, initiating_user = tracing::field::Empty))]
pub async fn direct_run(
    AuthClaims(claims): AuthClaims,
    State(state): State<AppState>,
    Json(payload): Json<DirectRunRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let run_id = Uuid::new_v4();
    let user_id = claims.email.clone().unwrap_or(claims.sub.clone());

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("initiating_user", tracing::field::display(&user_id));

    // Publish to NATS stormchaser.run.direct
    let event = serde_json::json!({
        "run_id": run_id,
        "dsl": payload.dsl,
        "initiating_user": user_id,
        "inputs": payload.inputs,
    });

    state
        .nats
        .publish("stormchaser.run.direct", event.to_string().into())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(EnqueueResponse {
        run_id,
        status: "started".to_string(),
    }))
}

pub async fn stream_workflow_runs_api(
    AuthClaims(_claims): AuthClaims,
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
        let mut subscriber = match nats.subscribe("stormchaser.run.>").await {
            Ok(sub) => sub,
            Err(e) => {
                tracing::error!("Failed to subscribe to NATS for workflow runs: {:?}", e);
                return;
            }
        };

        while let Some(msg) = subscriber.next().await {
            let payload: Value = match serde_json::from_slice(&msg.payload) {
                Ok(p) => p,
                Err(_) => continue,
            };

            if let Some(run_id_str) = payload.get("run_id").and_then(|id| id.as_str()) {
                if let Ok(run_id) = Uuid::parse_str(run_id_str) {
                    // Fetch full detail for the run
                    let detail = crate::db::get_workflow_run_detail(&pool, run_id)
                        .await
                        .unwrap_or(None);

                    if let Some(run) = detail {
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
