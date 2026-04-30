use std::collections::HashMap;
use stormchaser_model::cron;

use super::{CreateCronWorkflowRequest, CronWorkflowResponse, EnqueueResponse};
use crate::{AppState, AuthClaims};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use k8s_openapi::api::batch::v1::{CronJob, CronJobSpec, JobSpec, JobTemplateSpec};
use k8s_openapi::api::core::v1::{Container, PodSpec, PodTemplateSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{
    api::{DeleteParams, PostParams},
    Api, Client,
};
use stormchaser_model::workflow::RunStatus;
use uuid::Uuid;

/// Create cron workflow.
pub async fn create_cron_workflow(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Json(payload): Json<CreateCronWorkflowRequest>,
) -> Result<Json<CronWorkflowResponse>, StatusCode> {
    let id = Uuid::new_v4();
    let secret_token = Uuid::new_v4().to_string();

    // 1. Register with external cron engine
    let external_job_id =
        register_external_cron(id, &payload.name, &payload.cronspec, &secret_token).await?;

    // 2. Save to database
    sqlx::query(
        r#"
        INSERT INTO cron_workflows (id, name, description, cronspec, workflow_name, repo_url, workflow_path, git_ref, inputs, secret_token, external_job_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
    )
    .bind(id)
    .bind(&payload.name)
    .bind(&payload.description)
    .bind(&payload.cronspec)
    .bind(&payload.workflow_name)
    .bind(&payload.repo_url)
    .bind(&payload.workflow_path)
    .bind(&payload.git_ref)
    .bind(&payload.inputs)
    .bind(&secret_token)
    .bind(&external_job_id)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create cron workflow: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(CronWorkflowResponse {
        id,
        secret_token,
        external_job_id,
    }))
}

/// List cron workflows.
pub async fn list_cron_workflows(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
) -> Result<Json<Vec<cron::CronWorkflow>>, StatusCode> {
    let workflows = crate::db::list_cron_workflows(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(workflows))
}

/// Deletes a cron workflow.
pub async fn delete_cron_workflow(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    // 1. Fetch to get external_job_id
    let workflow = crate::db::get_cron_workflow(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let workflow = match workflow {
        Some(w) => w,
        None => return Err(StatusCode::NOT_FOUND),
    };

    // 2. Unregister from external cron engine (K8s)
    if let Some(ext_id) = workflow.external_job_id {
        unregister_external_cron(&ext_id).await?;
    }

    // 3. Delete from DB
    crate::db::delete_cron_workflow(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Trigger cron workflow.
pub async fn trigger_cron_workflow(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EnqueueResponse>, StatusCode> {
    // 1. Fetch CronWorkflow
    let cron = crate::db::get_active_cron_workflow(&state.pool, id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // 2. Validate Token (using constant-time approach via hashing)
    let auth_header = headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let expected = format!("Bearer {}", cron.secret_token);

    use sha2::{Digest, Sha256};
    let mut hasher1 = Sha256::new();
    hasher1.update(auth_header.as_bytes());
    let hash1 = hasher1.finalize();

    let mut hasher2 = Sha256::new();
    hasher2.update(expected.as_bytes());
    let hash2 = hasher2.finalize();

    if hash1 != hash2 {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // 3. Enqueue a new run
    let run_id = Uuid::new_v4();

    tracing::info!(run_id = %run_id, "Enqueuing cron workflow: {}", cron.workflow_name);
    let fencing_token = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    crate::db::insert_workflow_run(
        &mut tx,
        run_id,
        &cron.workflow_name,
        "system:cron",
        &cron.repo_url,
        &cron.workflow_path,
        &cron.git_ref,
        RunStatus::Queued,
        fencing_token,
    )
    .await
    .inspect_err(|e| tracing::error!(run_id = %run_id, "Database error inserting run: {:?}", e))
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    crate::db::insert_run_context(
        &mut tx,
        run_id,
        "v1",
        serde_json::json!({}),
        "",
        &cron.inputs,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    crate::db::insert_run_quotas(&mut tx, run_id, 10, "1", "4Gi", "10Gi", "1h")
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

    Ok(Json(EnqueueResponse {
        run_id,
        status: "queued".to_string(),
    }))
}

async fn register_ofelia_cron(
    id: Uuid,
    name: &str,
    cronspec: &str,
    secret_token: &str,
) -> Result<Option<String>, StatusCode> {
    use bollard::container::{Config, CreateContainerOptions};
    use bollard::Docker;

    let docker = Docker::connect_with_local_defaults().map_err(|e| {
        tracing::error!("Failed to connect to Docker: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let system_url =
        std::env::var("SYSTEM_URL").unwrap_or_else(|_| "http://localhost:3000".to_string());
    let trigger_url = format!("{}/api/v1/cron-trigger/{}", system_url, id);
    let container_name = format!("stormchaser-cron-{}", id);

    let mut labels = HashMap::new();
    labels.insert("ofelia.enabled".to_string(), "true".to_string());
    labels.insert(
        format!("ofelia.job-run.{}.schedule", name),
        cronspec.to_string(),
    );
    labels.insert(
        format!("ofelia.job-run.{}.image", name),
        "curlimages/curl:latest".to_string(),
    );
    labels.insert(
        format!("ofelia.job-run.{}.command", name),
        format!(
            "-X POST -H \"Authorization: Bearer {}\" {}",
            secret_token, trigger_url
        ),
    );

    let config = Config {
        image: Some("curlimages/curl:latest".to_string()),
        labels: Some(labels),
        entrypoint: Some(vec!["/bin/true".to_string()]),
        ..Default::default()
    };

    docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                ..Default::default()
            }),
            config,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create Ofelia cron container: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Some(container_name))
}

async fn unregister_ofelia_cron(container_name: &str) -> Result<(), StatusCode> {
    use bollard::container::RemoveContainerOptions;
    use bollard::Docker;

    let docker =
        Docker::connect_with_local_defaults().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    docker
        .remove_container(
            container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to remove Ofelia cron container: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(())
}

async fn register_external_cron(
    id: Uuid,
    name: &str,
    cronspec: &str,
    secret_token: &str,
) -> Result<Option<String>, StatusCode> {
    let engine = std::env::var("CRON_ENGINE").unwrap_or_else(|_| "kubernetes".to_string());

    if engine == "none" {
        tracing::info!(
            "Cron engine set to 'none', skipping external registration for {}",
            id
        );
        return Ok(None);
    }

    if engine == "ofelia" {
        return register_ofelia_cron(id, name, cronspec, secret_token).await;
    }

    if engine != "kubernetes" {
        tracing::error!("Unsupported CRON_ENGINE: {}", engine);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let client = Client::try_default().await.map_err(|e| {
        tracing::error!("Failed to initialize K8s client for cron: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let namespace = std::env::var("KUBERNETES_NAMESPACE").unwrap_or_else(|_| "default".to_string());
    let cronjobs: Api<CronJob> = Api::namespaced(client, &namespace);

    let system_url = std::env::var("SYSTEM_URL")
        .unwrap_or_else(|_| "http://stormchaser-api.default.svc.cluster.local".to_string());
    let trigger_url = format!("{}/api/v1/cron-trigger/{}", system_url, id);
    let auth_header = format!("Authorization: Bearer {}", secret_token);

    let cron_job = CronJob {
        metadata: ObjectMeta {
            name: Some(format!("stormchaser-{}", id)),
            labels: Some(std::collections::BTreeMap::from([
                (
                    "app.kubernetes.io/managed-by".to_string(),
                    "stormchaser".to_string(),
                ),
                (
                    "stormchaser.paninfracon.net/workflow-name".to_string(),
                    name.to_string(),
                ),
            ])),
            ..Default::default()
        },
        spec: Some(CronJobSpec {
            schedule: cronspec.to_string(),
            job_template: JobTemplateSpec {
                metadata: None,
                spec: Some(JobSpec {
                    template: PodTemplateSpec {
                        spec: Some(PodSpec {
                            containers: vec![Container {
                                name: "trigger".to_string(),
                                image: Some("curlimages/curl:latest".to_string()),
                                command: Some(vec![
                                    "curl".to_string(),
                                    "-X".to_string(),
                                    "POST".to_string(),
                                    "-H".to_string(),
                                    auth_header,
                                    trigger_url,
                                ]),
                                ..Default::default()
                            }],
                            restart_policy: Some("OnFailure".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    };

    cronjobs
        .create(&PostParams::default(), &cron_job)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create K8s CronJob: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Some(format!("stormchaser-{}", id)))
}

async fn unregister_external_cron(external_job_id: &str) -> Result<(), StatusCode> {
    let engine = std::env::var("CRON_ENGINE").unwrap_or_else(|_| "kubernetes".to_string());

    if engine == "none" {
        return Ok(());
    }

    if engine == "ofelia" {
        return unregister_ofelia_cron(external_job_id).await;
    }

    if engine != "kubernetes" {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let client = Client::try_default()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let namespace = std::env::var("KUBERNETES_NAMESPACE").unwrap_or_else(|_| "default".to_string());
    let cronjobs: Api<CronJob> = Api::namespaced(client, &namespace);

    cronjobs
        .delete(external_job_id, &DeleteParams::default())
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete K8s CronJob {}: {:?}", external_job_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(())
}
