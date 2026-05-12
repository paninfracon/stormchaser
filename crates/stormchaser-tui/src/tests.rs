use crate::app::WorkflowRunDetail;
use crate::app::{App, AppState, Pane};
use crate::ui::ui;
use chrono::{TimeZone, Utc};
use insta::assert_debug_snapshot;
use ratatui::{backend::TestBackend, Terminal};
use ratatui_textarea::TextArea;
use stormchaser_model::connections::ArtifactRegistry;
use stormchaser_model::cron::CronWorkflow;
use stormchaser_model::event_rules::EventRule;
use stormchaser_model::test_report::TestCase;
use stormchaser_model::test_report::TestCaseStatus;
use stormchaser_model::test_report::TestSummary;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::CronWorkflowId;
use stormchaser_model::RuleId;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_model::TestReportId;
use stormchaser_model::WebhookId;
use stormchaser_model::{
    connections::{Connection, ConnectionType},
    event_rules::WebhookConfig,
};
use uuid::Uuid;

fn create_test_app<'a>() -> App<'a> {
    let (tx, _) = tokio::sync::mpsc::channel(100);
    let mut app = App::new(
        "http://localhost:3000".to_string(),
        "http://localhost:3001".to_string(),
        Some("token".to_string()),
        tx,
    );
    app.state = AppState::LoggedIn;
    app
}

