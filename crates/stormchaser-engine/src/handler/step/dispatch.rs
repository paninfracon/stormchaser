use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::connections::Connection;
use stormchaser_model::connections::ConnectionType;
use stormchaser_model::dsl::Step;
use stormchaser_model::events::{
    EventSource, EventType, SchemaVersion, StepEventType, StepScheduledEvent,
};
use stormchaser_model::nats::publish_cloudevent;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;

use crate::handler::{fetch_run_context, RunContext};

use stormchaser_model::dsl;

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

async fn apply_intrinsic_mutations(
    run_id: RunId,
    step_type: &mut String,
    resolved_spec: &mut Value,
    pool: &PgPool,
) -> Result<()> {
    super::intrinsic::git_checkout::mutate(step_type, resolved_spec, Some(pool)).await?;
    super::intrinsic::jq::mutate_if_has_files(step_type, resolved_spec);
    super::intrinsic::terraform::mutate_if_terraform(run_id.into_inner(), step_type, resolved_spec)
        .await?;
    super::intrinsic::terraform::mutate_if_terraform_approval(step_type, resolved_spec);

    // Inject generic connections
    if step_type == "RunContainer" {
        if let Ok(mut spec) =
            serde_json::from_value::<dsl::CommonContainerSpec>(resolved_spec.clone())
        {
            inject_connection_env_vars(&spec.connections, &mut spec.env, pool).await?;
            if let Ok(new_spec) = serde_json::to_value(spec) {
                *resolved_spec = new_spec;
            }
        }
    } else if step_type == "RunK8sJob" {
        if let Ok(mut spec) = serde_json::from_value::<dsl::K8sJobSpec>(resolved_spec.clone()) {
            inject_connection_env_vars(&spec.connections, &mut spec.env, pool).await?;
            if let Ok(new_spec) = serde_json::to_value(spec) {
                *resolved_spec = new_spec;
            }
        }
    }

    Ok(())
}

async fn inject_connection_env_vars(
    connections: &Option<Vec<String>>,
    env: &mut Option<Vec<dsl::EnvVar>>,
    pool: &PgPool,
) -> Result<()> {
    let Some(connection_names) = connections else {
        return Ok(());
    };

    let mut envs = env.clone().unwrap_or_default();
    for conn_name in connection_names {
        if let Some(conn) = crate::db::connections::get_storage_backend_by_name::<
            _,
            stormchaser_model::Connection,
        >(pool, conn_name)
        .await?
        {
            let prefix = format!(
                "STORMCHASER_CONN_{}_",
                conn_name.to_uppercase().replace("-", "_")
            );
            if let Some(url) = conn.config.get("url").and_then(|v| v.as_str()) {
                envs.push(dsl::EnvVar {
                    name: format!("{}URL", prefix),
                    value: url.to_string(),
                });
            }
            if let Some(user) = conn.config.get("username").and_then(|v| v.as_str()) {
                envs.push(dsl::EnvVar {
                    name: format!("{}USERNAME", prefix),
                    value: user.to_string(),
                });
            }
            if let Some(creds) = &conn.encrypted_credentials {
                envs.push(dsl::EnvVar {
                    name: format!("{}PASSWORD", prefix),
                    value: creds.to_string(),
                });
            }
        }
    }
    *env = Some(envs);
    Ok(())
}

