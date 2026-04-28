use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use stormchaser_api::db;
use stormchaser_model::workflow::RunStatus;
use uuid::Uuid;

use stormchaser_api::ListRunsQuery;

#[tokio::test]
async fn test_db_functions() {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://stormchaser:stormchaser@localhost:5432/stormchaser".into());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();

    let run_id = Uuid::new_v4();
    let workflow_name = format!("test-db-workflow-{}", run_id);

    // insert_workflow_run
    db::insert_workflow_run(
        &mut tx,
        run_id,
        &workflow_name,
        "test-db-user",
        "https://github.com/test/repo",
        "workflow.storm",
        "HEAD",
        RunStatus::Running,
        1,
    )
    .await
    .unwrap();

    // insert_run_context
    db::insert_run_context(&mut tx, run_id, "1.0", json!({}), "test code", &json!({}))
        .await
        .unwrap();

    // insert_run_quotas
    db::insert_run_quotas(&mut tx, run_id, 10, "1", "1Gi", "10Gi", "1h")
        .await
        .unwrap();

    tx.commit().await.unwrap();

    // get_workflow_run_detail
    let detail = db::get_workflow_run_detail(&pool, run_id).await.unwrap();
    assert!(detail.is_some());

    // list_workflow_runs
    let params = ListRunsQuery {
        workflow_name: Some(workflow_name.clone()),
        status: None,
        initiating_user: None,
        repo_url: None,
        workflow_path: None,
        created_after: None,
        created_before: None,
        limit: Some(10),
        offset: Some(0),
    };
    let runs = db::list_workflow_runs(&pool, &params, 10, 0).await.unwrap();
    assert!(!runs.is_empty());

    // get_workflow_run_status
    let status = db::get_workflow_run_status(&pool, run_id).await.unwrap();
    assert_eq!(status, Some("running".into()));

    // get_step_instances
    let steps = db::get_step_instances(&pool, run_id).await.unwrap();
    assert!(steps.is_empty());

    // create_webhook
    let webhook_id = Uuid::new_v4();
    let webhook_name = format!("db-test-webhook-{}", webhook_id);
    db::create_webhook(
        &pool,
        webhook_id,
        &webhook_name,
        &None,
        "generic",
        &Some("secret".into()),
    )
    .await
    .unwrap();

    // get_webhook
    let webhook = db::get_webhook(&pool, webhook_id).await.unwrap();
    assert!(webhook.is_some());

    // list_webhooks
    let webhooks = db::list_webhooks(&pool).await.unwrap();
    assert!(!webhooks.is_empty());

    // create_event_rule
    let rule_id = Uuid::new_v4();
    let rule_name = format!("db-test-rule-{}", rule_id);
    db::create_event_rule(
        &pool,
        rule_id,
        &rule_name,
        &None,
        Some(webhook_id),
        "generic",
        &Some("1==1".into()),
        "test-flow",
        "url",
        "path",
        "HEAD",
        json!({}),
    )
    .await
    .unwrap();

    // list_event_rules
    let rules = db::list_event_rules(&pool).await.unwrap();
    assert!(!rules.is_empty());

    // create_cron_workflow
    let cron_id = Uuid::new_v4();
    let cron_name = format!("db-test-cron-{}", cron_id);
    db::create_cron_workflow(
        &pool,
        cron_id,
        &cron_name,
        &None,
        "0 0 * * *",
        "test-flow",
        "url",
        "path",
        "HEAD",
        &json!({}),
        "secret",
        &None,
    )
    .await
    .unwrap();

    // list_cron_workflows
    let crons = db::list_cron_workflows(&pool).await.unwrap();
    assert!(!crons.is_empty());

    // delete_cron_workflow
    db::delete_cron_workflow(&pool, cron_id).await.unwrap();

    // delete_event_rule
    db::delete_event_rule(&pool, rule_id).await.unwrap();

    // delete_webhook
    db::delete_webhook(&pool, webhook_id).await.unwrap();

    // test step id by name
    let step_id = db::get_step_id_by_name(&pool, run_id, "nonexistent")
        .await
        .unwrap();
    assert!(step_id.is_none());

    // test list_run_artifacts
    let artifacts = db::list_run_artifacts(&pool, run_id).await.unwrap();
    assert!(artifacts.is_empty());

    // test list_run_test_reports
    let reports = db::list_run_test_reports(&pool, run_id).await.unwrap();
    assert!(reports.is_empty());

    // Clean up
    db::delete_workflow_run(&pool, run_id).await.unwrap();
}