#[test]
fn render_connections_tab() {
    let mut app = create_test_app();
    app.active_pane = Pane::ConnectionsList;

    let created_at = Utc.timestamp_opt(1609459200, 0).unwrap();
    let updated_at = Utc.timestamp_opt(1609459200, 0).unwrap();

    app.connections = vec![Connection {
        id: stormchaser_model::ConnectionId::new(Uuid::nil()),
        name: "test-sfs".to_string(),
        description: Some("Test SFS".to_string()),
        connection_type: ConnectionType::S3,
        is_default_sfs: true,
        encrypted_credentials: None,
        config: serde_json::json!({"path": "/tmp/sfs"}),
        aws_assume_role_arn: None,
        ca_cert: None,
        client_cert: None,
        client_key: None,
        created_at,
        updated_at,
    }];
    app.selected_connection = app.connections.first().cloned();

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_connection_dialog() {
    let mut app = create_test_app();
    app.active_pane = Pane::ConnectionsList;
    app.connection_dialog_active = true;
    app.connection_inputs = vec![
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
    ];
    app.connection_inputs[0].insert_str("new-sfs");
    app.connection_inputs[1].insert_str("Desc");
    app.connection_inputs[2].insert_str("{}");

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_webhooks_tab() {
    let mut app = create_test_app();
    app.active_pane = Pane::WebhooksList;

    let created_at = Utc.timestamp_opt(1609459200, 0).unwrap();
    let updated_at = Utc.timestamp_opt(1609459200, 0).unwrap();

    app.webhooks = vec![WebhookConfig {
        id: WebhookId::new(Uuid::nil()),
        name: "test-webhook".to_string(),
        description: Some("Test Webhook".to_string()),
        source_type: "github".to_string(),
        is_active: true,
        secret_token: None,
        ca_cert: None,
        client_cert: None,
        client_key: None,
        created_at,
        updated_at,
    }];
    app.selected_webhook = app.webhooks.first().cloned();

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_event_rules_tab() {
    let mut app = create_test_app();
    app.active_pane = Pane::EventRulesList;

    let created_at = Utc.timestamp_opt(1609459200, 0).unwrap();
    let updated_at = Utc.timestamp_opt(1609459200, 0).unwrap();

    app.event_rules = vec![EventRule {
        id: RuleId::new(Uuid::nil()),
        name: "test-rule".to_string(),
        description: Some("Test Rule".to_string()),
        webhook_id: Some(WebhookId::new(Uuid::nil())),
        event_type_pattern: "push".to_string(),
        condition_expr: Some("payload.ref == 'refs/heads/main'".to_string()),
        workflow_name: "test".to_string(),
        repo_url: "http://example.com".to_string(),
        workflow_path: "test.storm".to_string(),
        git_ref: "main".to_string(),
        input_mappings: serde_json::json!({}),
        is_active: true,
        created_at,
        updated_at,
    }];
    app.selected_event_rule = app.event_rules.first().cloned();

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_event_rule_dialog() {
    let mut app = create_test_app();
    app.active_pane = Pane::EventRulesList;
    app.event_rule_dialog_active = true;
    app.event_rule_inputs = vec![
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
    ];
    app.event_rule_inputs[0].insert_str("new-rule");
    app.event_rule_inputs[3].insert_str("push");
    app.event_rule_inputs[5].insert_str("test");

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_cron_workflows_tab() {
    let mut app = create_test_app();
    app.active_pane = Pane::CronWorkflowsList;

    let created_at = Utc.timestamp_opt(1609459200, 0).unwrap();
    let updated_at = Utc.timestamp_opt(1609459200, 0).unwrap();

    app.cron_workflows = vec![CronWorkflow {
        id: CronWorkflowId::new(Uuid::nil()),
        name: "test-cron".to_string(),
        description: Some("Test Cron".to_string()),
        cronspec: "* * * * *".to_string(),
        workflow_name: "test".to_string(),
        repo_url: "http://example.com".to_string(),
        workflow_path: "test.storm".to_string(),
        git_ref: "main".to_string(),
        inputs: serde_json::json!({}),
        secret_token: "secret".to_string(),
        is_active: true,
        external_job_id: None,
        created_at,
        updated_at,
    }];
    app.selected_cron_workflow = app.cron_workflows.first().cloned();

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_cron_dialog() {
    let mut app = create_test_app();
    app.active_pane = Pane::CronWorkflowsList;
    app.cron_dialog_active = true;
    app.cron_inputs = vec![
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
    ];
    app.cron_inputs[0].insert_str("new-cron");
    app.cron_inputs[2].insert_str("* * * * *");
    app.cron_inputs[3].insert_str("test");

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_webhook_dialog() {
    let mut app = create_test_app();
    app.active_pane = Pane::WebhooksList;
    app.webhook_dialog_active = true;
    app.webhook_inputs = vec![
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
    ];
    app.webhook_inputs[0].insert_str("new-hook");
    app.webhook_inputs[1].insert_str("Desc");

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_approval_dialog() {
    let mut app = create_test_app();
    app.active_pane = Pane::RunDetail;
    app.approval_dialog_active = true;
    app.approval_inputs = TextArea::default();
    app.approval_inputs.insert_str("{\"approve\": true}");

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_runs_tab_empty() {
    let mut app = create_test_app();
    app.active_pane = Pane::RunsList;

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_runs_tab_populated() {
    let mut app = create_test_app();
    app.active_pane = Pane::RunsList;

    let created_at = Utc.timestamp_opt(1609459200, 0).unwrap();

    app.runs = vec![WorkflowRunDetail {
        id: RunId::new(Uuid::nil()),
        workflow_name: "test-workflow".to_string(),
        initiating_user: "jacrisp".to_string(),
        status: RunStatus::Succeeded,
        created_at,
        finished_at: Some(created_at),
    }];
    app.runs_state.select(Some(0));

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_run_detail_populated() {
    let mut app = create_test_app();
    app.active_pane = Pane::RunDetail;

    let created_at = Utc.timestamp_opt(1609459200, 0).unwrap();

    let detail = crate::app::WorkflowRunFullDetail {
        detail: WorkflowRunDetail {
            id: RunId::new(Uuid::nil()),
            workflow_name: "test-workflow".to_string(),
            initiating_user: "jacrisp".to_string(),
            status: RunStatus::Running,
            created_at,
            finished_at: None,
        },
        steps: vec![crate::app::StepDetail {
            instance: serde_json::json!({"step_name": "build", "status": "running"}),
            outputs: vec![],
            history: vec![],
            logs: vec![
                "Compiling source...".to_string(),
                "Running tests...".to_string(),
            ],
        }],
        artifacts: vec![],
        test_summaries: vec![],
        test_cases: vec![],
    };
    app.selected_run = Some(detail);
    app.runs_state.select(Some(0));
    app.run_logs = vec![
        "Compiling source...".to_string(),
        "Running tests...".to_string(),
    ];

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_filter_dialog() {
    let mut app = create_test_app();
    app.filter_dialog_active = true;
    app.filter_inputs = vec![
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
    ];
    app.filter_inputs[0].insert_str("owner");

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_schedule_git_dialog() {
    let mut app = create_test_app();
    app.schedule_git_dialog_active = true;
    app.schedule_git_inputs = vec![
        TextArea::default(),
        TextArea::default(),
        TextArea::default(),
    ];
    app.schedule_git_inputs[0].insert_str("https://github.com/test");

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_file_browser() {
    let mut app = create_test_app();
    app.file_browser_active = true;

    // Use a stable fixture directory to ensure consistent snapshots
    let dir_path = std::path::PathBuf::from("../../tests/fixtures/file_browser");
    app.file_explorer = tui_file_explorer::FileExplorer::new(dir_path, vec![]);

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_test_results_pane() {
    let mut app = create_test_app();
    app.active_pane = Pane::TestResults;

    let created_at = Utc.timestamp_opt(1609459200, 0).unwrap();

    let detail = crate::app::WorkflowRunFullDetail {
        detail: WorkflowRunDetail {
            id: RunId::new(Uuid::nil()),
            workflow_name: "test-workflow".to_string(),
            initiating_user: "jacrisp".to_string(),
            status: RunStatus::Succeeded,
            created_at,
            finished_at: None,
        },
        steps: vec![],
        artifacts: vec![],
        test_summaries: vec![TestSummary {
            id: TestReportId::new(Uuid::nil()),
            run_id: RunId::new(Uuid::nil()),
            step_instance_id: StepInstanceId::new(Uuid::nil()),
            report_name: "test_step".to_string(),
            total_tests: 10,
            passed: 9,
            failed: 1,
            skipped: 0,
            errors: 0,
            duration_ms: 1500,
            created_at,
        }],
        test_cases: vec![
            TestCase {
                id: TestReportId::new(Uuid::nil()),
                run_id: RunId::new(Uuid::nil()),
                step_instance_id: StepInstanceId::new(Uuid::nil()),
                report_name: "test_step".to_string(),
                test_suite: Some("MySuite".to_string()),
                test_case: "test_success".to_string(),
                status: TestCaseStatus::Passed,
                duration_ms: Some(100),
                message: None,
                created_at,
            },
            TestCase {
                id: TestReportId::new(Uuid::nil()),
                run_id: RunId::new(Uuid::nil()),
                step_instance_id: StepInstanceId::new(Uuid::nil()),
                report_name: "test_step".to_string(),
                test_suite: Some("MySuite".to_string()),
                test_case: "test_failure".to_string(),
                status: TestCaseStatus::Failed,
                duration_ms: Some(100),
                message: Some("assertion failed".to_string()),
                created_at,
            },
        ],
    };
    app.runs = vec![detail.detail.clone()];
    app.selected_run = Some(detail);
    app.runs_state.select(Some(0));

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_run_detail_with_artifacts() {
    let mut app = create_test_app();
    app.active_pane = Pane::RunDetail;

    let created_at = Utc.timestamp_opt(1609459200, 0).unwrap();
    let step_instance_id = StepInstanceId::new_v4();

    let detail = crate::app::WorkflowRunFullDetail {
        detail: WorkflowRunDetail {
            id: RunId::new(Uuid::nil()),
            workflow_name: "test-workflow".to_string(),
            initiating_user: "jacrisp".to_string(),
            status: RunStatus::Succeeded,
            created_at,
            finished_at: None,
        },
        steps: vec![crate::app::StepDetail {
            instance: serde_json::json!({
                "id": step_instance_id.to_string(),
                "step_name": "build",
                "status": "succeeded"
            }),
            outputs: vec![],
            history: vec![],
            logs: vec![],
        }],
        artifacts: vec![ArtifactRegistry {
            id: stormchaser_model::ArtifactId::new(Uuid::nil()),
            run_id: RunId::new(Uuid::nil()),
            step_instance_id,
            artifact_name: "binary".to_string(),
            connection_id: stormchaser_model::ConnectionId::new(Uuid::nil()),
            remote_path: "path/to/bin".to_string(),
            metadata: serde_json::json!({"size": 1024}),
            created_at,
        }],
        test_summaries: vec![],
        test_cases: vec![],
    };
    app.runs = vec![detail.detail.clone()];
    app.selected_run = Some(detail);
    app.runs_state.select(Some(0));
    app.overview_scroll = 4;

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}
