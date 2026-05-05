#![allow(clippy::explicit_auto_deref)]
use chrono::Utc;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use stormchaser_engine::step_machine::StepMachine;
use stormchaser_engine::workflow_machine::WorkflowMachine;
use stormchaser_model::step::{StepInstance, StepStatus};
use stormchaser_model::workflow::{RunStatus, WorkflowRun};
use uuid::Uuid;

async fn get_pool() -> sqlx::PgPool {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap()
}

fn create_test_run() -> WorkflowRun {
    let id = Uuid::new_v4();
    WorkflowRun {
        id,
        workflow_name: format!("test-workflow-{}", id),
        initiating_user: "test-user".to_string(),
        repo_url: "https://github.com/example/repo.git".to_string(),
        workflow_path: "workflow.storm".to_string(),
        git_ref: "main".to_string(),
        status: RunStatus::Queued,
        version: 1,
        fencing_token: 100,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_resolving_at: None,
        started_at: None,
        finished_at: None,
        error: None,
    }
}

async fn insert_test_run(pool: &sqlx::PgPool, run: &WorkflowRun) {
    sqlx::query(
        r#"INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, version, fencing_token, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#
    )
    .bind(run.id)
    .bind(&run.workflow_name)
    .bind(&run.initiating_user)
    .bind(&run.repo_url)
    .bind(&run.workflow_path)
    .bind(&run.git_ref)
    .bind(&run.status)
    .bind(run.version)
    .bind(run.fencing_token)
    .bind(run.created_at)
    .bind(run.updated_at)
    .execute(pool)
    .await
    .unwrap();
}

fn create_test_step(run_id: Uuid, name: &str) -> StepInstance {
    StepInstance {
        id: Uuid::new_v4(),
        run_id,
        step_name: name.to_string(),
        step_type: "RunContainer".to_string(),
        status: StepStatus::Pending,
        iteration_index: None,
        runner_id: None,
        affinity_context: None,
        started_at: None,
        finished_at: None,
        exit_code: None,
        error: None,
        spec: Value::Null,
        params: Value::Null,
        created_at: Utc::now(),
    }
}

async fn insert_test_step(pool: &sqlx::PgPool, step: &StepInstance) {
    sqlx::query(
        r#"INSERT INTO step_instances (id, run_id, step_name, step_type, status, created_at, spec, params)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#
    )
    .bind(step.id)
    .bind(step.run_id)
    .bind(&step.step_name)
    .bind(&step.step_type)
    .bind(&step.status)
    .bind(step.created_at)
    .bind(&step.spec)
    .bind(&step.params)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn test_workflow_step_success_interaction() {
    let pool = get_pool().await;
    let run = create_test_run();
    let run_id = run.id;
    insert_test_run(&pool, &run).await;

    let mut conn = pool.acquire().await.unwrap();

    let wf_machine = WorkflowMachine::new(run)
        .start_resolving(&mut conn)
        .await
        .unwrap()
        .start_pending(&mut conn)
        .await
        .unwrap()
        .start(&mut conn)
        .await
        .unwrap();

    // Now workflow is Running. Let's create steps.
    let step1 = create_test_step(run_id, "step1");
    let step2 = create_test_step(run_id, "step2");
    insert_test_step(&pool, &step1).await;
    insert_test_step(&pool, &step2).await;

    let step_machine1 = StepMachine::new(step1)
        .start("runner-1".to_string(), &mut conn)
        .await
        .unwrap()
        .succeed(&mut conn)
        .await
        .unwrap();

    let step_machine2 = StepMachine::new(step2)
        .start("runner-2".to_string(), &mut conn)
        .await
        .unwrap()
        .succeed(&mut conn)
        .await
        .unwrap();

    assert_eq!(step_machine1.into_instance().status, StepStatus::Succeeded);
    assert_eq!(step_machine2.into_instance().status, StepStatus::Succeeded);

    // All steps succeeded, so workflow succeeds.
    let wf_machine = wf_machine.succeed(&mut conn).await.unwrap();
    assert_eq!(wf_machine.into_run().status, RunStatus::Succeeded);

    // Cleanup
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_workflow_step_failure_interaction() {
    let pool = get_pool().await;
    let run = create_test_run();
    let run_id = run.id;
    insert_test_run(&pool, &run).await;

    let mut conn = pool.acquire().await.unwrap();

    let wf_machine = WorkflowMachine::new(run)
        .start_resolving(&mut conn)
        .await
        .unwrap()
        .start_pending(&mut conn)
        .await
        .unwrap()
        .start(&mut conn)
        .await
        .unwrap();

    let step1 = create_test_step(run_id, "step1");
    insert_test_step(&pool, &step1).await;

    let step_machine1 = StepMachine::new(step1)
        .start("runner-1".to_string(), &mut conn)
        .await
        .unwrap()
        .fail("failed to execute".to_string(), Some(1), &mut conn)
        .await
        .unwrap();

    assert_eq!(step_machine1.into_instance().status, StepStatus::Failed);

    // Step failed, so workflow fails.
    let wf_machine = wf_machine
        .fail("step failed".to_string(), &mut conn)
        .await
        .unwrap();
    assert_eq!(wf_machine.into_run().status, RunStatus::Failed);

    // Cleanup
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_workflow_abort_cascades_to_step() {
    let pool = get_pool().await;
    let run = create_test_run();
    let run_id = run.id;
    insert_test_run(&pool, &run).await;

    let mut conn = pool.acquire().await.unwrap();

    let wf_machine = WorkflowMachine::new(run)
        .start_resolving(&mut conn)
        .await
        .unwrap()
        .start_pending(&mut conn)
        .await
        .unwrap()
        .start(&mut conn)
        .await
        .unwrap();

    let step1 = create_test_step(run_id, "step1");
    insert_test_step(&pool, &step1).await;
    let step_machine1 = StepMachine::new(step1)
        .start("runner-1".to_string(), &mut conn)
        .await
        .unwrap();

    // User aborts workflow
    let wf_machine = wf_machine.abort(&mut conn).await.unwrap();

    // Engine should cascade abort to running steps
    let step_machine1 = step_machine1.abort(&mut conn).await.unwrap();

    assert_eq!(wf_machine.into_run().status, RunStatus::Aborted);
    assert_eq!(step_machine1.into_instance().status, StepStatus::Aborted);

    // Cleanup
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
}
