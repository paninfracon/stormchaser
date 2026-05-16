use anyhow::Result;
use chrono::Utc;
use sqlx::PgPool;
use std::env::var;
use stormchaser_engine::step_machine::{state, StepMachine};
use stormchaser_model::step::StepStatus;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;

// Helper to setup database test environment. Assuming a setup script handles standard test env setup.
async fn setup_db() -> Result<PgPool> {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set (run scripts/setup.sh first)")
        )
    });
    let pool = PgPool::connect(&db_url).await?;
    Ok(pool)
}

#[tokio::test]
async fn test_step_machine_pending_fail() -> Result<()> {
    if var("SQL_OFFLINE").is_ok() {
        return Ok(());
    }

    let pool = setup_db().await?;

    let mut tx = pool.begin().await?;

    let run_id = RunId::new_v4();
    stormchaser_engine::db::insert_workflow_run(
        &mut *tx,
        run_id,
        "test-workflow",
        Some("test-user"),
        Some("http://example.com"),
        Some("main.storm"),
        Some("main"),
        RunStatus::Running,
        Some(1),
        Utc::now(),
        Utc::now(),
        None,
    )
    .await?;

    let step_id = StepInstanceId::new_v4();
    stormchaser_engine::db::insert_step_instance_with_spec(
        &mut *tx,
        step_id,
        run_id,
        "test",
        "docker",
        StepStatus::Pending,
        None,
        serde_json::json!({}),
        serde_json::json!({}),
        Utc::now(),
    )
    .await?;

    let instance = stormchaser_engine::db::get_step_instance_by_id(&mut *tx, step_id)
        .await?
        .unwrap();

    let machine = StepMachine::<state::Pending>::from_instance(instance);
    let failed_machine = machine
        .fail("Test failure".to_string(), None, &mut tx)
        .await?;

    assert_eq!(failed_machine.instance.status, StepStatus::Failed);
    assert_eq!(
        failed_machine.instance.error,
        Some("Test failure".to_string())
    );
    assert!(failed_machine.instance.finished_at.is_some());

    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn test_step_machine_waiting_for_event_fail() -> Result<()> {
    if var("SQL_OFFLINE").is_ok() {
        return Ok(());
    }

    let pool = setup_db().await?;

    let mut tx = pool.begin().await?;

    let run_id = RunId::new_v4();
    stormchaser_engine::db::insert_workflow_run(
        &mut *tx,
        run_id,
        "test-workflow",
        Some("test-user"),
        Some("http://example.com"),
        Some("main.storm"),
        Some("main"),
        RunStatus::Running,
        Some(1),
        Utc::now(),
        Utc::now(),
        None,
    )
    .await?;

    let step_id = StepInstanceId::new_v4();
    stormchaser_engine::db::insert_step_instance_with_spec(
        &mut *tx,
        step_id,
        run_id,
        "test_event",
        "docker",
        StepStatus::WaitingForEvent,
        None,
        serde_json::json!({}),
        serde_json::json!({}),
        Utc::now(),
    )
    .await?;

    let instance = stormchaser_engine::db::get_step_instance_by_id(&mut *tx, step_id)
        .await?
        .unwrap();

    let machine = StepMachine::<state::WaitingForEvent>::from_instance(instance);
    let failed_machine = machine
        .fail("Event failure".to_string(), None, &mut tx)
        .await?;

    assert_eq!(failed_machine.instance.status, StepStatus::Failed);
    assert_eq!(
        failed_machine.instance.error,
        Some("Event failure".to_string())
    );
    assert!(failed_machine.instance.finished_at.is_some());

    tx.rollback().await?;
    Ok(())
}
