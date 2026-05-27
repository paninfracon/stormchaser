use chrono::Utc;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::env::var;
use std::sync::Arc;
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::step::{StepInstance, StepStatus};
use stormchaser_model::RunId;

use stormchaser_tls::TlsConfig;
use stormchaser_tls::TlsReloader;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn test_heavy_load_queues_and_quotas() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,stormchaser_engine=debug")
        .try_init();

    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(50)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));
    let tls_reloader = Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap());
    let log_backend = Arc::new(None);

    let run_id = RunId::new_v4();

    // Generate an array of 500 items for iteration
    let items: Vec<String> = (0..500).map(|i| i.to_string()).collect();
    let iterate_json = serde_json::to_string(&items).unwrap();
    let iterate_escaped = iterate_json.replace("\"", "\\\"");

    let dsl = format!(
        r#"
        stormchaser_dsl_version = "v1"
        workflow "load-test-quotas" {{
            quotas {{
                max_concurrency = 50
            }}
            steps {{
                step "generate" "RunContainer" {{
                    image = "alpine"
                    next = ["process"]
                }}
                step "process" "RunContainer" {{
                    iterate = "{}"
                    iterate_as = "item"
                    params = {{
                        val = "${{item}}"
                    }}
                    image = "alpine"
                }}
            }}
        }}
        "#,
        iterate_escaped
    );

    let subscriber = nats_client.subscribe("stormchaser.v1.>").await.unwrap();
    let (tx_nats, mut rx_nats) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        use futures::StreamExt;
        let mut stream = subscriber.fuse();
        while let Some(msg) = stream.next().await {
            let _ = tx_nats.send(msg);
        }
    });

    // 1. Trigger handle_workflow_direct
    let payload = json!({
        "run_id": run_id,
        "dsl": dsl,
        "inputs": {},
        "initiating_user": "load-test-user"
    });

    handler::handle_workflow_direct(
        payload,
        pool.clone(),
        opa_client.clone(),
        nats_client.clone(),
    )
    .await
    .unwrap();

    // 2. Schedule initial steps
    handler::handle_workflow_start_pending(
        run_id,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();

    // 3. Worker loop
    let mut active_dispatched = std::collections::HashSet::new();
    let mut completed_generate = false;
    let mut completed_process_count = 0;

    let fencing_token: i64 =
        sqlx::query_scalar("SELECT fencing_token FROM workflow_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .expect("workflow run fencing token should be queryable");

    loop {
        // Collect all available NATS messages
        while let Ok(msg) =
            tokio::time::timeout(std::time::Duration::from_millis(50), rx_nats.recv()).await
        {
            if let Some(msg) = msg {
                if let Ok(event) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                    if event["type"].as_str() == Some("StepScheduledEvent") {
                        if let Ok(payload) = serde_json::from_value::<
                            stormchaser_model::events::StepScheduledEvent,
                        >(event["data"].clone())
                        {
                            if payload.run_id == run_id {
                                active_dispatched.insert(payload.step_id);
                            }
                        }
                    }
                }
            }
        }

        let active_count = active_dispatched.len();
        println!("DEBUG: active_dispatched has {} items", active_count);
        assert!(
            active_count <= 51,
            "Quota exceeded! Found {} active steps",
            active_count
        ); // 50 process + 1 generate

        let to_complete: Vec<_> = active_dispatched.drain().collect();
        println!("DEBUG: to_complete has {} items", to_complete.len());

        if to_complete.is_empty() {
            // Check if workflow is completely finished
            let all_finished: bool = sqlx::query_scalar(
                "SELECT COUNT(*) = 0 FROM step_instances WHERE run_id = $1 AND status != $2 AND status != $3"
            )
            .bind(run_id)
            .bind(StepStatus::Succeeded)
            .bind(StepStatus::Failed)
            .fetch_one(&pool)
            .await
            .unwrap_or(false);

            if all_finished && completed_generate {
                break;
            }

            // Print debug info if we are looping without doing anything
            let counts: Vec<(StepStatus, i64)> = sqlx::query_as(
                "SELECT status, COUNT(*) FROM step_instances WHERE run_id = $1 GROUP BY status",
            )
            .bind(run_id)
            .fetch_all(&pool)
            .await
            .unwrap();
            println!("DEBUG: Waiting. Current step counts: {:?}", counts);
        }

        for chunk in to_complete.chunks(10) {
            let mut futures = vec![];
            for step_id in chunk {
                let pool = pool.clone();
                let nats_client = nats_client.clone();
                let log_backend = log_backend.clone();
                let tls_reloader = tls_reloader.clone();
                let step_id = *step_id;

                futures.push(tokio::spawn(async move {
                    let instance: StepInstance = sqlx::query_as(r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE id = $1"#)
                        .bind(step_id)
                        .fetch_one(&pool)
                        .await
                        .unwrap();

                    // Simulate runner changing state to Running to properly acquire quotas
                    sqlx::query("UPDATE step_instances SET status = $1 WHERE id = $2")
                        .bind(StepStatus::Running)
                        .bind(step_id)
                        .execute(&pool)
                        .await
                        .unwrap();

                    let completed_payload = serde_json::json!({
                        "run_id": run_id,
                        "step_id": step_id,
                        "event_type": "StepCompletedEvent",
                        "fencing_token": fencing_token,
                        "timestamp": Utc::now(),
                        "exit_code": 0
                    });

                    handler::handle_step_completed(
                        serde_json::from_value(completed_payload).unwrap(),
                        pool.clone(),
                        nats_client.clone(),
                        log_backend.clone(),
                        tls_reloader.clone(),
                    )
                    .await
                    .unwrap();

                    instance.step_name
                }));
            }

            let results = futures::future::join_all(futures).await;
            for step_name in results.into_iter().flatten() {
                if step_name == "generate" {
                    completed_generate = true;
                } else if step_name == "process" {
                    completed_process_count += 1;
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(
        completed_process_count, 500,
        "Should have completed exactly 500 process steps"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn test_heavy_load_multiple_workflows() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,stormchaser_engine=debug")
        .try_init();

    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(50)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));
    let tls_reloader = Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap());
    let log_backend = Arc::new(None);

    let dsl = r#"
        stormchaser_dsl_version = "v1"
        workflow "load-test-multiple" {
            steps {
                step "generate" "RunContainer" {
                    image = "alpine"
                }
            }
        }
    "#;

    let subscriber = nats_client.subscribe("stormchaser.v1.>").await.unwrap();
    let (tx_nats, mut rx_nats) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        use futures::StreamExt;
        let mut stream = subscriber.fuse();
        while let Some(msg) = stream.next().await {
            let _ = tx_nats.send(msg);
        }
    });

    let mut all_run_ids = std::collections::HashSet::new();

    for _ in 0..500 {
        let run_id = RunId::new_v4();
        all_run_ids.insert(run_id);

        let payload = json!({
            "run_id": run_id,
            "dsl": dsl,
            "inputs": {},
            "initiating_user": "load-test-user"
        });

        // Trigger direct run (DB insert and publish)
        handler::handle_workflow_direct(
            payload,
            pool.clone(),
            opa_client.clone(),
            nats_client.clone(),
        )
        .await
        .unwrap();

        // Directly kickstart it
        handler::handle_workflow_start_pending(
            run_id,
            pool.clone(),
            nats_client.clone(),
            tls_reloader.clone(),
        )
        .await
        .unwrap();
    }

    // 3. Worker loop
    let mut active_dispatched = std::collections::HashSet::new();
    let mut completed_generate_count = 0;

    let fencing_tokens: std::collections::HashMap<RunId, i64> = sqlx::query(
        "SELECT id, fencing_token FROM workflow_runs WHERE initiating_user = 'load-test-user'",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| {
        use sqlx::Row;
        let id: RunId = row.get("id");
        let token: i64 = row.get("fencing_token");
        (id, token)
    })
    .collect();

    loop {
        // Collect all available NATS messages
        while let Ok(msg) =
            tokio::time::timeout(std::time::Duration::from_millis(50), rx_nats.recv()).await
        {
            if let Some(msg) = msg {
                if let Ok(event) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                    if event["type"].as_str() == Some("StepScheduledEvent") {
                        if let Ok(payload) = serde_json::from_value::<
                            stormchaser_model::events::StepScheduledEvent,
                        >(event["data"].clone())
                        {
                            if all_run_ids.contains(&payload.run_id) {
                                active_dispatched.insert(payload.step_id);
                            }
                        }
                    }
                }
            }
        }

        let to_complete: Vec<_> = active_dispatched.drain().collect();

        if to_complete.is_empty() {
            if completed_generate_count == 500 {
                break;
            }
            println!(
                "DEBUG: completed {}/500 workflows, waiting...",
                completed_generate_count
            );
        }

        for chunk in to_complete.chunks(25) {
            let mut futures = vec![];
            for step_id in chunk {
                let pool = pool.clone();
                let nats_client = nats_client.clone();
                let log_backend = log_backend.clone();
                let tls_reloader = tls_reloader.clone();
                let fencing_tokens = fencing_tokens.clone();
                let step_id = *step_id;

                futures.push(tokio::spawn(async move {
                    let instance: StepInstance = sqlx::query_as(r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE id = $1"#)
                        .bind(step_id)
                        .fetch_one(&pool)
                        .await
                        .unwrap();

                    sqlx::query("UPDATE step_instances SET status = $1 WHERE id = $2")
                        .bind(StepStatus::Running)
                        .bind(step_id)
                        .execute(&pool)
                        .await
                        .unwrap();

                    let fencing_token = fencing_tokens.get(&instance.run_id).unwrap();

                    let completed_payload = serde_json::json!({
                        "run_id": instance.run_id,
                        "step_id": step_id,
                        "event_type": "StepCompletedEvent",
                        "fencing_token": fencing_token,
                        "timestamp": Utc::now(),
                        "exit_code": 0
                    });

                    handler::handle_step_completed(
                        serde_json::from_value(completed_payload).unwrap(),
                        pool.clone(),
                        nats_client.clone(),
                        log_backend.clone(),
                        tls_reloader.clone(),
                    )
                    .await
                    .unwrap();

                    instance.step_name
                }));
            }

            let results = futures::future::join_all(futures).await;
            for step_name in results.into_iter().flatten() {
                if step_name == "generate" {
                    completed_generate_count += 1;
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(
        completed_generate_count, 500,
        "Should have completed exactly 500 distinct workflows"
    );
}