async fn resolve_storage_provision(
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

async fn setup_storage_urls(
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

#[allow(clippy::too_many_arguments)]
async fn try_dispatch_intrinsic(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    fencing_token: i64,
    step_type: &str,
    resolved_spec: &Value,
    resolved_params: &Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<bool> {
    if super::intrinsic::wasm::try_dispatch(
        run_id,
        step_instance_id,
        fencing_token,
        step_type,
        resolved_spec,
        resolved_params,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if super::intrinsic::lambda::try_dispatch(
        run_id,
        step_instance_id,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if super::intrinsic::rest_api::try_dispatch(
        run_id,
        step_instance_id,
        fencing_token,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if super::intrinsic::sql_execute::try_dispatch(
        run_id,
        step_instance_id,
        fencing_token,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if super::intrinsic::webhook::try_dispatch(
        run_id,
        step_instance_id,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if super::intrinsic::slack::try_dispatch(
        run_id,
        step_instance_id,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if super::intrinsic::teams::try_dispatch(
        run_id,
        step_instance_id,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if super::intrinsic::jinja::try_dispatch(
        run_id,
        step_instance_id,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if super::intrinsic::email::try_dispatch(
        run_id,
        step_instance_id,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if super::intrinsic::test_report_email::try_dispatch(
        run_id,
        step_instance_id,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if super::intrinsic::jq::try_dispatch(
        run_id,
        step_instance_id,
        fencing_token,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    Ok(false)
}

async fn setup_test_report_urls(
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

/// Dispatches a step instance, handling intrinsic types or forwarding to runners via NATS.
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_step_instance(
    run_id: RunId,
    step_instance_id: StepInstanceId,
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

    let fencing_token = crate::db::get_workflow_run_fencing_token_by_id(&pool, run_id).await?;

    apply_intrinsic_mutations(run_id, &mut step_type, &mut resolved_spec, &pool).await?;

    let run_context = fetch_run_context(run_id, &pool).await?;
    let workflow: dsl::Workflow = serde_json::from_value(run_context.workflow_definition.clone())
        .context("Failed to parse workflow definition from context")?;

    let storage_urls =
        setup_storage_urls(run_id, &pool, &workflow, &resolved_spec, &run_context).await?;

    if try_dispatch_intrinsic(
        run_id,
        step_instance_id,
        fencing_token,
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

    let mut dsl_step_val = Value::Null;
    if let Some(found_step) = find_step(&workflow.steps, step_name) {
        dsl_step_val = serde_json::to_value(found_step).unwrap_or(Value::Null);
    }

    let test_report_urls =
        setup_test_report_urls(run_id, step_instance_id, step_name, &pool, &workflow).await?;

    let mut registry_auth = None;
    if let Some(reg_conn_name) = resolved_spec
        .get("registry_connection")
        .and_then(|v| v.as_str())
    {
        let conn = crate::db::connections::get_storage_backend_by_name::<
            _,
            stormchaser_model::Connection,
        >(&pool, reg_conn_name)
        .await?
        .with_context(|| format!("Registry connection '{}' not found", reg_conn_name))?;

        if conn.connection_type != stormchaser_model::connections::ConnectionType::Oci
            && conn.connection_type != stormchaser_model::connections::ConnectionType::Jfrog
        {
            anyhow::bail!(
                "Registry connection '{}' has unsupported type {:?}; expected oci or jfrog",
                reg_conn_name,
                conn.connection_type
            );
        }

        let username = conn
            .config
            .get("username")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .with_context(|| {
                format!(
                    "Registry connection '{}' is missing required config.username",
                    reg_conn_name
                )
            })?;
        let password = conn
            .encrypted_credentials
            .filter(|v| !v.is_empty())
            .with_context(|| {
                format!(
                    "Registry connection '{}' is missing encrypted credentials",
                    reg_conn_name
                )
            })?;

        registry_auth = Some(serde_json::json!({
            "username": username,
            "password": password,
            "url": conn.config.get("url").and_then(|v| v.as_str()).unwrap_or(""),
        }));
    }

    let payload = StepScheduledEvent {
        run_id,
        step_id: step_instance_id,
        fencing_token,
        step_name: Some(step_name.to_string()),
        step_type: Some(step_type.clone()),
        spec: Some(resolved_spec),
        params: Some(resolved_params.clone()),
        storage: Some(storage_urls.into_iter().collect()),
        test_report_urls: Some(test_report_urls.into_iter().collect()),
        registry_auth,
        timestamp: Utc::now(),
        event_type: EventType::Step(StepEventType::Scheduled),
        step_dsl: dsl_step_val,
    };

    let js = async_nats::jetstream::new(nats_client);
    use stormchaser_model::nats::NatsSubject;
    let subject = NatsSubject::StepScheduled(
        step_type.clone(),
        Some(stormchaser_model::nats::compute_shard_id(&run_id)),
    );
    publish_cloudevent(
        &js,
        subject,
        EventType::Step(StepEventType::Scheduled),
        EventSource::System,
        serde_json::to_value(payload).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await?;

    Ok(())
}
