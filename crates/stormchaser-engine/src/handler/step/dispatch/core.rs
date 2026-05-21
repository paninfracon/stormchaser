use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::dsl::Step;
use stormchaser_model::events::{
    EventSource, EventType, SchemaVersion, StepEventType, StepScheduledEvent,
};
use stormchaser_model::nats::publish_cloudevent;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;

use crate::handler::fetch_run_context;

use stormchaser_model::dsl;

/// Recursively searches for a step by name within a list of steps.
use super::intrinsic::{apply_intrinsic_mutations, try_dispatch_intrinsic};
use super::storage::{setup_storage_urls, setup_test_report_urls};
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
