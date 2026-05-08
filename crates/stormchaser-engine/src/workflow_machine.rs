use anyhow::Result;
use chrono::Utc;
use std::marker::PhantomData;
use stormchaser_model::workflow::{RunStatus, WorkflowRun};

/// State markers for the workflow typestate pattern
#[allow(dead_code)]
pub mod state {
    /// State representing a workflow run that is queued and waiting for resolution.
    pub struct Queued;
    /// State representing a workflow run that is resolving its source repository and dependencies.
    pub struct Resolving;
    /// State representing a workflow run that has been resolved and is pending start.
    pub struct StartPending;
    /// State representing a workflow run that is currently executing.
    pub struct Running;
    /// State representing a workflow run that has successfully completed.
    pub struct Succeeded;
    /// State representing a workflow run that has failed.
    pub struct Failed;
    /// State representing a workflow run that has been aborted.
    pub struct Aborted;
}

/// A state machine for managing the lifecycle of a `WorkflowRun`.
#[allow(dead_code)]
pub struct WorkflowMachine<S> {
    /// The underlying workflow run data model.
    pub run: WorkflowRun,
    _state: PhantomData<S>,
}

#[allow(dead_code)]
impl<S> WorkflowMachine<S> {
    /// New from run.
    pub fn new_from_run(run: WorkflowRun) -> Self {
        Self {
            run,
            _state: PhantomData,
        }
    }

    /// Into run.
    pub fn into_run(self) -> WorkflowRun {
        self.run
    }
}

#[allow(dead_code)]
impl WorkflowMachine<state::Queued> {
    /// New.
    pub fn new(run: WorkflowRun) -> Self {
        let mut run = run;
        run.status = RunStatus::Queued;

        Self {
            run,
            _state: PhantomData,
        }
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// Start resolving.
    pub async fn start_resolving(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Resolving>> {
        self.run.status = RunStatus::Resolving;
        self.run.started_resolving_at = Some(Utc::now());
        self.run.updated_at = Utc::now();

        crate::persistence::persist_run(&mut self.run, executor).await?;

        Ok(WorkflowMachine {
            run: self.run,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// Abort.
    pub async fn abort(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Aborted>> {
        self.run.status = RunStatus::Aborted;
        self.run.finished_at = Some(Utc::now());
        self.run.updated_at = Utc::now();

        crate::persistence::persist_run(&mut self.run, executor).await?;

        Ok(WorkflowMachine {
            run: self.run,
            _state: PhantomData,
        })
    }
}

#[allow(dead_code)]
impl WorkflowMachine<state::Resolving> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// Start pending.
    pub async fn start_pending(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::StartPending>> {
        self.run.status = RunStatus::StartPending;
        self.run.updated_at = Utc::now();

        crate::persistence::persist_run(&mut self.run, executor).await?;

        Ok(WorkflowMachine {
            run: self.run,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// Fail.
    pub async fn fail(
        mut self,
        error: String,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Failed>> {
        self.run.status = RunStatus::Failed;
        self.run.finished_at = Some(Utc::now());
        self.run.updated_at = Utc::now();
        self.run.error = Some(error);

        crate::persistence::persist_run(&mut self.run, executor).await?;

        Ok(WorkflowMachine {
            run: self.run,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// Abort.
    pub async fn abort(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Aborted>> {
        self.run.status = RunStatus::Aborted;
        self.run.finished_at = Some(Utc::now());
        self.run.updated_at = Utc::now();

        crate::persistence::persist_run(&mut self.run, executor).await?;

        Ok(WorkflowMachine {
            run: self.run,
            _state: PhantomData,
        })
    }
}

#[allow(dead_code)]
impl WorkflowMachine<state::StartPending> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// Start.
    pub async fn start(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Running>> {
        self.run.status = RunStatus::Running;
        self.run.started_at = Some(Utc::now());
        self.run.updated_at = Utc::now();

        crate::persistence::persist_run(&mut self.run, executor).await?;

        Ok(WorkflowMachine {
            run: self.run,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// Fail.
    pub async fn fail(
        mut self,
        error: String,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Failed>> {
        self.run.status = RunStatus::Failed;
        self.run.finished_at = Some(Utc::now());
        self.run.updated_at = Utc::now();
        self.run.error = Some(error);

        crate::persistence::persist_run(&mut self.run, executor).await?;

        Ok(WorkflowMachine {
            run: self.run,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// Abort.
    pub async fn abort(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Aborted>> {
        self.run.status = RunStatus::Aborted;
        self.run.finished_at = Some(Utc::now());
        self.run.updated_at = Utc::now();

        crate::persistence::persist_run(&mut self.run, executor).await?;

        Ok(WorkflowMachine {
            run: self.run,
            _state: PhantomData,
        })
    }
}

#[allow(dead_code)]
impl WorkflowMachine<state::Running> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// Succeed.
    pub async fn succeed(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Succeeded>> {
        self.run.status = RunStatus::Succeeded;
        self.run.finished_at = Some(Utc::now());
        self.run.updated_at = Utc::now();

        crate::persistence::persist_run(&mut self.run, executor).await?;

        Ok(WorkflowMachine {
            run: self.run,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// Fail.
    pub async fn fail(
        mut self,
        error: String,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Failed>> {
        self.run.status = RunStatus::Failed;
        self.run.finished_at = Some(Utc::now());
        self.run.updated_at = Utc::now();
        self.run.error = Some(error);

        crate::persistence::persist_run(&mut self.run, executor).await?;

        Ok(WorkflowMachine {
            run: self.run,
            _state: PhantomData,
        })
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// Abort.
    pub async fn abort(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Aborted>> {
        self.run.status = RunStatus::Aborted;
        self.run.finished_at = Some(Utc::now());
        self.run.updated_at = Utc::now();

        crate::persistence::persist_run(&mut self.run, executor).await?;

        Ok(WorkflowMachine {
            run: self.run,
            _state: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stormchaser_model::RunId;

    fn dummy_run() -> WorkflowRun {
        WorkflowRun {
            id: RunId::new_v4(),
            workflow_name: "test".to_string(),
            initiating_user: "u".to_string(),
            status: RunStatus::Succeeded, // start with something else
            created_at: Utc::now(),
            started_resolving_at: None,
            started_at: None,
            finished_at: None,
            updated_at: Utc::now(),
            repo_url: "https://example.com/repo".to_string(),
            workflow_path: "workflow.yaml".to_string(),
            version: 1,
            fencing_token: 1,
            git_ref: "main".to_string(),
            error: None,
        }
    }

    #[test]
    fn test_workflow_machine_new() {
        let run = dummy_run();
        let machine: WorkflowMachine<state::Queued> = WorkflowMachine::new(run.clone());
        assert_eq!(machine.run.status, RunStatus::Queued);
        assert_eq!(machine.run.id, run.id);
    }

    #[test]
    fn test_workflow_machine_new_from_run() {
        let run = dummy_run();
        let machine: WorkflowMachine<state::Succeeded> = WorkflowMachine::new_from_run(run.clone());
        assert_eq!(machine.run.status, RunStatus::Succeeded);
    }

    #[test]
    fn test_workflow_machine_into_run() {
        let run = dummy_run();
        let machine: WorkflowMachine<state::Succeeded> = WorkflowMachine::new_from_run(run.clone());
        let extracted = machine.into_run();
        assert_eq!(extracted.id, run.id);
        assert_eq!(extracted.status, RunStatus::Succeeded);
    }
}
