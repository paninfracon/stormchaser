use crate::handler::fetch_step_instance;
use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use tracing::info;

#[tracing::instrument(skip(payload, pool), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step packing sfs.
pub async fn handle_step_packing_sfs(payload: Value, pool: PgPool) -> Result<()> {
    let run_id_str = payload["run_id"].as_str().context("Missing run_id")?;
    let run_id = uuid::Uuid::parse_str(run_id_str).map(RunId::new)?;
    let step_id_str = payload["step_id"].as_str().context("Missing step_id")?;
    let step_id = uuid::Uuid::parse_str(step_id_str).map(StepInstanceId::new)?;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));

    info!("Step {} (Run {}) is now packing SFS", step_id, run_id);

    let instance = fetch_step_instance(step_id, &pool).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
            instance,
        );
    let _ = machine.start_packing(&mut *pool.acquire().await?).await?;

    Ok(())
}
