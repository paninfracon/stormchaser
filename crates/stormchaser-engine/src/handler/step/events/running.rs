use crate::handler::fetch_step_instance;
use anyhow::Result;
use opentelemetry::KeyValue;
use sqlx::PgPool;
use tracing::info;

#[tracing::instrument(skip(event, pool), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step running.
pub async fn handle_step_running(
    event: stormchaser_model::events::StepRunningEvent,
    pool: PgPool,
) -> Result<()> {
    let run_id = event.run_id;
    let step_id = event.step_id;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));
    // 1. Fetch current instance
    let instance = fetch_step_instance(step_id, &pool).await?;
    if !super::helpers::step_belongs_to_run(&instance, run_id) {
        return Ok(());
    }

    let runner_id = event
        .runner_id
        .as_deref()
        .or(instance.runner_id.as_deref())
        .unwrap_or("unknown");

    info!(
        "Step {} (Run {}) is now running on runner {}",
        step_id, run_id, runner_id
    );

    if !super::helpers::report_is_current_attempt(&pool, &instance, event.attempt).await? {
        return Ok(());
    }

    let mut conn = pool.acquire().await?;

    // 2. Use state machine to transition. Before it runs, only the runner that holds the
    // step (or, while none does, the runner that signed this) may say it has started.
    let starting = matches!(
        instance.status,
        stormchaser_model::step::StepStatus::Pending
            | stormchaser_model::step::StepStatus::Initializing
            | stormchaser_model::step::StepStatus::UnpackingSfs
    );
    if starting && !super::helpers::reporter_holds_step(&instance, event.runner_id.as_deref()) {
        return Ok(());
    }
    if instance.status == stormchaser_model::step::StepStatus::UnpackingSfs {
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::UnpackingSfs>::load(
                instance.clone(),
            )?;
        let _ = machine.start_running(&mut *conn).await?;
    } else if instance.status == stormchaser_model::step::StepStatus::Initializing {
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Initializing>::load(
                instance.clone(),
            )?;
        // Claimed by the engine before it was handed out, so no runner is on it yet
        // unless its initializing report came first
        machine.start(event.runner_id.clone(), &mut conn).await?;
    } else if instance.status == stormchaser_model::step::StepStatus::Pending {
        // Its report that it took the step has not arrived yet
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::load(
            instance.clone(),
        )?
        .start(runner_id.to_string(), &mut conn)
        .await?;
    } else {
        // Already running. If another runner now reports it, that runner picked the step
        // up (it adopted the container or job after a restart, or took the redelivered
        // task), so the step follows it. Otherwise the liveness check would go on judging
        // the step by a runner that no longer has it.
        if let Some(reporting) = event.runner_id.as_deref() {
            // A step is only taken from a runner that has gone quiet. While its runner is
            // alive and heartbeating, another runner saying "it is mine now" is refused:
            // otherwise any runner could take any step, and then report its result.
            if let Some(current) = instance.runner_id.as_deref() {
                if current != reporting
                    && !crate::db::steps::runner_has_gone_quiet(&mut *conn, current).await?
                {
                    tracing::warn!(
                        "Runner {} reports step {} (Run {}) as its own, but runner {} holds it and is alive; ignoring",
                        reporting, step_id, run_id, current
                    );
                    return Ok(());
                }
            }
            if crate::db::steps::reassign_step_runner(&mut *conn, step_id, reporting).await? {
                info!(
                    "Step {} (Run {}) moved from runner {} to runner {}",
                    step_id,
                    run_id,
                    instance.runner_id.as_deref().unwrap_or("unknown"),
                    reporting
                );
                return Ok(());
            }
        }
        info!(
            "Step {} is already past Initializing/Pending state, ignoring Running event.",
            step_id
        );
        return Ok(());
    }

    crate::STEPS_STARTED.add(
        1,
        &[
            KeyValue::new("step_name", instance.step_name),
            KeyValue::new("step_type", instance.step_type),
            KeyValue::new("runner_id", runner_id.to_string()),
        ],
    );

    Ok(())
}
