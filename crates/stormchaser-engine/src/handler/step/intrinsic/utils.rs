use anyhow::Result;
use sqlx::PgPool;
use stormchaser_model::{RunId, StepInstanceId};

/// Reports that a step the engine runs itself has failed. The failure goes through the
/// step-failed handler, as a runner's would, so that the run's failure handling follows;
/// marking the step failed here instead would leave the handler nothing to do.
pub(super) async fn fail_step_instance(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    pool: &PgPool,
    nats_client: async_nats::Client,
    error_message: String,
) -> Result<()> {
    crate::handler::integrations::utils::publish_step_failed_event(
        run_id,
        step_instance_id,
        0,
        error_message,
        pool,
        nats_client,
    )
    .await
}
