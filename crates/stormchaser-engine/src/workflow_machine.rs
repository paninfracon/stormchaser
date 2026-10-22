//! A run's lifecycle, as a typestate machine.
//!
//! Every change to a run's status goes through a transition here. A machine is loaded
//! from the stored row with [`WorkflowMachine::load`], which refuses a row whose
//! status is not its state's, and saves with a compare-and-set on the version and
//! status it loaded (see [`crate::persistence::persist_run`]). The database refuses a
//! move the diagram in `docs/state_machines.md` does not allow.

use crate::persistence::persist_run;
use anyhow::Result;
use chrono::Utc;
use std::marker::PhantomData;
use stormchaser_model::workflow::{RunStatus, WorkflowRun};

/// State markers for the workflow typestate pattern
pub mod state {
    /// Saved by the API, waiting for the engine.
    pub struct Queued;
    /// The engine is fetching the workflow, checking policy and running queries.
    pub struct Resolving;
    /// Resolved; its first steps are about to be created.
    pub struct StartPending;
    /// Steps are being created and dispatched.
    pub struct Running;
    /// Every step finished in a status that lets the run go on.
    pub struct Succeeded;
    /// A step failed, or the run could not be resolved or started.
    pub struct Failed;
    /// The run timed out.
    pub struct Aborted;
}

/// The stored status a state stands for.
pub trait RunState {
    const STATUS: RunStatus;
}

macro_rules! run_state {
    ($($state:ident),+) => {
        $(impl RunState for state::$state {
            const STATUS: RunStatus = RunStatus::$state;
        })+
    };
}

run_state!(
    Queued,
    Resolving,
    StartPending,
    Running,
    Succeeded,
    Failed,
    Aborted
);

/// A run loaded in a state its stored status is not.
#[derive(Debug)]
pub struct WrongState {
    pub run: stormchaser_model::RunId,
    pub status: RunStatus,
    pub expected: RunStatus,
}

impl std::fmt::Display for WrongState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "run {} is {:?}, not {:?}",
            self.run, self.status, self.expected
        )
    }
}

impl std::error::Error for WrongState {}

/// A state machine for managing the lifecycle of a `WorkflowRun`.
pub struct WorkflowMachine<S> {
    run: WorkflowRun,
    _state: PhantomData<S>,
}

impl<S: RunState> WorkflowMachine<S> {
    /// The machine for a stored run, if its status is the state's.
    pub fn load(run: WorkflowRun) -> std::result::Result<Self, WrongState> {
        if run.status != S::STATUS {
            return Err(WrongState {
                run: run.id,
                status: run.status,
                expected: S::STATUS,
            });
        }
        Ok(Self {
            run,
            _state: PhantomData,
        })
    }

    /// The run as the machine holds it.
    pub fn run(&self) -> &WorkflowRun {
        &self.run
    }

    /// The run as the machine holds it.
    pub fn into_run(self) -> WorkflowRun {
        self.run
    }

