use crate::handler::fetch_step_instance;
use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

#[tracing::instrument(skip(payload, pool), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step running.
pub async fn handle_step_running(payload: Value, pool: PgPool) -> Result<()> {
    let run_id_str = payload["run_id"].as_str().context("Missing run_id")?;
    let run_id = Uuid::parse_str(run_id_str)?;
    let step_id_str = payload["step_id"].as_str().context("Missing step_id")?;
    let step_id = Uuid::parse_str(step_id_str)?;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));
    let runner_id = payload["runner_id"].as_str().unwrap_or("unknown");

    info!(
        "Step {} (Run {}) is now running on runner {}",
        step_id, run_id, runner_id
    );

    // 1. Fetch current instance
    let instance = fetch_step_instance(step_id, &pool).await?;

    // 2. Use state machine to transition
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
            instance.clone(),
        );
    let _ = machine
        .start(runner_id.to_string(), &mut *pool.acquire().await?)
        .await?;

    crate::STEPS_STARTED.add(
        1,
        &[
            opentelemetry::KeyValue::new("step_name", instance.step_name),
            opentelemetry::KeyValue::new("step_type", instance.step_type),
            opentelemetry::KeyValue::new("runner_id", runner_id.to_string()),
        ],
    );

    Ok(())
}
