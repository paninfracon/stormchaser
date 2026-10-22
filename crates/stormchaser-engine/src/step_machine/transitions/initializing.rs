use crate::step_machine::{state, StepMachine};
use anyhow::{Context, Result};
use chrono::Utc;
use std::marker::PhantomData;
use stormchaser_model::step::StepStatus;

/// What [`StepMachine::<state::Initializing>::requeue`] did.
pub enum Requeue {
    /// The step is pending again, to be dispatched later.
    Requeued(StepMachine<state::Pending>),
    /// Its task may already have been published, so putting it back could run it
    /// twice. It is left as it was.
    HandedOver(StepMachine<state::Initializing>),
}

impl StepMachine<state::Initializing> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// The step has started: a runner reports it, or the engine starts a built-in step
    /// (`runner_id` then names the built-in).
    pub async fn start(
        mut self,
        runner_id: Option<String>,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Running>> {
        self.instance.status = StepStatus::Running;
        self.instance.started_at = Some(Utc::now());
        if runner_id.is_some() {
            self.instance.runner_id = runner_id;
        }
        self.save(executor).await
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// The runner is restoring the step's storage.
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

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Puts a claimed step back to pending, if its task was never handed over: the run
    /// has no room for it yet, or dispatching it failed before its task was published.
    pub async fn requeue(mut self, executor: &mut sqlx::PgConnection) -> Result<Requeue> {
        let requeued = sqlx::query(
            r#"
            UPDATE step_instances
            SET status = 'pending', claimed_at = NULL, engine_id = NULL
            WHERE id = $1 AND status = 'initializing' AND dispatched_at IS NULL
            "#,
        )
        .bind(self.instance.id)
        .execute(&mut *executor)
        .await
        .with_context(|| format!("Failed to put step {} back to pending", self.instance.id))?
        .rows_affected()
            > 0;
        if !requeued {
            return Ok(Requeue::HandedOver(self));
        }
        self.instance.status = StepStatus::Pending;
        Ok(Requeue::Requeued(StepMachine {
            stored: StepStatus::Pending,
            instance: self.instance,
            _state: PhantomData,
        }))
    }
}
