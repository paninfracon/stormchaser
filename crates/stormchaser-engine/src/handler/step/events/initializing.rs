use crate::handler::fetch_step_instance;
use anyhow::Result;
use sqlx::PgPool;
use tracing::info;

#[tracing::instrument(skip(event, pool), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step initializing.
pub async fn handle_step_initializing(
    event: stormchaser_model::events::StepInitializingEvent,
    pool: PgPool,
) -> Result<()> {
    let run_id = event.run_id;
    let step_id = event.step_id;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));
    let runner_id = event.runner_id.as_deref().unwrap_or("unknown");

    info!(
        "Step {} (Run {}) is now initializing on runner {}",
        step_id, run_id, runner_id
    );

    // 1. Fetch current instance
    let instance = fetch_step_instance(step_id, &pool).await?;

    // 2. Use state machine to transition
    if instance.status == stormchaser_model::step::StepStatus::Pending {
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
                instance.clone(),
            );
        let _ = machine
            .initializing(runner_id.to_string(), &mut *pool.acquire().await?)
            .await?;
    } else {
        info!(
            "Step {} is already past Pending state, ignoring Initializing event.",
            step_id
        );
    }

    Ok(())
}
