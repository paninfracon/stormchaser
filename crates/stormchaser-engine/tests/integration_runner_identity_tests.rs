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
            stormchaser_model::nats::ARTIFACT_AUTH_SUBJECT,
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

#[tokio::test]
async fn a_step_no_runner_has_claimed_takes_a_result_only_from_a_runner_with_a_key() {
    let pool = pool().await;
    let (nats, tls) = (nats().await, tls().await);
    let keyed = format!("keyed-{}", uuid::Uuid::new_v4());
    insert_runner(&pool, &keyed, 0.0).await;
    stormchaser_engine::db::store_runner_session(&pool, &keyed, &[9u8; 32])
        .await
        .unwrap();
    let (run_id, step_id) = insert_step_on(&pool, &keyed).await;
    sqlx::query("UPDATE step_instances SET runner_id = NULL WHERE id = $1")
        .bind(step_id)
        .execute(&pool)
        .await
        .unwrap();

    // Naming no runner, or one the engine never gave a key: anyone could send that
    let made_up = format!("made-up-{}", uuid::Uuid::new_v4());
    for reporter in [None, Some(made_up.as_str())] {
        handler::step::events::handle_step_failed(
            failed(run_id, step_id, reporter),
            pool.clone(),
            nats.clone(),
            tls.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            status_and_runner(&pool, step_id).await,
            Some((StepStatus::Running, None)),
            "a result from {reporter:?} was accepted"
        );
    }

    // A runner with a key (whose signature the router has checked) is taken as the one
    // that has it
    handler::step::events::handle_step_completed(
        completed(run_id, step_id, Some(&keyed)),
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
        "the keyed runner's result was not accepted: {live:?}"
    );

    sqlx::query("DELETE FROM runner_sessions WHERE runner_id = $1")
        .bind(&keyed)
        .execute(&pool)
        .await
        .unwrap();
    clean_up(&pool, run_id, &[&keyed]).await;
}

#[tokio::test]
async fn storage_reports_move_a_step_only_for_the_runner_that_holds_it() {
    let pool = pool().await;
    let holder = format!("holder-{}", uuid::Uuid::new_v4());
    let other = format!("other-{}", uuid::Uuid::new_v4());
    insert_runner(&pool, &holder, 0.0).await;
    insert_runner(&pool, &other, 0.0).await;
    let (run_id, step_id) = insert_step_on(&pool, &holder).await;
    let report = |runner: &str| json!({"run_id": run_id, "step_id": step_id, "runner_id": runner});

    // Another runner cannot claim a running step by saying it is unpacking it, nor
    // push it into packing
    handler::handle_step_unpacking_sfs(report(&other), pool.clone())
        .await
        .unwrap();
    handler::handle_step_packing_sfs(report(&other), pool.clone())
        .await
        .unwrap();
    assert_eq!(
        status_and_runner(&pool, step_id).await,
        Some((StepStatus::Running, Some(holder.clone())))
    );

    // Nor can the holder send a running step back to unpacking
    handler::handle_step_unpacking_sfs(report(&holder), pool.clone())
        .await
        .unwrap();
    assert_eq!(
        status_and_runner(&pool, step_id).await,
        Some((StepStatus::Running, Some(holder.clone())))
    );

    // The holder packs it
    handler::handle_step_packing_sfs(report(&holder), pool.clone())
        .await
        .unwrap();
    assert_eq!(
        status_and_runner(&pool, step_id).await,
        Some((StepStatus::PackingSfs, Some(holder.clone())))
    );

    // A finished step cannot be reopened by an unpacking report
    sqlx::query("UPDATE step_instances SET status = 'succeeded' WHERE id = $1")
        .bind(step_id)
        .execute(&pool)
        .await
        .unwrap();
    handler::handle_step_unpacking_sfs(report(&other), pool.clone())
        .await
        .unwrap();
    assert_eq!(
        status_and_runner(&pool, step_id).await,
        Some((StepStatus::Succeeded, Some(holder.clone())))
    );

    // A pending step is claimed by the runner that unpacks it, and then runs for it. A
    // step of its own (another iteration of the same step): the database does not let a
    // finished step be reopened.
    let step_id = StepInstanceId::new_v4();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, iteration_index, status, step_type, spec, completion_token) VALUES ($1, $2, 'build', 1, 'pending', 'RunContainer', '{}', NULL)")
        .bind(step_id)
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
    let report = |runner: &str| json!({"run_id": run_id, "step_id": step_id, "runner_id": runner});
    handler::handle_step_unpacking_sfs(report(&holder), pool.clone())
        .await
        .unwrap();
    assert_eq!(
        status_and_runner(&pool, step_id).await,
        Some((StepStatus::UnpackingSfs, Some(holder.clone())))
    );
    handler::handle_step_running(running(run_id, step_id, &other), pool.clone())
        .await
        .unwrap();
    assert_eq!(
        status_and_runner(&pool, step_id).await,
        Some((StepStatus::UnpackingSfs, Some(holder.clone())))
    );
    handler::handle_step_running(running(run_id, step_id, &holder), pool.clone())
        .await
        .unwrap();
    assert_eq!(
        status_and_runner(&pool, step_id).await,
        Some((StepStatus::Running, Some(holder.clone())))
    );

    clean_up(&pool, run_id, &[&holder, &other]).await;
}

