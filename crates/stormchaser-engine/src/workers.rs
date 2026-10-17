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

            // 2. Detect zombie steps and fail them out
            if let Ok(mut tx) = pool.begin().await {
                match crate::db::steps::fetch_zombie_steps(&mut *tx).await {
                    Ok(zombies) => {
                        for zombie in zombies {
                            tracing::warn!(
                                "Detected zombie step {} (Run {}) on dead runner {}",
                                zombie.id,
                                zombie.run_id,
                                zombie.runner_id.clone().unwrap_or_default()
                            );

                            let fail_event = stormchaser_model::events::StepFailedEvent {
                                run_id: zombie.run_id,
                                step_id: zombie.id,
                                fencing_token: zombie.fencing_token,
                                completion_token: None,
                                event_type: stormchaser_model::events::EventType::Step(
                                    stormchaser_model::events::StepEventType::Failed,
                                ),
                                error: "lost_zombie".to_string(),
                                runner_id: zombie.runner_id,
                                exit_code: None,
                                storage_hashes: None,
                                artifacts: None,
                                test_reports: None,
                                outputs: None,
                                timestamp: chrono::Utc::now(),
                            };

                            let event_payload = match serde_json::to_value(&fail_event) {
                                Ok(value) => value,
                                Err(error) => {
                                    tracing::error!(
                                    "Failed to serialize zombie failure event for run {} step {}: {:?}",
                                    fail_event.run_id,
                                    fail_event.step_id,
                                    error
                                );
                                    continue;
                                }
                            };

                            if let Err(error) = crate::db::outbox::insert_outbox_event(
                                &mut *tx,
                                stormchaser_model::nats::NatsSubject::StepFailed(Some(
                                    stormchaser_model::nats::compute_shard_id(&zombie.run_id),
                                )),
                                stormchaser_model::events::EventType::Step(
                                    stormchaser_model::events::StepEventType::Failed,
                                ),
                                stormchaser_model::events::EventSource::System,
                                event_payload,
                                Some(stormchaser_model::events::SchemaVersion::new(
                                    "1.0".to_string(),
                                )),
                                None,
                            )
                            .await
                            {
                                tracing::error!(
                                "Failed to enqueue zombie failure event for run {} step {}: {:?}",
                                zombie.run_id,
                                zombie.id,
                                error
                            );
                            }
                        }
                    }
                    Err(e) => tracing::error!("Failed to fetch zombie steps: {:?}", e),
                }
                if let Err(error) = tx.commit().await {
                    tracing::error!("Failed to commit liveness worker transaction: {:?}", error);
                }
            }
        }
    })
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
                        tracing::warn!("Recovering crashed resolution for run {}", run_id);
                        if let Ok(run) = handler::fetch_run(run_id, &pool).await {
                            if run.status == RunStatus::Resolving {
                                let machine = crate::workflow_machine::WorkflowMachine::<
                                    crate::workflow_machine::state::Resolving,
                                >::new_from_run(run);
                                if let Ok(mut tx) = pool.begin().await {
                                    let result = machine
                                        .fail(
                                            "Engine crashed during resolution".to_string(),
                                            &mut tx,
                                        )
                                        .await;
                                    if result.is_ok() {
                                        // Emit failed event
                                        let event = stormchaser_model::events::WorkflowFailedEvent {
                                            run_id,
                                            event_type: stormchaser_model::events::EventType::Workflow(stormchaser_model::events::WorkflowEventType::Failed),
                                            timestamp: chrono::Utc::now(),
                                            status: stormchaser_model::workflow::RunStatus::Failed,
                                        };
                                        let _ = crate::db::outbox::insert_outbox_event(
                                            &mut *tx,
                                            stormchaser_model::nats::NatsSubject::RunFailed(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
                                            stormchaser_model::events::EventType::Workflow(stormchaser_model::events::WorkflowEventType::Failed),
                                            stormchaser_model::events::EventSource::System,
                                            serde_json::to_value(event).unwrap(),
                                            Some(stormchaser_model::events::SchemaVersion::new("1.0".to_string())),
                                            None,
                                        ).await;
                                        if let Err(error) = tx.commit().await {
                                            tracing::error!(
                                                "Failed to commit resolver crash recovery transaction for run {}: {:?}",
                                                run_id,
                                                error
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => tracing::error!("Failed to fetch stalled resolving runs: {:?}", e),
            }
        }
    })
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

                            match js
                                .publish_with_headers(event.subject, headers, event.payload.into())
                                .await
                            {
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
    for lost in db::reset_lost_dispatches(pool, lost_after_secs).await? {
        tracing::warn!(
            "Step {} of run {} was claimed for dispatch and never handed over; dispatching it again",
            lost.id,
            lost.run_id
        );
        // What it claimed of its run's quota, as a failed dispatch gives back
        let quotas = handler::fetch_quotas(lost.run_id, pool).await?;
        let max_cpu = crate::resource_utils::parse_cpu(&quotas.max_cpu).unwrap_or(0.0);
        let max_mem = crate::resource_utils::parse_memory(&quotas.max_memory).unwrap_or(0);
        if max_cpu > 0.0 || max_mem > 0 {
            let req =
                crate::resource_utils::get_step_resource_requirements(&lost.step_type, &lost.spec);
            let mut conn = pool.acquire().await?;
            db::release_step_quota(&mut conn, lost.run_id, req.cpu, req.memory).await?;
        }
    }

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
