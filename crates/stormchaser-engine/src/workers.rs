use crate::db;
use crate::handler;
use crate::parse_duration;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::runner::RunnerStatus;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;
use stormchaser_tls::TlsReloader;
use uuid::Uuid;

pub fn start_liveness_worker(
    pool: sqlx::PgPool,
    _nats_client: async_nats::Client,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            interval.tick().await;

            // 1. Mark stale runners offline
            let result =
                db::mark_stale_runners_offline(&pool, RunnerStatus::Offline, RunnerStatus::Online)
                    .await;

            match result {
                Ok(res) => {
                    let affected = res.rows_affected();
                    if affected > 0 {
                        tracing::info!(
                            "Marked {} runners as offline due to heartbeat timeout",
                            affected
                        );
                    }
                }
                Err(e) => tracing::error!("Failed to check runner liveness: {:?}", e),
            }

            // 2. Fail the steps of runners that have gone
            if let Err(e) = fail_zombie_steps(&pool).await {
                tracing::error!("Failed to fail the steps of lost runners: {:?}", e);
            }

            // 3. Fail the built-in steps of engines that have gone
            if let Err(e) =
                fail_lost_engine_steps(&pool, crate::engine_instance::LOST_AFTER_SECS).await
            {
                tracing::error!("Failed to fail the steps of lost engines: {:?}", e);
            }
        }
    })
}

/// Fails the steps of runners that have gone: offline, or silent for 30 seconds.
pub async fn fail_zombie_steps(pool: &sqlx::PgPool) -> anyhow::Result<usize> {
    let mut tx = pool.begin().await?;
    let zombies = db::steps::fetch_zombie_steps(&mut *tx).await?;
    for zombie in &zombies {
        tracing::warn!(
            "Detected zombie step {} (Run {}) on dead runner {}",
            zombie.id,
            zombie.run_id,
            zombie.runner_id.as_deref().unwrap_or_default()
        );
        let event = stormchaser_model::events::StepFailedEvent {
            runner_id: zombie.runner_id.clone(),
            ..stormchaser_model::events::StepFailedEvent::new(
                zombie.run_id,
                zombie.id,
                zombie.fencing_token,
                "lost_zombie",
            )
        };
        crate::db::outbox::insert_step_failed(&mut *tx, &event).await?;
    }
    tx.commit().await?;
    Ok(zombies.len())
}

/// How long an engine may go without a heartbeat before it is forgotten
const ENGINE_GONE_SECS: f64 = 86400.0;

/// Fails the built-in steps whose engine has sent no heartbeat for `lost_after_secs`.
/// They would never finish. Failing them, rather than running them again, is what
/// happens to a step whose runner has gone: a built-in step may have had its effect
/// before its engine died.
pub async fn fail_lost_engine_steps(
    pool: &sqlx::PgPool,
    lost_after_secs: f64,
) -> anyhow::Result<usize> {
    let mut tx = pool.begin().await?;
    let lost = db::take_lost_engine_steps(&mut tx, lost_after_secs).await?;
    for step in &lost {
        tracing::warn!(
            "Step {} of run {} was running in an engine that has stopped; failing it",
            step.id,
            step.run_id
        );
        let event = stormchaser_model::events::StepFailedEvent {
            completion_token: step.completion_token.clone(),
            ..stormchaser_model::events::StepFailedEvent::new(
                step.run_id,
                step.id,
                step.fencing_token,
                "The engine running this step stopped before it finished",
            )
        };
        crate::db::outbox::insert_step_failed(&mut *tx, &event).await?;
    }
    db::delete_gone_engines(&mut *tx, ENGINE_GONE_SECS).await?;
    tx.commit().await?;
    Ok(lost.len())
}

