use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::dsl::Step;
use stormchaser_model::events::{
    EventSource, EventType, SchemaVersion, StepEventType, StepScheduledEvent,
};
use stormchaser_model::nats::publish_cloudevent_with_sequence;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;

use crate::handler::fetch_run_context;

use stormchaser_model::dsl;

/// Recursively searches for a step by name within a list of steps.
use super::intrinsic::{apply_intrinsic_mutations, try_dispatch_intrinsic, BuiltInStep};
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

/// Why a step of type `dispatched` (after the engine's rewrites) cannot be handed to a
/// runner, or `None` when it can.
fn undispatchable(dispatched: &str) -> Option<String> {
    use stormchaser_model::dsl::step_types::{step_type, Executor};
    let executor = step_type(dispatched)?.executor;
    let reason = match executor {
        Executor::Runner => return None,
        Executor::Engine => format!("this engine build does not run {} steps", dispatched),
        Executor::Rewritten => format!(
            "a {} step needs a spec the engine can turn into a container step",
            dispatched
        ),
        Executor::Waits => format!("a {} step only waits, and cannot be dispatched", dispatched),
        Executor::Planned => format!("{} steps are not supported yet", dispatched),
    };
    Some(reason)
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

    let storage_urls = setup_storage_urls(
        run_id,
        &pool,
        &workflow,
        step_name,
        &resolved_spec,
        &run_context,
    )
    .await?;

    let built_in = BuiltInStep {
        run_id,
        step_id: step_instance_id,
        fencing_token,
        step_type: step_type.clone(),
        spec: resolved_spec.clone(),
        params: resolved_params.clone(),
        pool: pool.clone(),
        tls_reloader: tls_reloader.clone(),
    };
    if try_dispatch_intrinsic(built_in).await? {
        return Ok(());
    }

    // Only runners take what is left. A built-in type that no runner runs would wait for
    // one until its run timed out; a type that is not built in may be a runner's own.
    if let Some(reason) = undispatchable(&step_type) {
        tracing::warn!("Run {}: step {}: {}", run_id, step_name, reason);
        crate::handler::integrations::utils::report_step_failed(
            &pool,
            run_id,
            step_instance_id,
            fencing_token,
            reason,
        )
        .await?;
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
        crate::authz::ensure_run_may_use(&pool, run_id, &conn).await?;

        if conn.connection_type != stormchaser_model::connections::ConnectionType::Oci
            && conn.connection_type != stormchaser_model::connections::ConnectionType::Jfrog
        {
            anyhow::bail!(
                "Registry connection '{}' has unsupported type {:?}; expected oci or jfrog",
                reg_conn_name,
                conn.connection_type
            );
        }

        // The runtime sends pull credentials to whichever registry the image names, so they
        // only leave the engine with an image on the connection's own registry.
        let registry_url = conn
            .config
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let image = resolved_spec
            .get("image")
            .and_then(|v| v.as_str())
            .with_context(|| {
                format!(
                    "Registry connection '{}' is set on a step with no image",
                    reg_conn_name
                )
            })?;
        if !stormchaser_model::registry::image_is_on_registry(image, registry_url) {
            anyhow::bail!(
                "Image '{}' is not on the registry of connection '{}' ({}); its credentials are only used for that registry",
                image,
                reg_conn_name,
                stormchaser_model::registry::registry_url_host(registry_url)
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
            .credentials
            .filter(|v| !v.is_empty())
            .with_context(|| {
                format!(
                    "Registry connection '{}' is missing encrypted credentials",
                    reg_conn_name
                )
            })?;

        // The runner pulls with it; a step that gets hold of it must not see it echoed
        crate::db::add_sensitive_values(&pool, run_id, std::slice::from_ref(&password)).await?;
        registry_auth = Some(serde_json::json!({
            "username": username,
            "password": password,
            "url": registry_url,
        }));
    }

    let payload = StepScheduledEvent {
        run_id,
        step_id: step_instance_id,
        attempt: Some(crate::db::step_attempt(&pool, step_instance_id).await?),
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
        step_dsl: dsl_step_val.clone(),
    };

    let js = async_nats::jetstream::new(nats_client);
    use stormchaser_model::nats::NatsSubject;
    let mut subject = NatsSubject::StepScheduled(
        step_type.clone(),
        Some(stormchaser_model::nats::compute_shard_id(&run_id)),
    );

    if let Some(affinity_context) = dsl_step_val
        .get("strategy")
        .and_then(|s| s.get("affinity"))
        .and_then(|a| a.as_str())
    {
        if affinity_context == "shared" {
            // Check whether a runner has already been established for this affinity group.
            let mut maybe_runner_id =
                crate::db::get_affinity_runner_id(&pool, run_id, affinity_context)
                    .await
                    .ok()
                    .flatten();

            if maybe_runner_id.is_none() {
                // No runner established yet. Elect a leader: the earliest-created
                // in-flight step for this affinity group proceeds with load-balanced
                // routing, while followers wait for that leader's runner_id to appear.
                // This prevents concurrent dispatches from routing to different runners.
                if let Ok(Some(leader_id)) =
                    crate::db::get_affinity_leader_step_id(&pool, run_id, affinity_context).await
                {
                    if leader_id != step_instance_id {
                        // We are a follower: wait up to 30 s for the leader's runner_id.
                        for _ in 0..60u32 {
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            if let Ok(Some(rid)) =
                                crate::db::get_affinity_runner_id(&pool, run_id, affinity_context)
                                    .await
                            {
                                maybe_runner_id = Some(rid);
                                break;
                            }
                        }
                    }
                }
            }

            // The group's steps go to the runner its first step landed on, on the subject
            // for that runner's kind: a container step can land on a Kubernetes runner too.
            if let Some(runner_id) = maybe_runner_id {
                match crate::db::get_runner(&pool, &runner_id).await {
                    Ok(Some(runner)) if matches!(runner.runner_type.as_str(), "k8s" | "docker") => {
                        subject = NatsSubject::Custom(format!(
                            "stormchaser.v1.runner.{}.{}",
                            runner.runner_type, runner_id
                        ));
                    }
                    _ => tracing::warn!(
                        "Affinity runner {} for run {} is not a known runner; dispatching step {} to any runner",
                        runner_id, run_id, step_instance_id
                    ),
                }
            }
        }
    }

    // From here the runner owns the step and reports its result without a token.
    crate::db::clear_step_completion_token(&pool, step_instance_id).await?;
    crate::db::mark_step_published(&pool, step_instance_id).await?;

    let stream_seq = publish_cloudevent_with_sequence(
        &js,
        subject,
        EventType::Step(StepEventType::Scheduled),
        EventSource::System,
        serde_json::to_value(payload).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await?;

    // The message holds the step's secrets; remember where it is so it can be erased
    // from the stream when the step is done.
    match stream_seq {
        Some(seq) => {
            if let Err(e) = crate::db::task_messages::record_task_message(
                &pool,
                seq,
                step_instance_id.into_inner(),
            )
            .await
            {
                tracing::warn!(
                    "Could not record the task message of step {}; it stays in the stream until it ages out: {:?}",
                    step_instance_id,
                    e
                );
            }
        }
        None => tracing::warn!(
            "The stream did not confirm the task message of step {}; it cannot be erased early",
            step_instance_id
        ),
    }

    Ok(())
}
