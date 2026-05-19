use anyhow::Result;
use sqlx::PgPool;
use stormchaser_model::StepInstanceId;

use crate::handler::fetch_step_instance;
use crate::step_machine::{
    state::{Pending, Running},
    StepMachine,
};

pub(super) async fn fail_step_instance(
    step_instance_id: StepInstanceId,
    pool: &PgPool,
    error_message: String,
) -> Result<()> {
    let instance = fetch_step_instance(step_instance_id, pool).await?;
    let machine = StepMachine::<Pending>::from_instance(instance);
    let mut conn = pool.acquire().await?;
    let _machine = machine
        .start("error-recovery".to_string(), &mut *conn)
        .await?;

    let instance = fetch_step_instance(step_instance_id, pool).await?;
    let machine = StepMachine::<Running>::from_instance(instance);
    let _ = machine.fail(error_message, None, &mut *conn).await?;
    Ok(())
}