pub fn start_timeout_worker(
    pool: sqlx::PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;

            #[derive(sqlx::FromRow)]
            struct TimeoutCheck {
                id: Uuid,
                #[sqlx(rename = "status")]
                _status: RunStatus,
                created_at: chrono::DateTime<chrono::Utc>,
                started_at: Option<chrono::DateTime<chrono::Utc>>,
                timeout: String,
            }

            let result = db::get_active_workflow_runs_with_quotas(&pool)
                .await
                .map(|v: Vec<TimeoutCheck>| v);

            match result {
                Ok(runs) => {
                    for run in runs {
                        let duration_res = parse_duration(&run.timeout);
                        let duration = match duration_res {
                            Ok(d) => d,
                            Err(e) => {
                                tracing::error!(
                                    "Failed to parse timeout '{}' for run {}: {:?}",
                                    run.timeout,
                                    run.id,
                                    e
                                );
                                continue;
                            }
                        };

                        let start_time = run.started_at.unwrap_or(run.created_at);
                        let elapsed = chrono::Utc::now() - start_time;

                        if elapsed
                            > chrono::Duration::from_std(duration)
                                .unwrap_or_else(|_| chrono::Duration::zero())
                        {
                            if let Err(e) = handler::handle_workflow_timeout(
                                RunId::new(run.id),
                                pool.clone(),
                                nats_client.clone(),
                                tls_reloader.clone(),
                            )
                            .await
                            {
                                tracing::error!(
                                    "Failed to handle timeout for run {}: {:?}",
                                    run.id,
                                    e
                                );
                            }
                        }
                    }
                }
                Err(e) => tracing::error!("Failed to fetch runs for timeout check: {:?}", e),
            }
        }
    })
}

pub fn start_resolver_crash_recovery_worker(
    pool: sqlx::PgPool,
    _nats_client: async_nats::Client,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;

            match db::get_stalled_resolving_runs(&pool, 5).await {
                Ok(run_ids) => {
                    for run_id in run_ids {
                        if let Err(e) = fail_crashed_resolution(&pool, run_id).await {
                            tracing::error!(
                                "Failed to recover the crashed resolution of run {}: {:?}",
                                run_id,
                                e
                            );
                        }
                    }
                }
                Err(e) => tracing::error!("Failed to fetch stalled resolving runs: {:?}", e),
            }
        }
    })
}