#[tokio::test]
async fn a_signature_holds_only_on_the_subject_it_was_made_for() {
    use stormchaser_engine::runner_auth::{refusal, Sender};
    let pool = pool().await;
    let runner = format!("signer-{}", uuid::Uuid::new_v4());
    let key = [11u8; 32];
    insert_runner(&pool, &runner, 0.0).await;
    stormchaser_engine::db::store_runner_session(&pool, &runner, &key)
        .await
        .unwrap();

    let signed = stormchaser_model::runner_auth::sign_payload(
        json!({"run_id": RunId::new_v4(), "step_id": StepInstanceId::new_v4(), "runner_id": runner}),
        &key,
        "stormchaser.v1.4.step.running",
    );
    assert_eq!(
        refusal(
            &pool,
            Sender::Runner,
            "stormchaser.v1.step.running",
            &signed,
            chrono::Utc::now().timestamp()
        )
        .await
        .unwrap(),
        None
    );
    // The same message, published again on the subject that takes a runner offline
    assert!(refusal(
        &pool,
        Sender::Runner,
        "stormchaser.v1.runner.offline",
        &signed,
        chrono::Utc::now().timestamp()
    )
    .await
    .unwrap()
    .is_some());

    clean_up(&pool, RunId::new_v4(), &[&runner]).await;
}

#[tokio::test]
async fn the_engines_verdict_on_a_lost_runner_needs_no_signature() {
    use stormchaser_engine::runner_auth::{refusal, Sender};
    let pool = pool().await;
    let gone = format!("gone-{}", uuid::Uuid::new_v4());
    insert_runner(&pool, &gone, 120.0).await;
    stormchaser_engine::db::store_runner_session(&pool, &gone, &[12u8; 32])
        .await
        .unwrap();
    let (run_id, step_id) = insert_step_on(&pool, &gone).await;

    let mut verdict = failed(run_id, step_id, Some(&gone));
    verdict.error = "lost_zombie".to_string();
    let payload = serde_json::to_value(&verdict).unwrap();
    assert_eq!(
        refusal(
            &pool,
            Sender::RunnerOrControlPlane,
            "stormchaser.v1.step.failed",
            &payload,
            chrono::Utc::now().timestamp()
        )
        .await
        .unwrap(),
        None
    );
    // Any other unsigned failure naming that runner is still refused
    let payload = serde_json::to_value(failed(run_id, step_id, Some(&gone))).unwrap();
    assert!(refusal(
        &pool,
        Sender::RunnerOrControlPlane,
        "stormchaser.v1.step.failed",
        &payload,
        chrono::Utc::now().timestamp()
    )
    .await
    .unwrap()
    .is_some());

    clean_up(&pool, run_id, &[&gone]).await;
}

#[tokio::test]
async fn a_fresh_key_cannot_be_taken_and_the_engines_ids_cannot_be_had() {
    let pool = pool().await;
    let id = format!("claim-{}", uuid::Uuid::new_v4());
    assert!(
        stormchaser_engine::db::claim_runner_session(&pool, &id, &[1u8; 32])
            .await
            .unwrap()
    );
    // Before the runner has sent a heartbeat, someone else asking for its id
    assert!(
        !stormchaser_engine::db::claim_runner_session(&pool, &id, &[2u8; 32])
            .await
            .unwrap()
    );
    assert_eq!(
        stormchaser_engine::db::get_runner_session_key(&pool, &id)
            .await
            .unwrap(),
        Some(vec![1u8; 32])
    );
    // Once the key is old and its runner silent, the id may be taken again
    sqlx::query(
        "UPDATE runner_sessions SET issued_at = NOW() - INTERVAL '5 minutes' WHERE runner_id = $1",
    )
    .bind(&id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        stormchaser_engine::db::claim_runner_session(&pool, &id, &[3u8; 32])
            .await
            .unwrap()
    );

    for reserved in ["intrinsic-sql", "engine-intrinsic"] {
        assert!(stormchaser_engine::runner_auth::is_reserved_runner_id(
            reserved
        ));
    }
    sqlx::query("DELETE FROM runner_sessions WHERE runner_id = $1")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_message_that_cannot_be_checked_is_not_refused() {
    use stormchaser_engine::runner_auth::{refusal, Sender};
    let pool = pool().await;
    // The database cannot be read: the message must be tried again, not dropped
    pool.close().await;
    let payload = json!({"run_id": RunId::new_v4(), "step_id": StepInstanceId::new_v4(), "runner_id": "anyone"});
    assert!(refusal(
        &pool,
        Sender::RunnerOrControlPlane,
        "stormchaser.v1.step.completed",
        &payload,
        chrono::Utc::now().timestamp()
    )
    .await
    .is_err());
}
