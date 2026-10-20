use crate::handler::step::dispatch::find_step;
use crate::handler::{
    archive_workflow, dispatch_pending_steps, fetch_run, fetch_run_context, fetch_step_instance,
};
use crate::step_machine::{state as step_state, StepMachine};
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::Result;
use chrono::Utc;
use opentelemetry::KeyValue;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_dsl::ast::{Step, Workflow};
use stormchaser_model::events::WorkflowFailedEvent;
use stormchaser_model::events::{EventSource, EventType, WorkflowEventType};
use stormchaser_model::step::StepStatus;
use stormchaser_tls::TlsReloader;
use tracing::{error, info};

use crate::handler::step::quota::release_step_quota_for_instance;

use super::completed::{advance_run, after_advance};
use super::helpers::{
    completion_authorized, declared_test_reports, is_terminal_step_status,
    persist_step_test_reports, reporter_is_assigned_runner, step_belongs_to_run,
};

#[tracing::instrument(skip(event, pool, nats_client, tls_reloader), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step failed.
pub async fn handle_step_failed(
    event: stormchaser_model::events::StepFailedEvent,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    let run_id = event.run_id;
    let step_id = event.step_id;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));
    let error_msg = &event.error;
    let exit_code = event.exit_code;

    info!("Step {} (Run {}) failed: {}", step_id, run_id, error_msg);

    let mut tx = pool.begin().await?;

    if crate::db::lock_workflow_run(&mut *tx, run_id)
        .await?
        .is_none()
    {
        return Ok(());
    }

    // Events carry the run's fencing token, but the run's token never changes, so
    // comparing them rejects nothing. What keeps a late or forged result out is the run's
    // row lock, the step's completion token and assigned runner, and the database
    // refusing to move a finished step (see the guard_step_transitions migration).

    let instance = fetch_step_instance(step_id, &mut *tx).await?;
    if !step_belongs_to_run(&instance, run_id) {
        return Ok(());
    }

    // A duplicate, or this event delivered again because the archive that followed it
    // failed: the run may still need archiving
    if is_terminal_step_status(&instance.status) {
        drop(tx);
        return archive_workflow(run_id, pool.clone()).await;
    }

    // A report on an attempt that was put back for a retry, delivered again before the
    // step ran again
    let attempt = crate::db::step_attempt(&mut *tx, step_id).await?;
    if instance.status == StepStatus::Pending && attempt > 1 {
        tracing::debug!(
            "Ignoring failure of run {} step {}: it is waiting to be retried",
            run_id,
            step_id
        );
        return Ok(());
    }

    if !completion_authorized(&mut tx, &instance, event.completion_token.as_deref()).await {
        return Ok(());
    }
    if !reporter_is_assigned_runner(&mut tx, &instance, event.runner_id.as_deref()).await {
        return Ok(());
    }

    // "lost_zombie" is the engine's own verdict on a step whose runner has gone. The
    // event arrives over NATS like any other, so check the verdict still holds: a runner
    // credential cannot use it to end someone else's step, and a step that another runner
    // adopted after the verdict was queued keeps running.
    let lost_zombie = error_msg == "lost_zombie";
    if lost_zombie && !crate::db::steps::step_runner_is_lost(&mut *tx, step_id).await? {
        tracing::warn!(
            "Ignoring lost_zombie event for run {} step {}: its runner is alive",
            run_id,
            step_id
        );
        return Ok(());
    }

    // A step's end can find it anywhere between being handed out and finishing; a
    // pending step has not been handed out, so a report on it is out of turn
    let machine = match StepMachine::<step_state::Unfinished>::load(instance.clone()) {
        Ok(machine) => machine,
        Err(wrong) => {
            tracing::warn!("Ignoring failure of run {}: {}", run_id, wrong);
            return Ok(());
        }
    };

    let _ = release_step_quota_for_instance(&mut *tx, run_id, step_id).await;

    let context = fetch_run_context(run_id, &mut *tx).await?;
    let workflow_ast: Option<Workflow> =
        match serde_json::from_value(context.workflow_definition.clone()) {
            Ok(workflow) => Some(workflow),
            Err(e) => {
                // The run cannot go on without it, so it fails
                error!("Run {}'s workflow definition does not parse: {}", run_id, e);
                None
            }
        };
    let dsl_step = workflow_ast
        .as_ref()
        .and_then(|w| find_step(&w.steps, &instance.step_name));

    // A step with retries left runs again once its delay is over, and its run waits
    if let Some(delay) = retry_delay(dsl_step, &instance.step_type, attempt) {
        machine.retry(delay, &mut tx).await?;
        tx.commit().await?;
        crate::STEPS_RETRIED.add(
            1,
            &[
                KeyValue::new("step_name", instance.step_name.clone()),
                KeyValue::new("step_type", instance.step_type.clone()),
            ],
        );
        info!(
            "Step {} (Run {}) failed on attempt {}; running it again in {:?}",
            step_id, run_id, attempt, delay
        );
        return Ok(());
    }

    let failed = if lost_zombie {
        let _ = machine.zombify(&mut *tx).await?;
        None
    } else {
        Some(
            machine
                .fail(error_msg.to_string(), exit_code, &mut *tx)
                .await?,
        )
    };

    let step_name = instance.step_name.clone();
    let attributes = [
        KeyValue::new("step_name", instance.step_name),
        KeyValue::new("step_type", instance.step_type),
        KeyValue::new("runner_id", instance.runner_id.unwrap_or_default()),
        KeyValue::new("error", error_msg.to_string()),
    ];

    crate::STEPS_FAILED.add(1, &attributes);

    if let Some(started_at) = instance.started_at {
        let duration = (Utc::now() - started_at).to_std().unwrap_or_default();
        crate::STEP_DURATION.record(duration.as_secs_f64(), &attributes);
    }

    if let Some(outputs) = event
        .outputs
        .as_ref()
        .map(|m| {
            m.clone()
                .into_iter()
                .collect::<serde_json::Map<String, serde_json::Value>>()
        })
        .as_ref()
    {
        for (key, value) in outputs {
            crate::db::upsert_step_output(&mut *tx, step_id, key, value).await?;
        }
    }

    // Persist test reports even on failure
    if event.test_reports.is_some() {
        let declared_reports = declared_test_reports(&mut tx, run_id, &step_name).await;
        persist_step_test_reports(
            event.test_reports.as_ref(),
            &declared_reports,
            &mut tx,
            run_id,
            step_id,
            &pool,
        )
        .await?;
    }

    // A step that sets allow_failure fails without failing its run, which goes on as if
    // the step had succeeded. A runner that is lost is not a step that failed.
    if let Some(failed) = failed {
        let allowed = dsl_step.filter(|s| s.allow_failure == Some(true));
        if let (Some(workflow_ast), Some(dsl_step)) = (&workflow_ast, allowed) {
            failed.ignore_failure(&mut *tx).await?;
            info!(
                "Step {} (Run {}) is allowed to fail; the run continues",
                step_id, run_id
            );
            let all_steps = crate::db::get_step_instances_by_run_id(&mut *tx, run_id).await?;
            let finished = advance_run(
                &mut *tx,
                run_id,
                Some(dsl_step),
                &all_steps,
                &context,
                workflow_ast,
                nats_client.clone(),
                pool.clone(),
                tls_reloader.clone(),
            )
            .await?;
            tx.commit().await?;
            return after_advance(finished, run_id, pool, nats_client, tls_reloader).await;
        }
    }

    let run = fetch_run(run_id, &mut *tx).await?;
    let run_machine = match WorkflowMachine::<state::Running>::load(run.clone()) {
        Ok(machine) => machine,
        Err(wrong) => {
            // It has already ended (it timed out, say); the step's failure is recorded
            tracing::warn!("Not failing run {} for step {}: {}", run_id, step_id, wrong);
            tx.commit().await?;
            return Ok(());
        }
    };
    run_machine
        .fail(format!("Step {} failed: {}", step_id, error_msg), &mut tx)
        .await?;

    use stormchaser_model::nats::NatsSubject;
    if let Err(e) = crate::db::outbox::insert_outbox_event(
        &mut *tx,
        NatsSubject::RunFailed(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
        EventType::Workflow(WorkflowEventType::Failed),
        EventSource::Engine,
        serde_json::to_value(WorkflowFailedEvent {
            run_id,
            event_type: EventType::Workflow(WorkflowEventType::Failed),
            timestamp: chrono::Utc::now(),
            status: stormchaser_model::workflow::RunStatus::Failed,
        })
        .unwrap(),
        None,
        None,
    )
    .await
    {
        error!(
            "Failed to enqueue workflow failed event for {}: {:?}",
            run_id, e
        );
    }

    crate::RUNS_FAILED.add(
        1,
        &[
            KeyValue::new("workflow_name", run.workflow_name),
            KeyValue::new("initiating_user", run.initiating_user),
            KeyValue::new("error", format!("Step {} failed", step_id)),
        ],
    );

    tx.commit().await?;
    info!("Workflow run {} failed, archiving...", run_id);
    archive_workflow(run_id, pool.clone()).await?;

    if let Err(e) = dispatch_pending_steps(run_id, pool, nats_client, tls_reloader).await {
        error!(
            "Failed to dispatch pending steps after failure for run {}: {:?}",
            run_id, e
        );
    }

    Ok(())
}

