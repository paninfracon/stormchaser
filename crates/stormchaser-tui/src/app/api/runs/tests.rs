use super::*;
use chrono::Utc;
use tokio::sync::mpsc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_refresh_runs_success() {
    let server = MockServer::start().await;

    let run_detail = WorkflowRunDetail {
        id: RunId::new_v4(),
        workflow_name: "test".to_string(),
        initiating_user: "user".to_string(),
        status: RunStatus::Succeeded,
        created_at: Utc::now(),
        finished_at: None,
    };

    Mock::given(method("GET"))
        .and(path("/api/v1/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(vec![run_detail.clone()]))
        .mount(&server)
        .await;

    let (tx, _rx) = mpsc::channel(1);
    let mut app = App::new(
        server.uri(),
        "http://localhost:3001".to_string(),
        Some("token".to_string()),
        tx,
    );

    let result = app.refresh_runs().await;
    result.unwrap();
    assert_eq!(app.runs.len(), 1);
    assert_eq!(app.runs[0].id, run_detail.id);
    assert!(app.error.is_none());
}

#[tokio::test]
async fn test_refresh_runs_filters() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<WorkflowRunDetail>::new()))
        .mount(&server)
        .await;

    let (tx, _rx) = mpsc::channel(1);
    let mut app = App::new(
        server.uri(),
        "http://localhost:3001".to_string(),
        Some("token".to_string()),
        tx,
    );
    app.filter_owner = Some("test_user".to_string());
    app.filter_status = Some("failed".to_string());

    let result = app.refresh_runs().await;
    result.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0]
        .url
        .query()
        .unwrap()
        .contains("initiating_user=test_user"));
    assert!(requests[0].url.query().unwrap().contains("status=failed"));
}

#[tokio::test]
async fn test_fetch_run_detail_success() {
    let server = MockServer::start().await;
    let run_id = RunId::new_v4();

    let full_detail = crate::app::WorkflowRunFullDetail {
        detail: WorkflowRunDetail {
            id: run_id,
            workflow_name: "test".to_string(),
            initiating_user: "user".to_string(),
            status: RunStatus::Succeeded,
            created_at: Utc::now(),
            finished_at: None,
        },
        steps: vec![],
        artifacts: vec![],
        test_summaries: vec![],
        test_cases: vec![],
    };

    Mock::given(method("GET"))
        .and(path(format!("/api/v1/runs/{}", run_id)))
        .respond_with(ResponseTemplate::new(200).set_body_json(&full_detail))
        .mount(&server)
        .await;

    let (tx, _rx) = mpsc::channel(1);
    let mut app = App::new(
        server.uri(),
        "http://localhost:3001".to_string(),
        Some("token".to_string()),
        tx,
    );
    app.runs.push(full_detail.detail.clone());
    app.runs_state.select(Some(0));

    let result = app.fetch_run_detail(run_id).await;
    result.unwrap();
    assert!(app.selected_run.is_some());
    assert_eq!(app.selected_run.unwrap().detail.id, run_id);
}

