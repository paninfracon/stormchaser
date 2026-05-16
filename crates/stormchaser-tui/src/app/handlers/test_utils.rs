use crate::app::App;
use crate::app::{StepDetail, WorkflowRunDetail, WorkflowRunFullDetail};
use chrono::Utc;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;
use tokio::sync::mpsc;

pub fn setup_app() -> App<'static> {
    let (tx, _rx) = mpsc::channel(1);
    App::new(
        "http://test".to_string(),
        "http://localhost:3001".to_string(),
        Some("token".to_string()),
        tx,
    )
}

pub fn mock_run_detail(id: RunId, status: RunStatus) -> WorkflowRunDetail {
    WorkflowRunDetail {
        id,
        workflow_name: "test".to_string(),
        initiating_user: "user".to_string(),
        status,
        created_at: Utc::now(),
        finished_at: None,
    }
}

pub fn mock_step_detail(name: &str, status: &str) -> StepDetail {
    StepDetail {
        instance: serde_json::json!({
            "step_name": name,
            "status": status,
        }),
        outputs: vec![],
        history: vec![],
        logs: vec![],
    }
}

pub fn mock_full_detail(
    id: RunId,
    status: RunStatus,
    steps: Vec<StepDetail>,
) -> WorkflowRunFullDetail {
    WorkflowRunFullDetail {
        detail: mock_run_detail(id, status),
        steps,
        artifacts: vec![],
        test_summaries: vec![],
        test_cases: vec![],
    }
}
