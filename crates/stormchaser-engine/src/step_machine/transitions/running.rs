use crate::step_machine::{state, StepMachine};
use anyhow::Result;
use stormchaser_model::step::StepStatus;

impl StepMachine<state::Running> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// The step's command has finished and the runner is parking its storage.
    pub async fn start_packing(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::PackingSfs>> {
        self.instance.status = StepStatus::PackingSfs;
        self.save(executor).await
    }
}
