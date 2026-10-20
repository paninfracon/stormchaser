//! The transitions each state allows, following the diagram in
//! `docs/state_machines.md`.

pub mod initializing;
pub mod pending;
pub mod running;
pub mod unfinished;
pub mod waiting;

use super::{state, StepMachine};
use anyhow::Result;
use stormchaser_model::step::StepStatus;

impl StepMachine<state::UnpackingSfs> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// The runner has restored the step's storage and started it.
    pub async fn start_running(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Running>> {
        self.instance.status = StepStatus::Running;
        self.save(executor).await
    }
}

impl StepMachine<state::Failed> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// The step allows failure, so its run goes on.
    pub async fn ignore_failure(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::FailedIgnored>> {
        self.instance.status = StepStatus::FailedIgnored;
        self.save(executor).await
    }
}
