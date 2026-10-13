use super::{
    CreateStorageBackendRequest, TestConnectionRequest, TestConnectionResponse,
    UpdateStorageBackendRequest,
};
use crate::authz::{self, Caller, Kind};
use crate::db;
use crate::{AppState, AuthClaims};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use stormchaser_model::connections::ArtifactRegistry;
use stormchaser_model::ConnectionId;
use stormchaser_model::RunId;
use stormchaser_model::TestReportId;

mod validate;
use validate::validate_connection;

/// Creates a storage backend.
#[utoipa::path(
    post,
    path = "/api/v1/connections",
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn create_connection(
    caller: Caller,
    State(state): State<AppState>,
    Json(payload): Json<CreateStorageBackendRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let id = stormchaser_model::ConnectionId::new_v4();

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // If setting as default SFS, unset existing default
    if payload.is_default_sfs {
        db::unset_default_sfs(&mut tx)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    db::create_connection(
        &mut tx,
        id,
        &payload.name,
        &payload.description,
        &payload.connection_type,
        &payload.config,
        &payload.aws_assume_role_arn,
        &payload.encrypted_credentials,
        payload.is_default_sfs,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to create storage backend: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tx.commit()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    authz::record_owner(&state.pool, Kind::Connection, id.into_inner(), &caller).await?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

/// List storage backends.
#[utoipa::path(
    get,
    path = "/api/v1/connections",
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn list_connections(
    caller: Caller,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    let backends = db::list_connections(&state.pool).await.map_err(|e| {
        tracing::error!("list_connections error: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let backends = authz::filter_readable(&state, &caller, Kind::Connection, backends, |c| {
        c.id.into_inner()
    })
    .await?;
    let backends: Vec<_> = backends
        .into_iter()
        .map(crate::redact::connection)
        .collect();
    Ok(Json(backends))
}

/// Get storage backend.
#[utoipa::path(
    get,
    path = "/api/v1/connections/{id}",
    params(("id" = stormchaser_model::ConnectionId, Path, description="Backend ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn get_connection(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(id): Path<ConnectionId>,
) -> Result<impl IntoResponse, StatusCode> {
    let backend = db::get_connection(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(crate::redact::connection(backend)))
}

/// Update storage backend.
#[utoipa::path(
    patch,
    path = "/api/v1/connections/{id}",
    params(("id" = stormchaser_model::ConnectionId, Path, description="Backend ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn update_connection(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(id): Path<ConnectionId>,
    Json(mut payload): Json<UpdateStorageBackendRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    // Responses mask stored secrets. A client that loads a connection, edits it and saves
    // it sends those masks back; they mean "unchanged", not a new value.
    if payload.encrypted_credentials.as_deref() == Some(crate::redact::MASK) {
        payload.encrypted_credentials = None;
    }
    if let Some(config) = payload.config.as_mut() {
        let stored = db::get_connection(&state.pool, id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        crate::redact::restore_masked(config, &stored.config);
    }

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Some(true) = payload.is_default_sfs {
        db::unset_default_sfs(&mut tx)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    db::update_connection(&mut tx, id, &payload)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update storage backend: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tx.commit()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

/// Test a connection.
#[utoipa::path(
    post,
    path = "/api/v1/connections/test",
    responses(
        (status = 200, description = "Success", body = TestConnectionResponse),
        (status = 400, description = "Bad Request"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn test_connection(
    AuthClaims(_claims): AuthClaims,
    State(_state): State<AppState>,
    Json(payload): Json<TestConnectionRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let (success, message) = validate_connection(&payload).await;

    Ok((
        StatusCode::OK,
        Json(TestConnectionResponse { success, message }),
    ))
}

/// Deletes a storage backend.
#[utoipa::path(
    delete,
    path = "/api/v1/connections/{id}",
    params(("id" = stormchaser_model::ConnectionId, Path, description="Backend ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn delete_connection(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(id): Path<ConnectionId>,
) -> Result<impl IntoResponse, StatusCode> {
    db::delete_connection(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/artifacts",
    params(
        ("id" = stormchaser_model::RunId, Path, description = "Run ID")
    ),
    responses(
        (status = 200, description = "List of artifacts", body = [ArtifactRegistry]),
        (status = 500, description = "Internal Server Error")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "storage"
)]
/// Lists run artifacts.
pub async fn list_run_artifacts(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
) -> Result<impl IntoResponse, StatusCode> {
    let artifacts = db::list_run_artifacts(&state.pool, run_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(artifacts))
}

/// Lists run test reports.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{run_id}/reports",
    params(("run_id" = stormchaser_model::RunId, Path, description="Run ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn list_run_test_reports(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
) -> Result<impl IntoResponse, StatusCode> {
    let reports = db::list_run_test_reports(&state.pool, run_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(reports))
}

/// Lists run test summaries.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{run_id}/test-summaries",
    params(("run_id" = stormchaser_model::RunId, Path, description="Run ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn list_run_test_summaries(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
) -> Result<impl IntoResponse, StatusCode> {
    let summaries = db::list_run_test_summaries(&state.pool, run_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(summaries))
}

#[utoipa::path(
    get,
    path = "/api/v1/runs/{run_id}/reports/{report_id}",
    params(
        ("run_id" = stormchaser_model::RunId, Path, description = "Run ID"),
        ("report_id" = stormchaser_model::TestReportId, Path, description = "Report ID")
    ),
    responses(
        (status = 200, description = "Test report content"),
        (status = 404, description = "Report not found"),
        (status = 500, description = "Internal Server Error")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "storage"
)]
/// Gets a test report.
pub async fn get_test_report(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path((run_id, report_id)): Path<(RunId, TestReportId)>,
) -> Result<impl IntoResponse, StatusCode> {
    // The run in the path is the one the caller was authorized for
    let report = db::get_test_report(&state.pool, run_id, report_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)
        .map(|c: String| serde_json::json!({ "content": c }))?;

    Ok(Json(report))
}
