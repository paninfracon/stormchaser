//! A step that is with a runner can only be reported on, taken over, or given registry
//! credentials for, by that runner.

use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use std::env::var;
use std::sync::Arc;
use stormchaser_engine::handler;
use stormchaser_engine::handler::step::events::artifact_auth::artifact_push_auth;
use stormchaser_model::events::{
    EventType, StepCompletedEvent, StepEventType, StepFailedEvent, StepRunningEvent,
};
use stormchaser_model::step::StepStatus;
use stormchaser_model::{RunId, StepInstanceId};
use stormchaser_tls::TlsReloader;

async fn pool() -> PgPool {
    PgPool::connect(&var("DATABASE_URL").expect("DATABASE_URL must be set"))
        .await
        .unwrap()
}

async fn tls() -> Arc<TlsReloader> {
    Arc::new(
        TlsReloader::new(stormchaser_tls::TlsConfig::default())
            .await
            .unwrap(),
    )
}

async fn nats() -> async_nats::Client {
    let url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    stormchaser_model::nats::connect(url).await.unwrap()
}

/// Inserts a runner whose last heartbeat was `seconds_ago`.
async fn insert_runner(pool: &PgPool, id: &str, seconds_ago: f64) {
    sqlx::query(
        "INSERT INTO runners (id, runner_type, protocol_version, nats_subject, last_heartbeat_at) VALUES ($1, 'docker', 'v1', $2, NOW() - make_interval(secs => $3))",
    )
    .bind(id)
    .bind(format!("stormchaser.v1.runner.docker.{id}"))
    .bind(seconds_ago)
    .execute(pool)
    .await
    .unwrap();
}

/// A running step that has been handed to `runner` (so it has no completion token).
async fn insert_step_on(pool: &PgPool, runner: &str) -> (RunId, StepInstanceId) {
    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();
    sqlx::query(
        "INSERT INTO workflow_runs (id, workflow_name, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'runner-identity-test', 'running', 1, 'http://git.local', 'workflow.storm', 'main')"
    )
    .bind(run_id)
    .execute(pool)
    .await
    .unwrap();
    let workflow = stormchaser_dsl::StormchaserParser::new()
        .parse(
            r#"
        stormchaser_dsl_version = "v1"
        workflow "runner-identity-test" {
            steps {
                step "build" "RunContainer" {
                    image = "alpine"
                }
            }
        }
    "#,
        )
        .unwrap();
    stormchaser_engine::db::runs::insert_run_context(
        pool,
        run_id,
        "v1",
        serde_json::to_value(&workflow).unwrap(),
        Some("code"),
        json!({}),
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, status, step_type, runner_id, started_at, spec, completion_token) VALUES ($1, $2, 'build', 'running', 'RunContainer', $3, NOW(), '{}', NULL)")
        .bind(step_id)
        .bind(run_id)
        .bind(runner)
        .execute(pool)
        .await
        .unwrap();
    (run_id, step_id)
}

async fn status_and_runner(
    pool: &PgPool,
    step_id: StepInstanceId,
) -> Option<(StepStatus, Option<String>)> {
    sqlx::query_as("SELECT status, runner_id FROM step_instances WHERE id = $1")
        .bind(step_id)
        .fetch_optional(pool)
        .await
        .unwrap()
}

fn completed(run_id: RunId, step_id: StepInstanceId, runner: Option<&str>) -> StepCompletedEvent {
    StepCompletedEvent {
        run_id,
        step_id,
        fencing_token: 1,
        completion_token: None,
        event_type: EventType::Step(StepEventType::Completed),
        runner_id: runner.map(str::to_owned),
        exit_code: Some(0),
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
        outputs: None,
        timestamp: Utc::now(),
    }
}

fn failed(run_id: RunId, step_id: StepInstanceId, runner: Option<&str>) -> StepFailedEvent {
    StepFailedEvent {
        run_id,
        step_id,
        fencing_token: 1,
        completion_token: None,
        event_type: EventType::Step(StepEventType::Failed),
        error: "forged".to_string(),
        runner_id: runner.map(str::to_owned),
        exit_code: Some(1),
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
        outputs: None,
        timestamp: Utc::now(),
    }
}

fn running(run_id: RunId, step_id: StepInstanceId, runner: &str) -> StepRunningEvent {
    StepRunningEvent {
        run_id,
        step_id,
        event_type: EventType::Step(StepEventType::Running),
        runner_id: Some(runner.to_string()),
        timestamp: Utc::now(),
    }
}

