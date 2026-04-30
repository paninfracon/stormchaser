use anyhow::Result;
use chrono::Utc;
use stormchaser_model::step::{StepInstance, StepStatus};

/// State markers for the typestate pattern
#[allow(dead_code)]
pub mod state {
    /// State representing a step that is waiting to be executed.
    pub struct Pending;
    /// State representing a step that is unpacking its state from the Stormchaser File System.
    pub struct UnpackingSfs;
    /// State representing a step that is currently executing.
    pub struct Running;
    /// State representing a step that is packing its state into the Stormchaser File System.
    pub struct PackingSfs;
    /// State representing a step that is waiting for an external event (e.g., Human-In-The-Loop approval).
    pub struct WaitingForEvent;
    /// State representing a step that has successfully completed.
    pub struct Succeeded;
    /// State representing a step that has failed.
    pub struct Failed;
    /// State representing a step that has been skipped (e.g., due to unmet conditions).
    pub struct Skipped;
    /// State representing a step that failed but the failure was configured to be ignored.
    pub struct FailedIgnored;
    /// State representing a step that has been aborted.
    pub struct Aborted;
}

/// A state machine for managing the lifecycle of a `StepInstance`.
pub struct StepMachine<S> {
    /// The underlying step instance data model.
    pub instance: StepInstance,
    _state: std::marker::PhantomData<S>,
}

impl<S> StepMachine<S> {
    /// From instance.
    pub fn from_instance(instance: StepInstance) -> Self {
        Self {
            instance,
            _state: std::marker::PhantomData,
        }
    }
}

#[allow(dead_code)]
impl StepMachine<state::Pending> {
    /// New.
    pub fn new(instance: StepInstance) -> Self {
        // Ensure the initial status is correct
        let mut instance = instance;
        instance.status = StepStatus::Pending;

        StepMachine {
            instance,
            _state: std::marker::PhantomData,
        }
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Start.
    pub async fn start(
        mut self,
        runner_id: String,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Running>> {
        self.instance.status = StepStatus::Running;
        self.instance.started_at = Some(Utc::now());
        self.instance.runner_id = Some(runner_id);

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Start unpacking.
    pub async fn start_unpacking(
        mut self,
        runner_id: String,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::UnpackingSfs>> {
        self.instance.status = StepStatus::UnpackingSfs;
        self.instance.started_at = Some(Utc::now());
        self.instance.runner_id = Some(runner_id);

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }
}

#[allow(dead_code)]
impl StepMachine<state::UnpackingSfs> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Start running.
    pub async fn start_running(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Running>> {
        self.instance.status = StepStatus::Running;

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Fail.
    pub async fn fail(
        mut self,
        error: String,
        exit_code: Option<i32>,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Failed>> {
        self.instance.status = StepStatus::Failed;
        self.instance.finished_at = Some(Utc::now());
        self.instance.error = Some(error);
        self.instance.exit_code = exit_code;

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Skip.
    pub async fn skip(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Skipped>> {
        self.instance.status = StepStatus::Skipped;
        self.instance.finished_at = Some(Utc::now());

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::Running> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Start packing.
    pub async fn start_packing(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::PackingSfs>> {
        self.instance.status = StepStatus::PackingSfs;

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Succeed.
    pub async fn succeed(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Succeeded>> {
        self.instance.status = StepStatus::Succeeded;
        self.instance.finished_at = Some(Utc::now());
        self.instance.exit_code = Some(0);

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Fail.
    pub async fn fail(
        mut self,
        error: String,
        exit_code: Option<i32>,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Failed>> {
        self.instance.status = StepStatus::Failed;
        self.instance.finished_at = Some(Utc::now());
        self.instance.error = Some(error);
        self.instance.exit_code = exit_code;

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Wait for event.
    pub async fn wait_for_event(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::WaitingForEvent>> {
        self.instance.status = StepStatus::WaitingForEvent;

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Abort.
    pub async fn abort(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Aborted>> {
        self.instance.status = StepStatus::Aborted;
        self.instance.finished_at = Some(Utc::now());

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::PackingSfs> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Succeed.
    pub async fn succeed(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Succeeded>> {
        self.instance.status = StepStatus::Succeeded;
        self.instance.finished_at = Some(Utc::now());
        self.instance.exit_code = Some(0);

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Fail.
    pub async fn fail(
        mut self,
        error: String,
        exit_code: Option<i32>,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Failed>> {
        self.instance.status = StepStatus::Failed;
        self.instance.finished_at = Some(Utc::now());
        self.instance.error = Some(error);
        self.instance.exit_code = exit_code;

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::WaitingForEvent> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Resume.
    pub async fn resume(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Running>> {
        self.instance.status = StepStatus::Running;

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Reschedule.
    pub async fn reschedule(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Pending>> {
        self.instance.status = StepStatus::Pending;

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::Failed> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Ignore failure.
    pub async fn ignore_failure(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::FailedIgnored>> {
        self.instance.status = StepStatus::FailedIgnored;

        crate::persistence::persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: std::marker::PhantomData,
        })
    }

    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::Succeeded> {
    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::Skipped> {
    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::FailedIgnored> {
    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::Aborted> {
    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}
