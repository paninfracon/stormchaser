use crate::step_machine::{state, StepMachine};
use anyhow::Result;
use stormchaser_model::step::StepStatus;

impl StepMachine<state::WaitingForEvent> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// An iteration held back by a concurrency limit may now be dispatched.
    pub async fn reschedule(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Pending>> {
        self.instance.status = StepStatus::Pending;
        self.save(executor).await
    }
}
