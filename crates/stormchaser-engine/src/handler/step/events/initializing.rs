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
    if !super::helpers::step_belongs_to_run(&instance, run_id) {
        return Ok(());
    }

    if !super::helpers::report_is_current_attempt(&pool, &instance, event.attempt).await? {
        return Ok(());
    }

    // 2. Use state machine to transition
    if instance.status == stormchaser_model::step::StepStatus::Pending {
        if !super::helpers::reporter_holds_step(&instance, event.runner_id.as_deref()) {
            return Ok(());
        }
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::load(
                instance.clone(),
            )?;
        let _ = machine
            .initializing(runner_id.to_string(), &mut *pool.acquire().await?)
            .await?;
    } else if instance.status == stormchaser_model::step::StepStatus::Initializing
        && instance.runner_id.is_none()
    {
        // The engine claimed it before handing it out; this is the runner that took it.
        // Recorded now, so that the step is judged by that runner's liveness from here on
        // and only that runner's results are accepted.
        if let Some(reporter) = event.runner_id.as_deref() {
            crate::db::steps::reassign_step_runner(&pool, step_id, reporter).await?;
        }
    } else {
        info!(
            "Step {} is already past Pending state, ignoring Initializing event.",
            step_id
        );
    }

    Ok(())
}
