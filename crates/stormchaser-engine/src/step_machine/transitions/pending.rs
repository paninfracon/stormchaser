use crate::persistence::persist_step_instance;
use crate::step_machine::{state, StepMachine};
use anyhow::Result;
use chrono::Utc;
use std::marker::PhantomData;
use stormchaser_model::step::StepInstance;
use stormchaser_model::step::StepStatus;

impl StepMachine<state::Pending> {
    /// New.
    pub fn new(instance: StepInstance) -> Self {
        // Ensure the initial status is correct
        let mut instance = instance;
        instance.status = StepStatus::Pending;

        StepMachine {
            instance,
            _state: PhantomData,
        }
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Initializing.
    pub async fn initializing(
        mut self,
        runner_id: String,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Initializing>> {
        self.instance.status = StepStatus::Initializing;
        self.instance.runner_id = Some(runner_id);

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Start (shortcut for intrinsic steps that skip Initializing).
    pub async fn start(
        mut self,
        runner_id: String,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Running>> {
        self.instance.status = StepStatus::Running;
        self.instance.started_at = Some(Utc::now());
        self.instance.runner_id = Some(runner_id);

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Fail immediately from Pending (e.g., timeout or bad intrinsic setup).
    pub async fn fail(
        mut self,
        reason: String,
        exit_code: Option<i32>,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Failed>> {
        self.instance.status = StepStatus::Failed;
        self.instance.error = Some(reason);
        self.instance.exit_code = exit_code;
        self.instance.finished_at = Some(Utc::now());

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Start unpacking (shortcut for when Initializing is skipped).
    pub async fn start_unpacking(
        mut self,
        runner_id: String,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::UnpackingSfs>> {
        self.instance.status = StepStatus::UnpackingSfs;
        self.instance.started_at = Some(Utc::now());
        self.instance.runner_id = Some(runner_id);

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }
}
