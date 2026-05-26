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

    let runner_id = event
        .runner_id
        .as_deref()
        .or(instance.runner_id.as_deref())
        .unwrap_or("unknown");

    info!(
        "Step {} (Run {}) is now running on runner {}",
        step_id, run_id, runner_id
    );

    // 2. Use state machine to transition
    if instance.status == stormchaser_model::step::StepStatus::Initializing {
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Initializing>::from_instance(
                instance.clone(),
            );
        let _ = machine.start(&mut *pool.acquire().await?).await?;
    } else if instance.status == stormchaser_model::step::StepStatus::Pending {
        // Fallback in case Initializing was missed
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
                instance.clone(),
            );
        let machine = machine
            .initializing(runner_id.to_string(), &mut *pool.acquire().await?)
            .await?;
        let _ = machine.start(&mut *pool.acquire().await?).await?;
    } else {
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
