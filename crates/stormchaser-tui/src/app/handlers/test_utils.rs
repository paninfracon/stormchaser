use crate::app::fixtures::{full_detail, run_detail, step_detail};
use crate::app::App;
use crate::app::{parse_step_status, StepDetail, WorkflowRunDetail, WorkflowRunFullDetail};
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::{RunId, StepInstanceId};
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
    run_detail(id, "test", status)
}

pub fn mock_step_detail(name: &str, status: &str) -> StepDetail {
    step_detail(
        StepInstanceId::new_v4(),
        name,
        parse_step_status(status).expect("a step status"),
    )
}

pub fn mock_full_detail(
    id: RunId,
    status: RunStatus,
    steps: Vec<StepDetail>,
) -> WorkflowRunFullDetail {
    full_detail(mock_run_detail(id, status), steps)
}
