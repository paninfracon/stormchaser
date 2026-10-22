use crate::step_machine::{state, StepMachine};
use anyhow::Result;
use chrono::Utc;
use stormchaser_model::step::StepStatus;

impl StepMachine<state::Pending> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// A runner reports it has taken the step.
    pub async fn initializing(
        mut self,
        runner_id: String,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Initializing>> {
        self.instance.status = StepStatus::Initializing;
        self.instance.runner_id = Some(runner_id);
        self.save(executor).await
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// A runner reports it has started the step, before its report that it took it.
    pub async fn start(
        mut self,
        runner_id: String,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Running>> {
        self.instance.status = StepStatus::Running;
        self.instance.started_at = Some(Utc::now());
        self.instance.runner_id = Some(runner_id);
        self.save(executor).await
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// A runner reports it is restoring the step's storage, before its report that it
    /// took it.
    pub async fn start_unpacking(
        mut self,
        runner_id: String,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::UnpackingSfs>> {
        self.instance.status = StepStatus::UnpackingSfs;
        self.instance.started_at = Some(Utc::now());
        self.instance.runner_id = Some(runner_id);
        self.save(executor).await
    }

    #[tracing::instrument(skip(self, executor, reason), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Fails a step that never started (its run timed out).
    pub async fn fail(
        mut self,
        reason: String,
        exit_code: Option<i32>,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Failed>> {
        self.finish(StepStatus::Failed, Some(reason), exit_code);
        self.save(executor).await
    }
}
