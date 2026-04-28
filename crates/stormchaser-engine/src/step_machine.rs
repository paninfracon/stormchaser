use anyhow::Result;
use chrono::Utc;
use stormchaser_model::step::{StepInstance, StepStatus};

/// State markers for the typestate pattern
#[allow(dead_code)]
pub mod state {
    pub struct Pending;
    pub struct UnpackingSfs;
    pub struct Running;
    pub struct PackingSfs;
    pub struct WaitingForEvent;
    pub struct Succeeded;
    pub struct Failed;
    pub struct Skipped;
    pub struct FailedIgnored;
    pub struct Aborted;
}

#[allow(dead_code)]
pub struct StepMachine<S> {
    pub instance: StepInstance,
    _state: std::marker::PhantomData<S>,
}

impl<S> StepMachine<S> {
    pub fn from_instance(instance: StepInstance) -> Self {
        Self {
            instance,
            _state: std::marker::PhantomData,
        }
    }
}

#[allow(dead_code)]
impl StepMachine<state::Pending> {
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

    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::Running> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
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

    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::PackingSfs> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
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

    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::WaitingForEvent> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
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

    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::Failed> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
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

    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::Succeeded> {
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::Skipped> {
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::FailedIgnored> {
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[allow(dead_code)]
impl StepMachine<state::Aborted> {
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}