    /// Saves the status a transition has set, if the row is unchanged since loading.
    async fn save<T: RunState>(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<T>> {
        debug_assert_eq!(self.run.status, T::STATUS);
        self.run.updated_at = Utc::now();
        persist_run(&mut self.run, &S::STATUS, executor).await?;
        Ok(WorkflowMachine {
            run: self.run,
            _state: PhantomData,
        })
    }

    fn finish(&mut self, status: RunStatus, error: Option<String>) {
        self.run.status = status;
        self.run.finished_at = Some(Utc::now());
        if error.is_some() {
            self.run.error = error;
        }
    }
}

/// `fail` and `abort`, which every unfinished state has.
macro_rules! ending_transitions {
    ($($state:ident),+) => {
        $(impl WorkflowMachine<state::$state> {
            #[tracing::instrument(skip(self, executor, error), fields(run_id = %self.run.id))]
            /// The run fails.
            pub async fn fail(
                mut self,
                error: String,
                executor: &mut sqlx::PgConnection,
            ) -> Result<WorkflowMachine<state::Failed>> {
                self.finish(RunStatus::Failed, Some(error));
                self.save(executor).await
            }

            #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
            /// The run timed out.
            pub async fn abort(
                mut self,
                executor: &mut sqlx::PgConnection,
            ) -> Result<WorkflowMachine<state::Aborted>> {
                self.finish(RunStatus::Aborted, None);
                self.save(executor).await
            }
        })+
    };
}

ending_transitions!(Queued, Resolving, StartPending);

impl WorkflowMachine<state::Queued> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// The engine starts resolving the run.
    pub async fn start_resolving(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Resolving>> {
        self.run.status = RunStatus::Resolving;
        self.run.started_resolving_at = Some(Utc::now());
        self.save(executor).await
    }
}

impl WorkflowMachine<state::Resolving> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// The run is resolved.
    pub async fn start_pending(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::StartPending>> {
        self.run.status = RunStatus::StartPending;
        self.save(executor).await
    }
}

impl WorkflowMachine<state::StartPending> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// The run's first steps are created.
    pub async fn start(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Running>> {
        self.run.status = RunStatus::Running;
        self.run.started_at = Some(Utc::now());
        self.save(executor).await
    }
}

impl WorkflowMachine<state::Running> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// Every step has finished in a status that lets the run go on.
    pub async fn succeed(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Succeeded>> {
        self.finish(RunStatus::Succeeded, None);
        self.save(executor).await
    }

    #[tracing::instrument(skip(self, executor, error), fields(run_id = %self.run.id))]
    /// The run fails, and runners are told to stop its other steps.
    pub async fn fail(
        mut self,
        error: String,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Failed>> {
        self.finish(RunStatus::Failed, Some(error));
        let failed = self.save(&mut *executor).await?;
        cancel_steps_on_runners(&failed.run, "the run failed", executor).await?;
        Ok(failed)
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.run.id))]
    /// The run timed out, and runners are told to stop its steps.
    pub async fn abort(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<WorkflowMachine<state::Aborted>> {
        self.finish(RunStatus::Aborted, None);
        let aborted = self.save(&mut *executor).await?;
        cancel_steps_on_runners(&aborted.run, "the run was aborted", executor).await?;
        Ok(aborted)
    }
}

/// Queues the message that tells runners to stop a run's steps.
///
/// A run that ends while it is running may have steps still going on runners: the other
/// branches of a run one step has failed, or everything in a run that timed out. Nothing
/// will read their results, so they are stopped rather than left to use a runner until
/// they finish. The message goes through the outbox on the connection that ends the run,
/// and so in the same transaction wherever the caller has one.
async fn cancel_steps_on_runners(
    run: &WorkflowRun,
    reason: &str,
    executor: &mut sqlx::PgConnection,
) -> Result<()> {
    use stormchaser_model::events::{
        EventSource, EventType, RunCancelEvent, RunnerEventType, SchemaVersion,
    };
    let event_type = EventType::Runner(RunnerEventType::Cancel);
    crate::db::outbox::insert_outbox_event(
        executor,
        stormchaser_model::nats::NatsSubject::RunnerCancel,
        event_type.clone(),
        EventSource::Engine,
        serde_json::to_value(RunCancelEvent {
            run_id: run.id,
            event_type,
            reason: reason.to_string(),
            timestamp: Utc::now(),
        })?,
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use stormchaser_model::RunId;

    fn run(status: RunStatus) -> WorkflowRun {
        WorkflowRun {
            id: RunId::new_v4(),
            workflow_name: "test".to_string(),
            initiating_user: "u".to_string(),
            status,
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
    fn a_run_loads_only_in_its_own_state() {
        assert!(WorkflowMachine::<state::Running>::load(run(RunStatus::Running)).is_ok());
        let wrong = WorkflowMachine::<state::Running>::load(run(RunStatus::Aborted))
            .err()
            .unwrap();
        assert_eq!(wrong.status, RunStatus::Aborted);
        assert_eq!(wrong.expected, RunStatus::Running);
    }
}