/// How long to wait before running a step again after its attempt number `attempt`
/// failed, or None when it has no retries left.
fn retry_delay(dsl_step: Option<&Step>, step_type: &str, attempt: i32) -> Option<Duration> {
    // Steps that wait for a person or an event are never run again
    if stormchaser_model::dsl::step_types::waits(step_type) {
        return None;
    }
    let policy = dsl_step?.retry.as_ref()?;
    // Attempt n failing calls for retry number n
    let retry = u32::try_from(attempt).ok().filter(|n| *n >= 1)?;
    (retry <= policy.count).then(|| policy.delay_before(retry))
}

#[cfg(test)]
mod tests {
    use super::*;
    use stormchaser_dsl::StormchaserParser;

    fn step(step_type: &str, retry: &str) -> Step {
        let workflow = StormchaserParser::new()
            .parse(&format!(
                r#"
                stormchaser_dsl_version = "v1"
                workflow "w" {{
                    steps {{
                        step "s" "{step_type}" {{
                            {retry}
                        }}
                    }}
                }}
                "#
            ))
            .unwrap();
        workflow.steps[0].clone()
    }

    #[test]
    fn a_step_is_retried_as_many_times_as_its_policy_says() {
        let s = step(
            "RunContainer",
            "retry {\n count = 2\n backoff = \"linear\"\n delay = \"3s\"\n }",
        );
        let delays: Vec<_> = (1..=3)
            .map(|attempt| retry_delay(Some(&s), &s.r#type, attempt))
            .collect();
        assert_eq!(
            delays,
            vec![
                Some(Duration::from_secs(3)),
                Some(Duration::from_secs(6)),
                None
            ]
        );
    }

    #[test]
    fn steps_without_a_policy_or_that_wait_are_not_retried() {
        let s = step("RunContainer", "");
        assert_eq!(retry_delay(Some(&s), &s.r#type, 1), None);
        assert_eq!(retry_delay(None, "RunContainer", 1), None);
        let approval = step("Approval", "retry {\n count = 2\n }");
        assert_eq!(retry_delay(Some(&approval), "Approval", 1), None);
    }
}
