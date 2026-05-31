use crate::persistence::persist_step_instance;
use anyhow::Result;
use chrono::Utc;
use std::marker::PhantomData;
use stormchaser_model::step::{StepInstance, StepStatus};

/// State markers for the typestate pattern
pub mod state {
    /// State representing a step that is waiting to be executed.
    pub struct Pending;
    /// State representing a step that has been assigned to a runner and is initializing.
    pub struct Initializing;
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
    /// State representing a step whose runner went offline.
    pub struct LostZombie;
}

/// A state machine for managing the lifecycle of a `StepInstance`.
pub struct StepMachine<S> {
    /// The underlying step instance data model.
    pub instance: StepInstance,
    _state: PhantomData<S>,
}

impl<S> StepMachine<S> {
    /// From instance.
    pub fn from_instance(instance: StepInstance) -> Self {
        Self {
            instance,
            _state: PhantomData,
        }
    }
}

pub mod transitions;

impl StepMachine<state::UnpackingSfs> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Start running.
    pub async fn start_running(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Running>> {
        self.instance.status = StepStatus::Running;

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
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

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
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

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }

    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

impl StepMachine<state::Running> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Zombify.
    pub async fn zombify(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::LostZombie>> {
        self.instance.status = StepStatus::LostZombie;
        self.instance.finished_at = Some(Utc::now());
        self.instance.error = Some("lost_zombie".to_string());

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Start packing.
    pub async fn start_packing(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::PackingSfs>> {
        self.instance.status = StepStatus::PackingSfs;

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
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

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
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

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Wait for event.
    pub async fn wait_for_event(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::WaitingForEvent>> {
        self.instance.status = StepStatus::WaitingForEvent;

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
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

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }

    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

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

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
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

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }

    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

impl StepMachine<state::WaitingForEvent> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Resume.
    pub async fn resume(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Running>> {
        self.instance.status = StepStatus::Running;

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Reschedule.
    pub async fn reschedule(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Pending>> {
        self.instance.status = StepStatus::Pending;

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }

    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
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

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }
}

impl StepMachine<state::Failed> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Ignore failure.
    pub async fn ignore_failure(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::FailedIgnored>> {
        self.instance.status = StepStatus::FailedIgnored;

        persist_step_instance(&self.instance, executor).await?;

        Ok(StepMachine {
            instance: self.instance,
            _state: PhantomData,
        })
    }

    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

impl StepMachine<state::Succeeded> {
    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

impl StepMachine<state::Skipped> {
    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

impl StepMachine<state::FailedIgnored> {
    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

impl StepMachine<state::Aborted> {
    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

impl StepMachine<state::LostZombie> {
    /// Into instance.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stormchaser_model::{RunId, StepInstanceId};
    use uuid::Uuid;

    fn dummy_instance() -> StepInstance {
        StepInstance {
            id: StepInstanceId::new(Uuid::new_v4()),
            run_id: RunId::new(Uuid::new_v4()),
            step_name: "test".to_string(),
            step_type: "docker".to_string(),
            status: StepStatus::Running, // Start with something else
            iteration_index: None,
            runner_id: None,
            affinity_context: None,
            started_at: None,
            finished_at: None,
            exit_code: None,
            error: None,
            spec: serde_json::json!({}),
            params: serde_json::json!({}),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_step_machine_new() {
        let instance = dummy_instance();
        let machine: StepMachine<state::Pending> = StepMachine::new(instance.clone());
        assert_eq!(machine.instance.status, StepStatus::Pending);
        assert_eq!(machine.instance.id, instance.id);
    }

    #[test]
    fn test_step_machine_from_instance() {
        let instance = dummy_instance();
        let machine: StepMachine<state::Running> = StepMachine::from_instance(instance.clone());
        assert_eq!(machine.instance.status, StepStatus::Running);
    }

    #[test]
    fn test_step_machine_into_instance() {
        let instance = dummy_instance();
        let machine: StepMachine<state::Running> = StepMachine::from_instance(instance.clone());
        let extracted = machine.into_instance();
        assert_eq!(extracted.id, instance.id);
        assert_eq!(extracted.status, StepStatus::Running);
    }
}