/// Fails a run whose resolution has stalled, its engine having crashed while resolving
/// it, together with the event that reports it.
async fn fail_crashed_resolution(pool: &sqlx::PgPool, run_id: RunId) -> anyhow::Result<()> {
    use stormchaser_model::events::{
        EventSource, EventType, SchemaVersion, WorkflowEventType, WorkflowFailedEvent,
    };
    let run = handler::fetch_run(run_id, pool).await?;
    if run.status != RunStatus::Resolving {
        return Ok(());
    }
    tracing::warn!("Recovering crashed resolution for run {}", run_id);
    let machine = crate::workflow_machine::WorkflowMachine::<
        crate::workflow_machine::state::Resolving,
    >::new_from_run(run);
    let mut tx = pool.begin().await?;
    machine
        .fail("Engine crashed during resolution".to_string(), &mut tx)
        .await?;
    let event = WorkflowFailedEvent {
        run_id,
        event_type: EventType::Workflow(WorkflowEventType::Failed),
        timestamp: chrono::Utc::now(),
        status: RunStatus::Failed,
    };
    crate::db::outbox::insert_outbox_event(
        &mut *tx,
        stormchaser_model::nats::NatsSubject::RunFailed(Some(
            stormchaser_model::nats::compute_shard_id(&run_id),
        )),
        EventType::Workflow(WorkflowEventType::Failed),
        EventSource::System,
        serde_json::to_value(event)?,
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub fn start_outbox_relay_worker(
    pool: sqlx::PgPool,
    nats_client: async_nats::Client,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        let js = async_nats::jetstream::new(nats_client.clone());

        loop {
            interval.tick().await;

            if let Ok(mut tx) = pool.begin().await {
                match db::outbox::fetch_outbox_events_for_processing(&mut *tx, 100).await {
                    Ok(events) => {
                        for event in events {
                            let mut headers = async_nats::HeaderMap::new();
                            if let serde_json::Value::Object(map) = event.headers {
                                for (k, v) in map {
                                    if let serde_json::Value::String(s) = v {
                                        headers.insert(k.as_str(), s.as_str());
                                    }
                                }
                            }

                            // The relay publishes an event again when it dies before
                            // deleting its row; the stream drops the second copy
                            headers.insert("Nats-Msg-Id", event.id.to_string().as_str());

                            // Published once the stream has stored it, not once sent
                            let published = match js
                                .publish_with_headers(event.subject, headers, event.payload.into())
                                .await
                            {
                                Ok(ack) => ack.await.map(|_| ()).map_err(anyhow::Error::from),
                                Err(e) => Err(anyhow::Error::from(e)),
                            };
                            match published {
                                Ok(_) => {
                                    if let Err(error) =
                                        db::outbox::delete_outbox_event(&mut *tx, event.id).await
                                    {
                                        tracing::error!(
                                            "Failed to delete outbox event {} after publish: {:?}",
                                            event.id,
                                            error
                                        );
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Failed to publish outbox event {}: {:?}",
                                        event.id,
                                        e
                                    );
                                    if let Err(error) =
                                        db::outbox::reschedule_outbox_event(&mut *tx, event.id)
                                            .await
                                    {
                                        tracing::error!(
                                            "Failed to reschedule outbox event {} after publish failure: {:?}",
                                            event.id,
                                            error
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => tracing::error!("Failed to fetch outbox events: {:?}", e),
                }
                if let Err(error) = tx.commit().await {
                    tracing::error!("Failed to commit outbox relay transaction: {:?}", error);
                }
            }
        }
    })
}

/// True when a stream message is the task message dispatched for `step_id`.
///
/// Checked before erasing, so that a stale record can never remove some other message
/// that later came to occupy the same sequence (a stream deleted and created again).
fn is_task_message_for(subject: &str, payload: &[u8], step_id: Uuid) -> bool {
    if !subject.contains(".step.scheduled.") && !subject.starts_with("stormchaser.v1.runner.") {
        return false;
    }
    serde_json::from_slice::<serde_json::Value>(payload)
        .ok()
        .and_then(|event| {
            event
                .get("data")?
                .get("step_id")?
                .as_str()
                .map(str::to_owned)
        })
        .is_some_and(|id| id == step_id.to_string())
}

/// Erases from the stream the task messages of steps that are finished or archived.
///
/// A task message carries the step's resolved spec, with secrets, registry credentials
/// and storage URLs. Nothing needs it once the step is over, so it is removed here rather
/// than left for the stream's age limit. Returns how many records were cleared.
pub async fn purge_finished_task_messages(
    pool: &sqlx::PgPool,
    js: &async_nats::jetstream::Context,
) -> anyhow::Result<usize> {
    use async_nats::jetstream::stream::RawMessageErrorKind;

    let stream = js.get_stream(stormchaser_model::nats::STREAM_NAME).await?;
    let mut tx = pool.begin().await?;
    let finished = db::task_messages::fetch_finished_task_messages(&mut *tx, 200).await?;
    let mut cleared = 0;

    for (seq, step_id) in finished {
        let erase = match stream.get_raw_message(seq as u64).await {
            Ok(message) => is_task_message_for(&message.subject, &message.payload, step_id),
            // Already aged out or erased
            Err(e) if e.kind() == RawMessageErrorKind::NoMessageFound => false,
            Err(e) => {
                tracing::warn!("Could not read task message {} to erase it: {:?}", seq, e);
                break;
            }
        };
        if erase {
            // The server overwrites the message's bytes on disk, not just its index entry
            if let Err(e) = stream.delete_message(seq as u64).await {
                tracing::warn!(
                    "Could not erase task message {} of step {}: {:?}",
                    seq,
                    step_id,
                    e
                );
                break;
            }
        }
        db::task_messages::delete_task_message_record(&mut *tx, seq).await?;
        cleared += 1;
    }

    tx.commit().await?;
    Ok(cleared)
}

pub fn start_task_message_purge_worker(
    pool: sqlx::PgPool,
    nats_client: async_nats::Client,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        let js = async_nats::jetstream::new(nats_client);
        loop {
            interval.tick().await;
            if let Err(e) = purge_finished_task_messages(&pool, &js).await {
                tracing::warn!("Task message purge failed: {:?}", e);
            }
        }
    })
}

/// How long a claimed step may take to be handed over before it is taken for lost
const LOST_DISPATCH_SECS: f64 = 300.0;
/// How long a step may wait for dispatch before the engine tries again
const WAITING_STEP_SECS: f64 = 60.0;

/// Puts back steps whose dispatch was lost, and dispatches again the runs whose
/// steps have been waiting longer than they should.
pub async fn recover_dispatches(
    pool: &sqlx::PgPool,
    nats_client: &async_nats::Client,
    tls_reloader: &Arc<TlsReloader>,
    lost_after_secs: f64,
    waiting_after_secs: f64,
) -> anyhow::Result<()> {
    put_back_lost_dispatches(pool, lost_after_secs).await?;

    for run_id in db::runs_with_waiting_steps(pool, waiting_after_secs).await? {
        if let Err(e) = handler::dispatch_pending_steps(
            run_id,
            pool.clone(),
            nats_client.clone(),
            tls_reloader.clone(),
        )
        .await
        {
            tracing::warn!(
                "Could not dispatch the waiting steps of run {}: {:?}",
                run_id,
                e
            );
        }
    }
    Ok(())
}

/// Puts back to pending the steps whose dispatch was lost, giving back what they
/// claimed of their run's quota, as a failed dispatch does. All or none, so that no
/// step is put back without its quota.
async fn put_back_lost_dispatches(pool: &sqlx::PgPool, lost_after_secs: f64) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    let lost = db::reset_lost_dispatches(&mut *tx, lost_after_secs).await?;
    let mut limits = std::collections::HashMap::new();
    for step in lost {
        tracing::warn!(
            "Step {} of run {} was claimed for dispatch and never handed over; dispatching it again",
            step.id,
            step.run_id
        );
        let (max_cpu, max_mem) = match limits.get(&step.run_id) {
            Some(limit) => *limit,
            None => {
                let quotas = handler::fetch_quotas(step.run_id, &mut *tx).await?;
                let limit = (
                    crate::resource_utils::parse_cpu(&quotas.max_cpu).unwrap_or(0.0),
                    crate::resource_utils::parse_memory(&quotas.max_memory).unwrap_or(0),
                );
                *limits.entry(step.run_id).or_insert(limit)
            }
        };
        if max_cpu > 0.0 || max_mem > 0 {
            let req =
                crate::resource_utils::get_step_resource_requirements(&step.step_type, &step.spec);
            db::release_step_quota(&mut tx, step.run_id, req.cpu, req.memory).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

pub fn start_dispatch_recovery_worker(
    pool: sqlx::PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(e) = recover_dispatches(
                &pool,
                &nats_client,
                &tls_reloader,
                LOST_DISPATCH_SECS,
                WAITING_STEP_SECS,
            )
            .await
            {
                tracing::warn!("Dispatch recovery failed: {:?}", e);
            }
        }
    })
}

#[cfg(test)]
mod task_message_tests {
    use super::is_task_message_for;
    use uuid::Uuid;

    #[test]
    fn only_the_steps_own_task_message_matches() {
        let step_id = Uuid::new_v4();
        let payload = serde_json::json!({"type": "x", "data": {"step_id": step_id}}).to_string();
        let task = "stormchaser.v1.3.step.scheduled.runcontainer";

        assert!(is_task_message_for(task, payload.as_bytes(), step_id));
        assert!(is_task_message_for(
            "stormchaser.v1.runner.docker.abc",
            payload.as_bytes(),
            step_id
        ));
        // Another step's message, another kind of event, or junk
        assert!(!is_task_message_for(
            task,
            payload.as_bytes(),
            Uuid::new_v4()
        ));
        assert!(!is_task_message_for(
            "stormchaser.v1.3.run.queued",
            payload.as_bytes(),
            step_id
        ));
        assert!(!is_task_message_for(task, b"not json", step_id));
    }
}
