use super::{CreateStorageBackendRequest, UpdateStorageBackendRequest};
use crate::db;
use crate::{AppState, AuthClaims};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use stormchaser_model::storage::ArtifactRegistry;
use stormchaser_model::BackendId;
use stormchaser_model::RunId;
use stormchaser_model::TestReportId;
use uuid::Uuid;

/// Creates a storage backend.
#[utoipa::path(
    post,
    path = "/api/v1/storage-backends",
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn create_storage_backend(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Json(payload): Json<CreateStorageBackendRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let id = Uuid::new_v4();

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

    db::create_storage_backend(
        &mut tx,
        BackendId::new(id),
        &payload.name,
        &payload.description,
        &payload.backend_type,
        &payload.config,
        &payload.aws_assume_role_arn,
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

    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

/// List storage backends.
#[utoipa::path(
    get,
    path = "/api/v1/storage-backends",
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn list_storage_backends(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    let backends = db::list_storage_backends(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(backends))
}

/// Get storage backend.
#[utoipa::path(
    get,
    path = "/api/v1/storage-backends/{id}",
    params(("id" = Uuid, Path, description="Backend ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn get_storage_backend(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(id): Path<BackendId>,
) -> Result<impl IntoResponse, StatusCode> {
    let backend = db::get_storage_backend(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(backend))
}

/// Update storage backend.
#[utoipa::path(
    put,
    path = "/api/v1/storage-backends/{id}",
    params(("id" = Uuid, Path, description="Backend ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn update_storage_backend(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(id): Path<BackendId>,
    Json(payload): Json<UpdateStorageBackendRequest>,
) -> Result<impl IntoResponse, StatusCode> {
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

    db::update_storage_backend(&mut tx, id, &payload)
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

/// Deletes a storage backend.
#[utoipa::path(
    delete,
    path = "/api/v1/storage-backends/{id}",
    params(("id" = Uuid, Path, description="Backend ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "storage"
)]
pub async fn delete_storage_backend(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(id): Path<BackendId>,
) -> Result<impl IntoResponse, StatusCode> {
    db::delete_storage_backend(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/artifacts",
    params(
        ("id" = Uuid, Path, description = "Run ID")
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
    Path(id): Path<BackendId>,
) -> Result<impl IntoResponse, StatusCode> {
    let artifacts = db::list_run_artifacts(&state.pool, RunId::new(id.into_inner()))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(artifacts))
}

/// Lists run test reports.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{run_id}/reports",
    params(("run_id" = Uuid, Path, description="Run ID")),
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
    Path(id): Path<BackendId>,
) -> Result<impl IntoResponse, StatusCode> {
    let reports = db::list_run_test_reports(&state.pool, RunId::new(id.into_inner()))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(reports))
}

/// Lists run test summaries.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{run_id}/test-summaries",
    params(("run_id" = Uuid, Path, description="Run ID")),
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
    Path(id): Path<BackendId>,
) -> Result<impl IntoResponse, StatusCode> {
    let summaries = db::list_run_test_summaries(&state.pool, RunId::new(id.into_inner()))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(summaries))
}

#[utoipa::path(
    get,
    path = "/api/v1/runs/{run_id}/reports/{report_id}",
    params(
        ("run_id" = Uuid, Path, description = "Run ID"),
        ("report_id" = Uuid, Path, description = "Report ID")
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
    Path((_run_id, report_id)): Path<(RunId, TestReportId)>,
) -> Result<impl IntoResponse, StatusCode> {
    let report = db::get_test_report(&state.pool, report_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)
        .map(|c: String| serde_json::json!({ "content": c }))?;

    Ok(Json(report))
}
