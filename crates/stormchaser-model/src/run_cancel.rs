//! The runner's side of run cancellation.
//!
//! When a run fails or is aborted the engine publishes a [`RunCancelEvent`] on
//! [`CANCEL_SUBJECT`]. Every runner reads it and stops whatever it holds for that run,
//! found by the run id label on its containers or Jobs, so nothing has to be remembered
//! in the runner and a restarted runner can still act on it.

use crate::events::RunCancelEvent;
use crate::runner::CANCEL_SUBJECT;
use anyhow::{Context, Result};
use async_nats::jetstream::consumer::{pull, AckPolicy, DeliverPolicy};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How far back a starting runner reads cancellations, to catch the ones sent while it
/// was down. Steps it re-adopts for a run cancelled in that time are stopped.
pub const REPLAY_WINDOW: Duration = Duration::from_secs(60 * 60);

/// How long a cancelled run is remembered, so a task for it that arrives late is refused.
const REMEMBER_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// When to look again for a cancelled run's steps after the first sweep. A step whose
/// image was still being pulled has no container or Job to stop the first time.
pub const RESWEEP_AFTER: [Duration; 2] = [Duration::from_secs(5), Duration::from_secs(30)];

static CANCELLED: OnceLock<Mutex<HashMap<Uuid, Instant>>> = OnceLock::new();

fn cancelled() -> std::sync::MutexGuard<'static, HashMap<Uuid, Instant>> {
    CANCELLED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Records that a run was cancelled.
pub fn mark_cancelled(run_id: Uuid) {
    let mut runs = cancelled();
    let now = Instant::now();
    runs.retain(|_, at| now.duration_since(*at) < REMEMBER_FOR);
    runs.insert(run_id, now);
}

/// Whether this runner has been told the run was cancelled.
pub fn is_cancelled(run_id: Uuid) -> bool {
    cancelled()
        .get(&run_id)
        .is_some_and(|at| at.elapsed() < REMEMBER_FOR)
}

/// The label selector that finds everything a runner started for a run.
pub fn run_label_selector(run_id: Uuid) -> String {
    format!("managed-by=stormchaser,stormchaser-run-id={run_id}")
}

/// Opens this runner's own reader on the cancel subject.
///
/// Each runner gets its own consumer, so all of them see every cancellation. It starts
/// [`REPLAY_WINDOW`] in the past and nothing is acknowledged: stopping a run twice is
/// harmless, and a runner that restarts simply reads the window again.
pub async fn subscribe(stream: &async_nats::jetstream::stream::Stream) -> Result<pull::Stream> {
    let start_time = time::OffsetDateTime::now_utc() - REPLAY_WINDOW;
    let consumer = stream
        .create_consumer(pull::Config {
            // Named so the create request carries the filter in its subject, which is
            // what NATS permissions match on. Unique, so a restart never collides with
            // the consumer its predecessor left behind.
            name: Some(format!("runner-cancel-{}", Uuid::new_v4().simple())),
            filter_subject: CANCEL_SUBJECT.to_string(),
            deliver_policy: DeliverPolicy::ByStartTime { start_time },
            ack_policy: AckPolicy::None,
            inactive_threshold: Duration::from_secs(300),
            ..Default::default()
        })
        .await
        .context("Failed to create the run cancellation consumer")?;
    consumer
        .messages()
        .await
        .context("Failed to read run cancellations")
}

/// Reads a cancellation, or `None` if the message is not one.
///
/// Only a message that arrived on the cancel subject counts. The subject is checked
/// here as well as in the consumer's filter because it is the whole of the message's
/// authority: NATS lets only the engine publish there.
pub fn parse(message: &async_nats::jetstream::Message) -> Option<RunCancelEvent> {
    if message.subject.as_str() != CANCEL_SUBJECT {
        return None;
    }
    let event: cloudevents::Event = serde_json::from_slice(&message.payload).ok()?;
    match cloudevents::AttributesReader::ty(&event) {
        "RunCancelEvent" => {}
        _ => return None,
    }
    match cloudevents::event::Event::data(&event) {
        Some(cloudevents::Data::Json(data)) => serde_json::from_value(data.clone()).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancelled_run_is_remembered_and_others_are_not() {
        let run = Uuid::new_v4();
        assert!(!is_cancelled(run));
        mark_cancelled(run);
        assert!(is_cancelled(run));
        assert!(!is_cancelled(Uuid::new_v4()));
    }

    #[test]
    fn the_selector_names_the_run_and_only_stormchaser_resources() {
        let run = Uuid::nil();
        assert_eq!(
            run_label_selector(run),
            "managed-by=stormchaser,stormchaser-run-id=00000000-0000-0000-0000-000000000000"
        );
    }
}