async fn clean_up(pool: &PgPool, run_id: RunId, runners: &[&str]) {
    for delete in [
        "DELETE FROM workflow_runs WHERE id = $1",
        "DELETE FROM archived_workflow_runs WHERE id = $1",
    ] {
        let _ = sqlx::query(delete).bind(run_id).execute(pool).await;
    }
    sqlx::query("DELETE FROM runners WHERE id = ANY($1)")
        .bind(runners.iter().map(|r| r.to_string()).collect::<Vec<_>>())
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn only_the_runner_a_step_is_with_can_report_its_result() {
    let pool = pool().await;
    let (nats, tls) = (nats().await, tls().await);
    let holder = format!("holder-{}", uuid::Uuid::new_v4());
    let other = format!("other-{}", uuid::Uuid::new_v4());
    insert_runner(&pool, &holder, 0.0).await;
    insert_runner(&pool, &other, 0.0).await;
    let (run_id, step_id) = insert_step_on(&pool, &holder).await;

    // Another runner, or something that names no runner, can neither fail nor complete it
    for reporter in [Some(other.as_str()), None] {
        handler::step::events::handle_step_failed(
            failed(run_id, step_id, reporter),
            pool.clone(),
            nats.clone(),
            tls.clone(),
        )
        .await
        .unwrap();
        handler::step::events::handle_step_completed(
            completed(run_id, step_id, reporter),
            pool.clone(),
            nats.clone(),
            Arc::new(None),
            tls.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            status_and_runner(&pool, step_id).await,
            Some((StepStatus::Running, Some(holder.clone()))),
            "a result from {reporter:?} was accepted"
        );
    }

    // Its own runner can
    handler::step::events::handle_step_completed(
        completed(run_id, step_id, Some(&holder)),
        pool.clone(),
        nats,
        Arc::new(None),
        tls,
    )
    .await
    .unwrap();
    let live = status_and_runner(&pool, step_id).await;
    assert!(
        live.is_none()
            || live
                .as_ref()
                .is_some_and(|(status, _)| *status == StepStatus::Succeeded),
        "the holder's own result was not accepted: {live:?}"
    );

    clean_up(&pool, run_id, &[&holder, &other]).await;
}

#[tokio::test]
async fn a_step_is_taken_over_only_from_a_runner_that_has_gone_quiet() {
    let pool = pool().await;
    let holder = format!("holder-{}", uuid::Uuid::new_v4());
    let other = format!("other-{}", uuid::Uuid::new_v4());
    insert_runner(&pool, &holder, 0.0).await;
    insert_runner(&pool, &other, 0.0).await;
    let (run_id, step_id) = insert_step_on(&pool, &holder).await;

    // The holder is alive: another runner cannot claim the step
    handler::step::events::handle_step_running(running(run_id, step_id, &other), pool.clone())
        .await
        .unwrap();
    assert_eq!(
        status_and_runner(&pool, step_id).await,
        Some((StepStatus::Running, Some(holder.clone())))
    );

    // The holder has missed a heartbeat, as it would have by the time its task is
    // redelivered: now the step follows the runner that picked it up
    sqlx::query(
        "UPDATE runners SET last_heartbeat_at = NOW() - INTERVAL '16 seconds' WHERE id = $1",
    )
    .bind(&holder)
    .execute(&pool)
    .await
    .unwrap();
    handler::step::events::handle_step_running(running(run_id, step_id, &other), pool.clone())
        .await
        .unwrap();
    assert_eq!(
        status_and_runner(&pool, step_id).await,
        Some((StepStatus::Running, Some(other.clone())))
    );

    clean_up(&pool, run_id, &[&holder, &other]).await;
}

#[tokio::test]
async fn registry_credentials_go_only_to_the_runner_that_has_the_step() {
    let pool = pool().await;
    let holder = format!("holder-{}", uuid::Uuid::new_v4());
    insert_runner(&pool, &holder, 0.0).await;
    let (run_id, step_id) = insert_step_on(&pool, &holder).await;

    // Unsigned: refused before anything else, whoever it names
    for asking in [json!(holder), json!("some-other-runner"), json!(null)] {
        let err = artifact_push_auth(
            &json!({"run_id": run_id, "step_id": step_id, "runner_id": asking}),
            &pool,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("no key"), "{asking}: {err}");
    }

    let signed_by = |runner: &str, key: &[u8; 32]| {
        stormchaser_model::runner_auth::sign_payload(
            json!({"run_id": run_id, "step_id": step_id, "runner_id": runner}),
            key,
        )
    };
    let other = format!("other-{}", uuid::Uuid::new_v4());
    stormchaser_engine::db::store_runner_session(&pool, &holder, &[1u8; 32])
        .await
        .unwrap();
    stormchaser_engine::db::store_runner_session(&pool, &other, &[2u8; 32])
        .await
        .unwrap();

    // Another runner, signing as itself, is not the one that has the step
    let err = artifact_push_auth(&signed_by(&other, &[2u8; 32]), &pool)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("assigned to another runner"),
        "{err}"
    );
    // Nor can it sign in the holder's name
    let err = artifact_push_auth(&signed_by(&holder, &[2u8; 32]), &pool)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not signed by runner"), "{err}");

    // The holder is answered. This step has no artifacts, so there is nothing in it.
    let config = artifact_push_auth(&signed_by(&holder, &[1u8; 32]), &pool)
        .await
        .unwrap();
    assert_eq!(config, json!({ "auths": {} }));

    sqlx::query("DELETE FROM runner_sessions WHERE runner_id = ANY($1)")
        .bind(vec![holder.clone(), other.clone()])
        .execute(&pool)
        .await
        .unwrap();
    clean_up(&pool, run_id, &[&holder]).await;
}
