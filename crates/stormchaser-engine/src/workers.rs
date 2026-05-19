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

pub fn start_liveness_worker(pool: sqlx::PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
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
        }
    });
}

pub fn start_timeout_worker(
    pool: sqlx::PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) {
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
    });
}

pub fn start_resolver_crash_recovery_worker(pool: sqlx::PgPool, nats_client: async_nats::Client) {
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
                                    if result.is_ok() && tx.commit().await.is_ok() {
                                        // Emit failed event
                                        let event = stormchaser_model::events::WorkflowFailedEvent {
                                            run_id,
                                            event_type: stormchaser_model::events::EventType::Workflow(stormchaser_model::events::WorkflowEventType::Failed),
                                            timestamp: chrono::Utc::now(),
                                            status: stormchaser_model::workflow::RunStatus::Failed,
                                        };
                                        let js = async_nats::jetstream::new(nats_client.clone());
                                        let _ = stormchaser_model::nats::publish_cloudevent(
                                            &js,
                                            stormchaser_model::nats::NatsSubject::RunFailed(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
                                            stormchaser_model::events::EventType::Workflow(stormchaser_model::events::WorkflowEventType::Failed),
                                            stormchaser_model::events::EventSource::System,
                                            serde_json::to_value(event).unwrap(),
                                            Some(stormchaser_model::events::SchemaVersion::new("1.0".to_string())),
                                            None,
                                        ).await;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => tracing::error!("Failed to fetch stalled resolving runs: {:?}", e),
            }
        }
    });
}
