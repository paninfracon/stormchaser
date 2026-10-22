use crate::step_machine::{state, StepMachine};
use anyhow::{Context, Result};
use std::marker::PhantomData;
use stormchaser_model::step::StepStatus;

/// Every in-flight state is one of `Unfinished`'s, so a machine in one can always be
/// seen as unfinished.
macro_rules! widen_to_unfinished {
    ($($state:ident),+) => {
        $(impl StepMachine<state::$state> {
            /// The same step, as an unfinished one.
            pub fn unfinished(self) -> StepMachine<state::Unfinished> {
                StepMachine {
                    instance: self.instance,
                    stored: self.stored,
                    _state: PhantomData,
                }
            }
        })+
    };
}

widen_to_unfinished!(
    Initializing,
    UnpackingSfs,
    Running,
    PackingSfs,
    WaitingForEvent
);

impl StepMachine<state::Unfinished> {
    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// The step has succeeded.
    pub async fn succeed(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Succeeded>> {
        self.finish(StepStatus::Succeeded, None, Some(0));
        self.save(executor).await
    }

    #[tracing::instrument(skip(self, executor, error), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// The step has failed: it reported failure, it was rejected, or its run timed out.
    pub async fn fail(
        mut self,
        error: String,
        exit_code: Option<i32>,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Failed>> {
        self.finish(StepStatus::Failed, Some(error), exit_code);
        self.save(executor).await
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// The runner holding the step was lost. Only a step that was handed out has one.
    pub async fn zombify(
        mut self,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::LostZombie>> {
        if self.instance.status == StepStatus::WaitingForEvent {
            anyhow::bail!(
                "step {} is waiting for an event and has no runner to lose",
                self.instance.id
            );
        }
        self.finish(
            StepStatus::LostZombie,
            Some("lost_zombie".to_string()),
            None,
        );
        self.save(executor).await
    }

    #[tracing::instrument(skip(self, executor), fields(run_id = %self.instance.run_id, step_id = %self.instance.id))]
    /// Puts a failed attempt back to pending for its next attempt, to be dispatched no
    /// sooner than `delay` from now. What the attempt left (runner, times, error) is
    /// cleared, and the step gets a new completion token, as a step that never ran has:
    /// a built-in step needs one, and a late report from the last attempt does not
    /// carry it.
    pub async fn retry(
        mut self,
        delay: std::time::Duration,
        executor: &mut sqlx::PgConnection,
    ) -> Result<StepMachine<state::Pending>> {
        let saved = sqlx::query(
            r#"
            UPDATE step_instances
            SET status = 'pending', attempt = attempt + 1,
                retry_at = NOW() + make_interval(secs => $2),
                runner_id = NULL, engine_id = NULL, started_at = NULL, finished_at = NULL,
                exit_code = NULL, error = NULL, completion_token = gen_random_uuid()::text,
                claimed_at = NULL, dispatched_at = NULL
            WHERE id = $1 AND status = $3
            "#,
        )
        .bind(self.instance.id)
        .bind(delay.as_secs_f64())
        .bind(&self.stored)
        .execute(&mut *executor)
        .await
        .with_context(|| format!("Failed to put step {} back for a retry", self.instance.id))?
        .rows_affected();
        if saved == 0 {
            anyhow::bail!(
                "step {} is no longer {:?}; not retrying it",
                self.instance.id,
                self.stored
            );
        }
        self.instance.status = StepStatus::Pending;
        self.instance.runner_id = None;
        self.instance.started_at = None;
        self.instance.finished_at = None;
        self.instance.exit_code = None;
        self.instance.error = None;
        Ok(StepMachine {
            stored: StepStatus::Pending,
            instance: self.instance,
            _state: PhantomData,
        })
    }
}
