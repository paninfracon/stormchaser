//! A step's lifecycle, as a typestate machine.
//!
//! Every change to a step's status goes through a transition here. A machine is
//! loaded from the stored row with [`StepMachine::load`], which refuses a row whose
//! status is not one its state stands for. A transition saves with a compare-and-set
//! on the status the machine was loaded with, so a step another engine or handler has
//! moved meanwhile is not moved again: the save fails instead (see
//! [`crate::persistence::persist_step_instance`]). Set-based transitions, such as
//! claiming a run's pending steps, are in [`bulk`]. The database refuses a move the
//! diagram in `docs/state_machines.md` does not allow, whichever code makes it.

use crate::persistence::persist_step_instance;
use anyhow::Result;
use std::marker::PhantomData;
use stormchaser_model::step::{StepInstance, StepStatus};

pub mod bulk;
pub mod transitions;

/// State markers for the typestate pattern
pub mod state {
    /// Waiting to be dispatched.
    pub struct Pending;
    /// Claimed for dispatch; with a runner, or being run by the engine.
    pub struct Initializing;
    /// The runner is restoring the step's parked storage.
    pub struct UnpackingSfs;
    /// The runner is executing the step.
    pub struct Running;
    /// The runner is parking the step's storage.
    pub struct PackingSfs;
    /// Waiting for a person or an event, or held back by a concurrency limit.
    pub struct WaitingForEvent;
    /// Any status a step has between being handed out and finishing. A step's result
    /// arrives on its own subject and can overtake the reports of its progress, so it
    /// may find the step in any of them.
    pub struct Unfinished;
    /// Finished successfully.
    pub struct Succeeded;
    /// Finished with an error.
    pub struct Failed;
    /// Failed, with `allow_failure`.
    pub struct FailedIgnored;
    /// Its runner was lost while it was in flight.
    pub struct LostZombie;
}

/// The stored statuses a state stands for.
pub trait StepState {
    const NAME: &'static str;
    const STATUSES: &'static [StepStatus];
}

macro_rules! step_state {
    ($state:ident, $($status:ident),+) => {
        impl StepState for state::$state {
            const NAME: &'static str = stringify!($state);
            const STATUSES: &'static [StepStatus] = &[$(StepStatus::$status),+];
        }
    };
}

step_state!(Pending, Pending);
step_state!(Initializing, Initializing);
step_state!(UnpackingSfs, UnpackingSfs);
step_state!(Running, Running);
step_state!(PackingSfs, PackingSfs);
step_state!(WaitingForEvent, WaitingForEvent);
step_state!(
    Unfinished,
    Initializing,
    UnpackingSfs,
    Running,
    PackingSfs,
    WaitingForEvent
);
step_state!(Succeeded, Succeeded);
step_state!(Failed, Failed);
step_state!(FailedIgnored, FailedIgnored);
step_state!(LostZombie, LostZombie);

/// A step loaded in a state its stored status does not belong to.
#[derive(Debug)]
pub struct WrongState {
    pub step: stormchaser_model::StepInstanceId,
    pub status: StepStatus,
    pub expected: &'static str,
}

impl std::fmt::Display for WrongState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "step {} is {:?}, not {}",
            self.step, self.status, self.expected
        )
    }
}

impl std::error::Error for WrongState {}

/// A state machine for managing the lifecycle of a `StepInstance`.
pub struct StepMachine<S> {
    instance: StepInstance,
    /// The status the row had when the machine was loaded or last saved
    stored: StepStatus,
    _state: PhantomData<S>,
}

impl<S: StepState> StepMachine<S> {
    /// The machine for a stored step, if its status is one of the state's.
    pub fn load(instance: StepInstance) -> std::result::Result<Self, WrongState> {
        if !S::STATUSES.contains(&instance.status) {
            return Err(WrongState {
                step: instance.id,
                status: instance.status,
                expected: S::NAME,
            });
        }
        Ok(Self {
            stored: instance.status.clone(),
            instance,
            _state: PhantomData,
        })
    }

    /// The step as the machine holds it.
    pub fn instance(&self) -> &StepInstance {
        &self.instance
    }

    /// The step as the machine holds it.
    pub fn into_instance(self) -> StepInstance {
        self.instance
    }

    /// Saves the step with the status a transition has set, if the stored status is
    /// still the one this machine was loaded with.
    async fn save<T: StepState>(self, executor: &mut sqlx::PgConnection) -> Result<StepMachine<T>> {
        debug_assert!(T::STATUSES.contains(&self.instance.status));
        persist_step_instance(&self.instance, &self.stored, executor).await?;
        Ok(StepMachine {
            stored: self.instance.status.clone(),
            instance: self.instance,
            _state: PhantomData,
        })
    }

    /// Ends the step with `status`.
    fn finish(&mut self, status: StepStatus, error: Option<String>, exit_code: Option<i32>) {
        self.instance.status = status;
        self.instance.finished_at = Some(chrono::Utc::now());
        self.instance.error = error;
        self.instance.exit_code = exit_code;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use stormchaser_model::{RunId, StepInstanceId};
    use uuid::Uuid;

    fn instance(status: StepStatus) -> StepInstance {
        StepInstance {
            id: StepInstanceId::new(Uuid::new_v4()),
            run_id: RunId::new(Uuid::new_v4()),
            step_name: "test".to_string(),
            step_type: "RunContainer".to_string(),
            status,
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
    fn a_step_loads_only_in_a_state_its_status_belongs_to() {
        assert!(StepMachine::<state::Running>::load(instance(StepStatus::Running)).is_ok());
        let wrong = StepMachine::<state::Running>::load(instance(StepStatus::Pending))
            .err()
            .unwrap();
        assert_eq!(wrong.status, StepStatus::Pending);
        assert_eq!(wrong.expected, "Running");
    }

    #[test]
    fn unfinished_covers_every_status_between_handing_out_and_finishing() {
        for status in [
            StepStatus::Initializing,
            StepStatus::UnpackingSfs,
            StepStatus::Running,
            StepStatus::PackingSfs,
            StepStatus::WaitingForEvent,
        ] {
            assert!(StepMachine::<state::Unfinished>::load(instance(status)).is_ok());
        }
        for status in [
            StepStatus::Pending,
            StepStatus::Succeeded,
            StepStatus::Failed,
            StepStatus::FailedIgnored,
            StepStatus::Skipped,
            StepStatus::Aborted,
            StepStatus::LostZombie,
        ] {
            assert!(StepMachine::<state::Unfinished>::load(instance(status)).is_err());
        }
    }
}
