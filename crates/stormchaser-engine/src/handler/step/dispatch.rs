use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::dsl::Step;
use stormchaser_tls::TlsReloader;
use uuid::Uuid;

use crate::handler::fetch_run_context;

use stormchaser_model::dsl;
use stormchaser_model::storage;
use stormchaser_model::storage::BackendType;

/// Recursively searches for a step by name within a list of steps.
pub fn find_step<'a>(steps: &'a [Step], name: &str) -> Option<&'a Step> {
    for step in steps {
        if step.name == name {
            return Some(step);
        }
        if let Some(inner) = &step.steps {
            if let Some(found) = find_step(inner, name) {
                return Some(found);
            }
        }
    }
    None
}

/// Dispatches a step instance, handling intrinsic types or forwarding to runners via NATS.
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_step_instance(
    run_id: Uuid,
    step_instance_id: Uuid,
    step_name: &str,
    step_type: &str,
    resolved_spec: &Value,
    resolved_params: &Value,
    nats_client: async_nats::Client,
    pool: PgPool,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    let mut step_type = step_type.to_string();
    let mut resolved_spec = resolved_spec.clone();

    super::intrinsic::git_checkout::mutate(&mut step_type, &mut resolved_spec);
    super::intrinsic::jq::mutate_if_has_files(&mut step_type, &mut resolved_spec);
    super::intrinsic::terraform::mutate_if_terraform(run_id, &mut step_type, &mut resolved_spec)
        .await?;
    super::intrinsic::terraform::mutate_if_terraform_approval(&mut step_type, &mut resolved_spec);

    let run_context = fetch_run_context(run_id, &pool).await?;
    let workflow: dsl::Workflow = serde_json::from_value(run_context.workflow_definition.clone())
        .context("Failed to parse workflow definition from context")?;

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

    if !workflow.storage.is_empty() {
        for storage in workflow.storage {
            if !mounted_storage_names.contains(&storage.name) {
                continue;
            }
            let backend: Option<storage::StorageBackend> =
                if let Some(ref backend_name) = storage.backend {
                    crate::db::get_storage_backend_by_name(&pool, backend_name).await?
                } else {
                    crate::db::get_default_sfs_backend(&pool).await?
                };

            if let Some(backend) = backend {
                let mut get_url = None;
                let mut put_url = None;

                if backend.backend_type == BackendType::S3 {
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
                        crate::s3::generate_presigned_url(&client, bucket, &key, true, expires)
                            .await?,
                    );
                }

                let last_hash: Option<(String,)> =
                    crate::db::get_run_storage_last_hash(&pool, run_id, &storage.name).await?;

                let mut artifacts_data = serde_json::Map::new();
                for artifact in storage.artifacts {
                    let instructions = crate::artifact::generate_parking_instructions(
                        &backend,
                        run_id,
                        &storage.name,
                        &artifact,
                    )
                    .await?;
                    artifacts_data.insert(artifact.name.clone(), instructions);
                }

                let mut provision_data = Vec::new();
                for mut prov in storage.provision {
                    if prov.resource_type == "artifact" {
                        // For artifacts, the name corresponds to the artifact_name in the DB
                        let (backend_id, remote_path) =
                            crate::db::get_artifact_by_name(&pool, run_id, &prov.name)
                                .await?
                                .with_context(|| {
                                    format!("Artifact '{}' not found for run {}", prov.name, run_id)
                                })?;
                        let backend_info: stormchaser_model::storage::StorageBackend =
                            crate::db::get_storage_backend_by_id(&pool, backend_id)
                                .await?
                                .with_context(|| {
                                    format!(
                                        "Storage backend {} not found for artifact '{}'",
                                        backend_id, prov.name
                                    )
                                })?;
                        if backend_info.backend_type != stormchaser_model::storage::BackendType::S3
                        {
                            anyhow::bail!(
                                "Artifact '{}' requires an S3 backend for provisioning; \
                                 backend '{}' is not S3",
                                prov.name,
                                backend_info.name
                            );
                        }
                        let bucket = backend_info
                            .config
                            .get("bucket")
                            .and_then(|b: &serde_json::Value| b.as_str())
                            .with_context(|| {
                                format!(
                                    "Missing 'bucket' in config for backend '{}' (artifact '{}')",
                                    backend_info.name, prov.name
                                )
                            })?;
                        let client = crate::s3::get_s3_client(&backend_info).await?;
                        let expires = std::time::Duration::from_secs(3600);
                        prov.url = Some(
                            crate::s3::generate_presigned_url(
                                &client,
                                bucket,
                                &remote_path,
                                false,
                                expires,
                            )
                            .await?,
                        );
                    } else if let Some(url) = &prov.url {
                        let mut val = Value::String(url.clone());
                        let hcl_ctx = crate::hcl_eval::create_context(
                            run_context.inputs.clone(),
                            run_id,
                            run_context.secrets.clone(),
                        );
                        if crate::hcl_eval::resolve_expressions(&mut val, &hcl_ctx).is_ok() {
                            prov.url = match val {
                                Value::String(s) => Some(s),
                                other => Some(other.to_string()),
                            };
                        }
                    }
                    provision_data.push(prov);
                }

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
    }

    if super::intrinsic::wasm::try_dispatch(
        run_id,
        step_instance_id,
        &step_type,
        &resolved_spec,
        resolved_params,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(());
    }

    if super::intrinsic::lambda::try_dispatch(
        run_id,
        step_instance_id,
        &step_type,
        &resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(());
    }

    if super::intrinsic::webhook::try_dispatch(
        run_id,
        step_instance_id,
        &step_type,
        &resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(());
    }

    if super::intrinsic::jinja::try_dispatch(
        run_id,
        step_instance_id,
        &step_type,
        &resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(());
    }

    if super::intrinsic::email::try_dispatch(
        run_id,
        step_instance_id,
        &step_type,
        &resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(());
    }

    if super::intrinsic::test_report_email::try_dispatch(
        run_id,
        step_instance_id,
        &step_type,
        &resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(());
    }

    if super::intrinsic::jq::try_dispatch(
        run_id,
        step_instance_id,
        &step_type,
        &resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(());
    }

    let mut dsl_step_val = Value::Null;
    if let Some(found_step) = find_step(&workflow.steps, step_name) {
        dsl_step_val = serde_json::to_value(found_step).unwrap_or(Value::Null);
    }

    let mut test_report_urls = serde_json::Map::new();
    if let Some(step) = workflow.steps.iter().find(|s| s.name == step_name) {
        if !step.reports.is_empty() {
            let backend = crate::db::get_default_sfs_backend(&pool)
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
                        "backend_id": backend.id,
                    }),
                );
            }
        }
    }

    let payload = stormchaser_model::events::StepScheduledEvent {
        run_id,
        step_id: step_instance_id,
        step_name: Some(step_name.to_string()),
        step_type: Some(step_type.clone()),
        spec: Some(resolved_spec),
        params: Some(resolved_params.clone()),
        storage: Some(storage_urls.into_iter().collect()),
        test_report_urls: Some(test_report_urls.into_iter().collect()),
        timestamp: Utc::now(),
        event_type: "stormchaser.v1.step.scheduled".to_string(),
        step_dsl: dsl_step_val,
    };

    let js = async_nats::jetstream::new(nats_client);
    let subject = format!("stormchaser.v1.step.scheduled.{}", step_type.to_lowercase());
    stormchaser_model::nats::publish_cloudevent(
        &js,
        &subject,
        &subject,
        "/stormchaser",
        serde_json::to_value(payload).unwrap(),
        Some("1.0"),
        None,
    )
    .await?;
    Ok(())
}
