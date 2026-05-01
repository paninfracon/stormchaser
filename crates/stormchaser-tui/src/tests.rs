use crate::app::{App, AppState, Pane};
use crate::ui::ui;
use chrono::{TimeZone, Utc};
use insta::assert_debug_snapshot;
use ratatui::{backend::TestBackend, Terminal};
use stormchaser_model::{
    event_rules::WebhookConfig,
    storage::{BackendType, StorageBackend},
};

fn create_test_app<'a>() -> App<'a> {
    let (tx, _) = tokio::sync::mpsc::channel(100);
    let mut app = App::new(
        "http://localhost:3000".to_string(),
        Some("token".to_string()),
        tx,
    );
    app.state = AppState::LoggedIn;
    app
}

#[test]
fn render_storage_backends_tab() {
    let mut app = create_test_app();
    app.active_pane = Pane::StorageBackendsList;

    let created_at = Utc.timestamp_opt(1609459200, 0).unwrap();
    let updated_at = Utc.timestamp_opt(1609459200, 0).unwrap();

    app.storage_backends = vec![StorageBackend {
        id: uuid::Uuid::nil(),
        name: "test-sfs".to_string(),
        description: Some("Test SFS".to_string()),
        backend_type: BackendType::S3,
        is_default_sfs: true,
        config: serde_json::json!({"path": "/tmp/sfs"}),
        aws_assume_role_arn: None,
        ca_cert: None,
        client_cert: None,
        client_key: None,
        created_at,
        updated_at,
    }];
    app.selected_storage_backend = app.storage_backends.first().cloned();

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui(f, &mut app)).unwrap();
    assert_debug_snapshot!(terminal.backend());
}

#[test]
fn render_storage_backend_dialog() {
    let mut app = create_test_app();
    app.active_pane = Pane::StorageBackendsList;
    app.storage_backend_dialog_active = true;
    app.storage_backend_inputs = vec![
        ratatui_textarea::TextArea::default(),
        ratatui_textarea::TextArea::default(),
        ratatui_textarea::TextArea::default(),
        ratatui_textarea::TextArea::default(),
    ];
    app.storage_backend_inputs[0].insert_str("new-sfs");
    app.storage_backend_inputs[1].insert_str("Desc");
    app.storage_backend_inputs[2].insert_str("{}");

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
        id: uuid::Uuid::nil(),
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
fn render_webhook_dialog() {
    let mut app = create_test_app();
    app.active_pane = Pane::WebhooksList;
    app.webhook_dialog_active = true;
    app.webhook_inputs = vec![
        ratatui_textarea::TextArea::default(),
        ratatui_textarea::TextArea::default(),
        ratatui_textarea::TextArea::default(),
    ];
    app.webhook_inputs[0].insert_str("new-hook");
    app.webhook_inputs[1].insert_str("Desc");

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

    app.runs = vec![crate::app::WorkflowRunDetail {
        id: uuid::Uuid::nil(),
        workflow_name: "test-workflow".to_string(),
        initiating_user: "jacrisp".to_string(),
        status: stormchaser_model::workflow::RunStatus::Succeeded,
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
        detail: crate::app::WorkflowRunDetail {
            id: uuid::Uuid::nil(),
            workflow_name: "test-workflow".to_string(),
            initiating_user: "jacrisp".to_string(),
            status: stormchaser_model::workflow::RunStatus::Running,
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
        ratatui_textarea::TextArea::default(),
        ratatui_textarea::TextArea::default(),
        ratatui_textarea::TextArea::default(),
        ratatui_textarea::TextArea::default(),
        ratatui_textarea::TextArea::default(),
        ratatui_textarea::TextArea::default(),
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
        ratatui_textarea::TextArea::default(),
        ratatui_textarea::TextArea::default(),
        ratatui_textarea::TextArea::default(),
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