#[tokio::test]
async fn test_start_listening_for_workflows_sse_success() {
    let server = MockServer::start().await;

    let run_detail = WorkflowRunDetail {
        id: RunId::new_v4(),
        workflow_name: "test-workflow".to_string(),
        initiating_user: "user".to_string(),
        status: RunStatus::Running,
        created_at: Utc::now(),
        finished_at: None,
    };

    let sse_body = format!(
        "event: workflow_run\ndata: {}\n\n",
        serde_json::to_string(&run_detail).unwrap()
    );

    Mock::given(method("GET"))
        .and(path("/api/v1/runs/stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .mount(&server)
        .await;

    let (tx, mut rx) = mpsc::channel(100);
    let mut app = App::new(
        server.uri(),
        "http://localhost:3001".to_string(),
        Some("token".to_string()),
        tx,
    );

    app.start_listening_for_workflows().await;

    assert!(app.workflow_handle.is_some());

    // We expect to receive a WorkflowUpdate event
    if let Ok(Some(event)) =
        tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
    {
        match event {
            AppEvent::WorkflowUpdate(received_run) => {
                assert_eq!(received_run.id, run_detail.id);
                assert_eq!(received_run.status, RunStatus::Running);
            }
            _ => panic!("Expected WorkflowUpdate event"),
        }
    } else {
        panic!("Did not receive WorkflowUpdate event from SSE stream in time");
    }
}

#[tokio::test]
async fn test_refresh_runs_clamps_out_of_range_selection() {
    let server = MockServer::start().await;
    let run_id = RunId::new_v4();

    let run_detail = WorkflowRunDetail {
        id: run_id,
        workflow_name: "test".to_string(),
        initiating_user: "user".to_string(),
        status: RunStatus::Running,
        created_at: Utc::now(),
        finished_at: None,
    };

    let full_detail = crate::app::WorkflowRunFullDetail {
        detail: run_detail.clone(),
        steps: vec![],
        artifacts: vec![],
        test_summaries: vec![],
        test_cases: vec![],
    };

    Mock::given(method("GET"))
        .and(path("/api/v1/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(vec![run_detail]))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/api/v1/runs/{}", run_id)))
        .respond_with(ResponseTemplate::new(200).set_body_json(full_detail))
        .mount(&server)
        .await;

    let (tx, _rx) = mpsc::channel(1);
    let mut app = App::new(
        server.uri(),
        "http://localhost:3001".to_string(),
        Some("token".to_string()),
        tx,
    );
    app.runs_state.select(Some(5));

    let result = app.refresh_runs().await;
    result.unwrap();
    assert_eq!(app.runs_state.selected(), Some(0));
}

#[tokio::test]
async fn test_delete_selected_run_clears_stale_selection_and_refreshes_details() {
    let server = MockServer::start().await;
    let first_run_id = RunId::new_v4();
    let second_run_id = RunId::new_v4();

    let first_run = WorkflowRunDetail {
        id: first_run_id,
        workflow_name: "workflow-1".to_string(),
        initiating_user: "user".to_string(),
        status: RunStatus::Running,
        created_at: Utc::now(),
        finished_at: None,
    };

    let second_run = WorkflowRunDetail {
        id: second_run_id,
        workflow_name: "workflow-2".to_string(),
        initiating_user: "user".to_string(),
        status: RunStatus::Queued,
        created_at: Utc::now(),
        finished_at: None,
    };

    let first_detail = crate::app::WorkflowRunFullDetail {
        detail: first_run.clone(),
        steps: vec![],
        artifacts: vec![],
        test_summaries: vec![],
        test_cases: vec![],
    };

    let second_detail = crate::app::WorkflowRunFullDetail {
        detail: second_run.clone(),
        steps: vec![],
        artifacts: vec![],
        test_summaries: vec![],
        test_cases: vec![],
    };

    Mock::given(method("DELETE"))
        .and(path(format!("/api/v1/runs/{}", second_run_id)))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v1/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(vec![first_run.clone()]))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/api/v1/runs/{}", first_run_id)))
        .respond_with(ResponseTemplate::new(200).set_body_json(first_detail))
        .mount(&server)
        .await;

    let (tx, _rx) = mpsc::channel(8);
    let mut app = App::new(
        server.uri(),
        "http://localhost:3001".to_string(),
        Some("token".to_string()),
        tx,
    );
    app.runs = vec![first_run, second_run];
    app.runs_state.select(Some(1));
    app.selected_run = Some(second_detail);

    let result = app.delete_selected_run().await;

    result.unwrap();
    assert_eq!(app.runs.len(), 1);
    assert_eq!(app.runs_state.selected(), Some(0));
    assert_eq!(
        app.selected_run.as_ref().map(|run| run.detail.id),
        Some(first_run_id)
    );
    assert!(app.error.is_none());
}
