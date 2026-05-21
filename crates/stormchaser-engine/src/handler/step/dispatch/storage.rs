use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;
use stormchaser_model::connections::Connection;
use stormchaser_model::connections::ConnectionType;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;

use crate::handler::RunContext;

use stormchaser_model::dsl;

/// Recursively searches for a step by name within a list of steps.
use super::core::find_step;
pub(crate) async fn resolve_storage_provision(
    run_id: RunId,
    pool: &PgPool,
    storage: &dsl::Storage,
    run_context: &RunContext,
) -> Result<Vec<dsl::Provision>> {
    let mut provision_data = Vec::new();
    for prov in &storage.provision {
        let mut prov_clone = prov.clone();
        if prov_clone.resource_type == "artifact" {
            let (connection_id, remote_path) =
                crate::db::get_artifact_by_name(pool, run_id.into_inner(), &prov_clone.name)
                    .await?
                    .with_context(|| {
                        format!(
                            "Artifact '{}' not found for run {}",
                            prov_clone.name, run_id
                        )
                    })?;

            let backend_info: Connection =
                crate::db::get_storage_backend_by_id(pool, connection_id)
                    .await?
                    .with_context(|| {
                        format!(
                            "Storage backend {} not found for artifact '{}'",
                            connection_id, prov_clone.name
                        )
                    })?;

            if backend_info.connection_type != ConnectionType::S3 {
                anyhow::bail!(
                    "Artifact '{}' requires an S3 backend for provisioning; backend '{}' is not S3",
                    prov_clone.name,
                    backend_info.name
                );
            }

            let bucket = backend_info
                .config
                .get("bucket")
                .and_then(|b| b.as_str())
                .with_context(|| {
                    format!(
                        "Missing 'bucket' in config for backend '{}' (artifact '{}')",
                        backend_info.name, prov_clone.name
                    )
                })?;

            let client = crate::s3::get_s3_client(&backend_info).await?;
            let expires = std::time::Duration::from_secs(3600);
            prov_clone.url = Some(
                crate::s3::generate_presigned_url(&client, bucket, &remote_path, false, expires)
                    .await?,
            );
        } else if let Some(url) = &prov_clone.url {
            let mut val = Value::String(url.clone());
            let hcl_ctx = crate::hcl_eval::create_context(
                run_context.inputs.clone(),
                run_id,
                run_context.secrets.clone(),
                serde_json::json!({}), // Steps value is not needed here
                None,
                None,
            );
            if crate::hcl_eval::resolve_expressions(&mut val, &hcl_ctx, true).is_ok() {
                prov_clone.url = match val {
                    Value::String(s) => Some(s),
                    other => Some(other.to_string()),
                };
            }
        }
        provision_data.push(prov_clone);
    }
    Ok(provision_data)
}

pub(crate) async fn setup_storage_urls(
    run_id: RunId,
    pool: &PgPool,
    workflow: &dsl::Workflow,
    resolved_spec: &Value,
    run_context: &RunContext,
) -> Result<serde_json::Map<String, Value>> {
    let mut storage_urls = serde_json::Map::new();

    let mut mounted_storage_names = std::collections::HashSet::new();
    if let Some(mounts) = resolved_spec
        .get("storage_mounts")
        .and_then(|m| m.as_array())
    {
        for mount in mounts {
            if let Some(name) = mount.get("name").and_then(|n| n.as_str()) {
                mounted_storage_names.insert(name.to_string());
            }
        }
    }

    if workflow.storage.is_empty() {
        return Ok(storage_urls);
    }

    for storage in &workflow.storage {
        if !mounted_storage_names.contains(&storage.name) {
            continue;
        }

        let backend: Option<Connection> = if let Some(ref backend_name) = storage.backend {
            crate::db::get_storage_backend_by_name(pool, backend_name).await?
        } else {
            crate::db::get_default_sfs_backend(pool).await?
        };

        if let Some(backend) = backend {
            let mut get_url = None;
            let mut put_url = None;

            if backend.connection_type == ConnectionType::S3 {
                let client = crate::s3::get_s3_client(&backend).await?;
                let bucket = backend.config["bucket"]
                    .as_str()
                    .context("Missing bucket in SFS backend config")?;

                let key = format!("{}/{}.tar.gz", run_id, storage.name);
                let expires = Duration::from_secs(3600);

                get_url = Some(
                    crate::s3::generate_presigned_url(&client, bucket, &key, false, expires)
                        .await?,
                );
                put_url = Some(
                    crate::s3::generate_presigned_url(&client, bucket, &key, true, expires).await?,
                );
            }

            let last_hash: Option<(String,)> =
                crate::db::get_run_storage_last_hash(pool, run_id.into_inner(), &storage.name)
                    .await?;

            let mut artifacts_data = serde_json::Map::new();
            for artifact in &storage.artifacts {
                let instructions = crate::artifact::generate_parking_instructions(
                    &backend,
                    run_id.into_inner(),
                    &storage.name,
                    artifact,
                )
                .await?;
                artifacts_data.insert(artifact.name.clone(), instructions);
            }

            let provision_data =
                resolve_storage_provision(run_id, pool, storage, run_context).await?;

            let mut preserve = storage.preserve.clone();
            if let Some(mounts) = resolved_spec
                .get("storage_mounts")
                .and_then(|m| m.as_array())
            {
                for mount in mounts {
                    if let Some(name) = mount.get("name").and_then(|n| n.as_str()) {
                        if name == storage.name {
                            if let Some(p) = mount.get("preserve").and_then(|p| p.as_array()) {
                                preserve = p
                                    .iter()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect();
                            }
                        }
                    }
                }
            }

            storage_urls.insert(
                storage.name.clone(),
                serde_json::json!({
                    "get_url": get_url,
                    "put_url": put_url,
                    "expected_hash": last_hash.map(|h| h.0),
                    "artifacts": artifacts_data,
                    "provision": provision_data,
                    "preserve": preserve,
                }),
            );
        }
    }

    Ok(storage_urls)
}

pub(crate) async fn setup_test_report_urls(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    step_name: &str,
    pool: &PgPool,
    workflow: &dsl::Workflow,
) -> Result<serde_json::Map<String, Value>> {
    let mut test_report_urls = serde_json::Map::new();
    if let Some(step) = find_step(&workflow.steps, step_name) {
        if !step.reports.is_empty() {
            let backend = crate::db::get_default_sfs_backend(pool)
                .await?
                .context("Default SFS backend required for test reports")?;
            let client = crate::s3::get_s3_client(&backend).await?;
            let bucket = backend.config["bucket"]
                .as_str()
                .context("Missing bucket")?;

            for report in &step.reports {
                let report_key = format!(
                    "test-reports/{}/{}/{}.tar.gz",
                    run_id, step_instance_id, report.name
                );
                let expires = Duration::from_secs(3600);
                let put_url =
                    crate::s3::generate_presigned_url(&client, bucket, &report_key, true, expires)
                        .await?;

                test_report_urls.insert(
                    report.name.clone(),
                    serde_json::json!({
                        "put_url": put_url,
                        "remote_path": report_key,
                        "connection_id": backend.id,
                    }),
                );
            }
        }
    }
    Ok(test_report_urls)
}
