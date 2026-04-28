use anyhow::Result;
use chrono::Utc;
use std::marker::PhantomData;
use stormchaser_model::workflow::{RunStatus, WorkflowRun};

/// State markers for the workflow typestate pattern
#[allow(dead_code)]
pub mod state {
    pub struct Queued;
    pub struct Resolving;
    pub struct StartPending;
    pub struct Running;
    pub struct Succeeded;
    pub struct Failed;
    pub struct Aborted;
}

#[allow(dead_code)]
pub struct WorkflowMachine<S> {
    pub run: WorkflowRun,
    _state: PhantomData<S>,
}

#[allow(dead_code)]
impl<S> WorkflowMachine<S> {
    pub fn new_from_run(run: WorkflowRun) -> Self {
        Self {
            run,
            _state: PhantomData,
        }
    }

    pub fn into_run(self) -> WorkflowRun {
        self.run
    }
}

#[allow(dead_code)]
impl WorkflowMachine<state::Queued> {
    pub fn new(run: WorkflowRun) -> Self {
        let mut run = run;
        run.status = RunStatus::Queued;

        Self {
            run,
            _state: PhantomData,
        }
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
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
