use anyhow::Result;
use chrono::Utc;
use sqlx::PgPool;
use stormchaser_engine::step_machine::{state, StepMachine};
use stormchaser_model::step::StepStatus;
use stormchaser_model::workflow::RunStatus;
use uuid::Uuid;

// Helper to setup database test environment. Assuming a setup script handles standard test env setup.
async fn setup_db() -> Result<PgPool> {
    let pool = PgPool::connect(&std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://stormchaser:password@localhost:5432/stormchaser".to_string()
    }))
    .await?;
    Ok(pool)
}

#[tokio::test]
async fn test_step_machine_pending_fail() -> Result<()> {
    if std::env::var("SQL_OFFLINE").is_ok() {
        return Ok(());
    }

    let pool = match setup_db().await {
        Ok(p) => p,
        Err(_) => return Ok(()), // Skip if DB is unavailable
    };

    let mut tx = pool.begin().await?;

    let run_id = Uuid::new_v4();
    stormchaser_engine::db::insert_workflow_run(
        &mut *tx,
        run_id,
        "test-workflow",
        None,
        None,
        None,
        None,
        RunStatus::Running,
        None,
        Utc::now(),
        Utc::now(),
        None,
    )
    .await?;

    let step_id = Uuid::new_v4();
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
    if std::env::var("SQL_OFFLINE").is_ok() {
        return Ok(());
    }

    let pool = match setup_db().await {
        Ok(p) => p,
        Err(_) => return Ok(()), // Skip if DB is unavailable
    };

    let mut tx = pool.begin().await?;

    let run_id = Uuid::new_v4();
    stormchaser_engine::db::insert_workflow_run(
        &mut *tx,
        run_id,
        "test-workflow",
        None,
        None,
        None,
        None,
        RunStatus::Running,
        None,
        Utc::now(),
        Utc::now(),
        None,
    )
    .await?;

    let step_id = Uuid::new_v4();
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
