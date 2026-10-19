//! Runs and steps for tests.

use super::{StepDetail, WorkflowRunDetail, WorkflowRunFullDetail};
use chrono::{DateTime, TimeZone, Utc};
use stormchaser_model::step::{StepInstance, StepStatus};
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::{RunId, StepInstanceId};

/// The time every fixture is created at.
pub fn created_at() -> DateTime<Utc> {
    Utc.timestamp_opt(1609459200, 0).unwrap()
}

pub fn run_detail(id: RunId, name: &str, status: RunStatus) -> WorkflowRunDetail {
    WorkflowRunDetail {
        id,
        workflow_name: name.to_string(),
        initiating_user: "jacrisp".to_string(),
        repo_url: String::new(),
        workflow_path: String::new(),
        git_ref: String::new(),
        status,
        version: 1,
        created_at: created_at(),
        updated_at: created_at(),
        started_resolving_at: None,
        started_at: None,
        finished_at: None,
        error: None,
        inputs: serde_json::json!({}),
        inputs_schema: None,
        source_code: String::new(),
        dsl_version: "1.0".to_string(),
    }
}

pub fn step_detail(id: StepInstanceId, name: &str, status: StepStatus) -> StepDetail {
    StepDetail {
        instance: StepInstance {
            id,
            run_id: RunId::new_v4(),
            step_name: name.to_string(),
            step_type: "docker".to_string(),
            status,
            iteration_index: None,
            runner_id: None,
            affinity_context: None,
            started_at: None,
            finished_at: None,
            exit_code: None,
            error: None,
            spec: serde_json::json!({}),
            params: serde_json::json!({}),
            created_at: created_at(),
        },
        outputs: vec![],
        history: vec![],
        logs: vec![],
    }
}

pub fn full_detail(detail: WorkflowRunDetail, steps: Vec<StepDetail>) -> WorkflowRunFullDetail {
    WorkflowRunFullDetail {
        detail,
        steps,
        artifacts: vec![],
        test_summaries: vec![],
        test_cases: vec![],
    }
}
