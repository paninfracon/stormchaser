use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

pub mod dialogs;
pub mod runs;

impl<'a> App<'a> {
    pub async fn handle_default_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('q') {
            return true;
        }

        if self.handle_pane_navigation_keys(key) {
            return false;
        }
        if self.handle_scroll_keys(key) {
            return false;
        }
        self.handle_action_keys(key).await;
        false
    }

    fn handle_pane_navigation_keys(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                match self.active_pane {
                    Pane::RunsList => self.next_run(),
                    Pane::StorageBackendsList => self.next_storage_backend(),
                    Pane::WebhooksList => self.next_webhook(),
                    Pane::EventRulesList => self.next_event_rule(),
                    Pane::CronWorkflowsList => self.next_cron_workflow(),
                    _ => self.next_step(),
                }
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                match self.active_pane {
                    Pane::RunsList => self.previous_run(),
                    Pane::StorageBackendsList => self.previous_storage_backend(),
                    Pane::WebhooksList => self.previous_webhook(),
                    Pane::EventRulesList => self.previous_event_rule(),
                    Pane::CronWorkflowsList => self.previous_cron_workflow(),
                    _ => self.previous_step(),
                }
                true
            }
            KeyCode::Char('1') => {
                self.active_pane = Pane::RunsList;
                true
            }
            KeyCode::Char('2') => {
                self.active_pane = Pane::StorageBackendsList;
                true
            }
            KeyCode::Char('3') => {
                self.active_pane = Pane::WebhooksList;
                true
            }
            KeyCode::Char('4') => {
                self.active_pane = Pane::EventRulesList;
                true
            }
            KeyCode::Char('5') => {
                self.active_pane = Pane::CronWorkflowsList;
                true
            }
            KeyCode::Tab => {
                self.active_pane = match self.active_pane {
                    Pane::RunsList => Pane::RunDetail,
                    Pane::RunDetail => Pane::TestResults,
                    Pane::TestResults => Pane::StorageBackendsList,
                    Pane::StorageBackendsList => Pane::StorageBackendDetail,
                    Pane::StorageBackendDetail => Pane::WebhooksList,
                    Pane::WebhooksList => Pane::WebhookDetail,
                    Pane::WebhookDetail => Pane::EventRulesList,
                    Pane::EventRulesList => Pane::EventRuleDetail,
                    Pane::EventRuleDetail => Pane::CronWorkflowsList,
                    Pane::CronWorkflowsList => Pane::CronWorkflowDetail,
                    Pane::CronWorkflowDetail => Pane::RunsList,
                };
                true
            }
            KeyCode::Char('t') => {
                if self.active_pane == Pane::TestResults {
                    self.active_pane = Pane::RunDetail;
                } else if self.active_pane == Pane::RunDetail || self.active_pane == Pane::RunsList
                {
                    self.active_pane = Pane::TestResults;
                }
                true
            }
            KeyCode::Char('h') | KeyCode::Left
                if !key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::SHIFT) =>
            {
                self.active_pane = match self.active_pane {
                    Pane::RunDetail | Pane::TestResults => Pane::RunsList,
                    Pane::StorageBackendDetail => Pane::StorageBackendsList,
                    Pane::WebhookDetail => Pane::WebhooksList,
                    Pane::EventRuleDetail => Pane::EventRulesList,
                    Pane::CronWorkflowDetail => Pane::CronWorkflowsList,
                    other => other,
                };
                true
            }
            KeyCode::Char('l') | KeyCode::Right
                if !key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::SHIFT) =>
            {
                self.active_pane = match self.active_pane {
                    Pane::RunsList => Pane::RunDetail,
                    Pane::StorageBackendsList => Pane::StorageBackendDetail,
                    Pane::WebhooksList => Pane::WebhookDetail,
                    Pane::EventRulesList => Pane::EventRuleDetail,
                    Pane::CronWorkflowsList => Pane::CronWorkflowDetail,
                    other => other,
                };
                true
            }
            _ => false,
        }
    }

    fn handle_scroll_keys(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::SHIFT) =>
            {
                self.scroll_logs_left();
                true
            }
            KeyCode::Char('l') | KeyCode::Right
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::SHIFT) =>
            {
                self.scroll_logs_right();
                true
            }
            KeyCode::Char('<') => {
                self.scroll_logs_left();
                true
            }
            KeyCode::Char('>') => {
                self.scroll_logs_right();
                true
            }
            KeyCode::Char('[') => {
                self.scroll_logs_up();
                true
            }
            KeyCode::Char(']') => {
                self.scroll_logs_down();
                true
            }
            KeyCode::Char('{') => {
                self.scroll_overview_up();
                true
            }
            KeyCode::Char('}') => {
                self.scroll_overview_down();
                true
            }
            KeyCode::PageUp => {
                for _ in 0..10 {
                    self.scroll_logs_up();
                }
                true
            }
            KeyCode::PageDown => {
                for _ in 0..10 {
                    self.scroll_logs_down();
                }
                true
            }
            KeyCode::Char('a') => {
                self.log_auto_scroll = !self.log_auto_scroll;
                true
            }
            _ => false,
        }
    }

    async fn handle_action_keys(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('A') if self.active_pane == Pane::RunDetail => {
                self.open_approval_dialog();
            }
            KeyCode::Char('R') if self.active_pane == Pane::RunDetail => {
                let _ = self.reject_selected_step().await;
            }
            KeyCode::Char('c') => match self.active_pane {
                Pane::StorageBackendsList | Pane::StorageBackendDetail => {
                    self.open_storage_backend_dialog(false)
                }
                Pane::WebhooksList | Pane::WebhookDetail => self.open_webhook_dialog(false),
                Pane::EventRulesList | Pane::EventRuleDetail => self.open_event_rule_dialog(false),
                Pane::CronWorkflowsList | Pane::CronWorkflowDetail => self.open_cron_dialog(false),
                Pane::RunsList | Pane::RunDetail => self.open_schedule_git_dialog(),
                _ => {}
            },
            KeyCode::Char('e') => match self.active_pane {
                Pane::StorageBackendsList | Pane::StorageBackendDetail => {
                    self.open_storage_backend_dialog(true)
                }
                Pane::WebhooksList | Pane::WebhookDetail => self.open_webhook_dialog(true),
                Pane::EventRulesList | Pane::EventRuleDetail => self.open_event_rule_dialog(true),
                Pane::CronWorkflowsList | Pane::CronWorkflowDetail => self.open_cron_dialog(true),
                _ => {}
            },
            KeyCode::Char('d') => match self.active_pane {
                Pane::StorageBackendsList | Pane::StorageBackendDetail => {
                    let _ = self.delete_selected_storage_backend().await;
                }
                Pane::WebhooksList | Pane::WebhookDetail => {
                    let _ = self.delete_selected_webhook().await;
                }
                Pane::EventRulesList | Pane::EventRuleDetail => {
                    let _ = self.delete_selected_event_rule().await;
                }
                Pane::CronWorkflowsList | Pane::CronWorkflowDetail => {
                    let _ = self.delete_selected_cron_workflow().await;
                }
                Pane::RunsList | Pane::RunDetail => {
                    self.open_delete_run_dialog();
                }
                _ => {}
            },
            KeyCode::Char('r') => {
                self.open_file_browser();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;
    use tokio::sync::mpsc;

    fn setup_app() -> App<'static> {
        let (tx, _rx) = mpsc::channel(1);
        App::new("http://test".to_string(), Some("token".to_string()), tx)
    }

    fn mock_run_detail(id: RunId, status: RunStatus) -> crate::app::WorkflowRunDetail {
        crate::app::WorkflowRunDetail {
            id,
            workflow_name: "test".to_string(),
            initiating_user: "user".to_string(),
            status,
            created_at: Utc::now(),
            finished_at: None,
        }
    }

    fn mock_step_detail(name: &str, status: &str) -> crate::app::StepDetail {
        crate::app::StepDetail {
            instance: serde_json::json!({
                "step_name": name,
                "status": status,
            }),
            outputs: vec![],
            history: vec![],
            logs: vec![],
        }
    }

    fn mock_full_detail(
        id: RunId,
        status: RunStatus,
        steps: Vec<crate::app::StepDetail>,
    ) -> WorkflowRunFullDetail {
        WorkflowRunFullDetail {
            detail: mock_run_detail(id, status),
            steps,
            artifacts: vec![],
            test_summaries: vec![],
            test_cases: vec![],
        }
    }

    #[test]
    fn test_handle_status_update_basic() {
        let mut app = setup_app();
        let run_id = RunId::new_v4();

        // 1. Initial state empty
        app.handle_status_update(run_id, "running".to_string());
        assert!(app.runs.is_empty());

        // 2. State has run
        app.runs.push(mock_run_detail(run_id, RunStatus::Queued));
        app.handle_status_update(run_id, "running".to_string());
        assert_eq!(app.runs[0].status, RunStatus::Running);

        // 3. State has selected run
        app.selected_run = Some(mock_full_detail(run_id, RunStatus::Queued, vec![]));
        app.handle_status_update(run_id, "succeeded".to_string());
        assert_eq!(
            app.selected_run.as_ref().unwrap().detail.status,
            RunStatus::Succeeded
        );
    }

    #[test]
    fn test_handle_workflow_update() {
        let mut app = setup_app();
        let run_id = RunId::new_v4();
        let detail = mock_run_detail(run_id, RunStatus::Queued);

        // 1. Insert new
        app.handle_workflow_update(detail.clone());
        assert_eq!(app.runs.len(), 1);

        // 2. Update existing
        let mut updated = detail.clone();
        updated.status = RunStatus::Running;
        app.handle_workflow_update(updated);
        assert_eq!(app.runs[0].status, RunStatus::Running);
    }

    #[tokio::test]
    async fn test_handle_step_update() {
        let mut app = setup_app();
        let run_id = RunId::new_v4();

        let step = mock_step_detail("test_step", "pending");
        let detail = mock_full_detail(run_id, RunStatus::Running, vec![step]);
        app.selected_run = Some(detail);

        app.handle_step_update(run_id, "test_step".to_string(), "running".to_string());

        let updated_step = &app.selected_run.as_ref().unwrap().steps[0];
        assert_eq!(
            updated_step
                .instance
                .get("status")
                .unwrap()
                .as_str()
                .unwrap(),
            "running"
        );
    }

    #[test]
    fn test_handle_step_logs_fetched_merge_with_live() {
        let mut app = setup_app();
        let run_id = RunId::new_v4();

        let mut step = mock_step_detail("test_step", "running");
        step.logs = vec!["live log 1".to_string(), "live log 2".to_string()];

        let detail = mock_full_detail(run_id, RunStatus::Running, vec![step]);
        app.selected_run = Some(detail);
        app.selected_step_index = 0;

        let fetched_logs = vec![
            "historic log 1".to_string(),
            "historic log 2".to_string(),
            "live log 1".to_string(), // simulate overlap
        ];

        app.handle_step_logs_fetched(run_id, 0, fetched_logs);

        let updated_step = &app.selected_run.as_ref().unwrap().steps[0];
        assert_eq!(updated_step.logs.len(), 4);
        assert_eq!(updated_step.logs[0], "historic log 1");
        assert_eq!(updated_step.logs[1], "historic log 2");
        assert_eq!(updated_step.logs[2], "live log 1");
        assert_eq!(updated_step.logs[3], "live log 2");

        assert_eq!(app.run_logs.len(), 4);
        assert_eq!(app.run_logs[3], "live log 2");
    }

    #[test]
    fn test_handle_log_line() {
        let mut app = setup_app();
        let run_id = RunId::new_v4();

        let step = mock_step_detail("test_step", "running");
        let detail = mock_full_detail(run_id, RunStatus::Running, vec![step]);
        app.selected_run = Some(detail);
        app.selected_step_index = 0;

        // 1. Normal log
        app.handle_log_line(run_id, "[test_step] Hello World".to_string());
        assert_eq!(app.run_logs.len(), 1);
        assert_eq!(app.run_logs[0], "Hello World");

        // 2. Extra spaces
        app.handle_log_line(run_id, "[test_step]  Extra Space".to_string());
        assert_eq!(app.run_logs.len(), 2);
        assert_eq!(app.run_logs[1], " Extra Space");
    }

    #[tokio::test]
    async fn test_handle_filter_dialog_key() {
        let mut app = setup_app();
        app.filter_dialog_active = true;
        app.filter_inputs = vec![
            ratatui_textarea::TextArea::default(),
            ratatui_textarea::TextArea::default(),
            ratatui_textarea::TextArea::default(),
            ratatui_textarea::TextArea::default(),
            ratatui_textarea::TextArea::default(),
            ratatui_textarea::TextArea::default(),
        ];
        app.filter_focus = 0;

        // Test Character Input
        app.handle_filter_dialog_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.filter_inputs[0].lines()[0], "a");

        // Test Tab
        app.handle_filter_dialog_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .await;
        assert_eq!(app.filter_focus, 1);

        // Test BackTab
        app.handle_filter_dialog_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.filter_focus, 0);

        // Test Tab wrapping around (0 to 6)
        app.filter_focus = 6; // Status Focus
        app.handle_filter_dialog_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .await;
        assert_eq!(app.filter_focus, 0);

        // Test BackTab wrapping around (0 to 6)
        app.handle_filter_dialog_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.filter_focus, 6);

        // Test Left/Right on Status Focus
        app.filter_status_index = 0;
        app.handle_filter_dialog_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
            .await;
        assert_eq!(app.filter_status_index, 1);
        app.handle_filter_dialog_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
            .await;
        assert_eq!(app.filter_status_index, 0);
        // Wrap Left
        app.handle_filter_dialog_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
            .await;
        assert_eq!(
            app.filter_status_index,
            crate::app::FILTER_STATUS_OPTIONS.len() - 1
        );

        // Test Esc
        app.handle_filter_dialog_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .await;
        assert!(!app.filter_dialog_active);
    }

    #[tokio::test]
    async fn test_handle_schedule_git_dialog_key() {
        let mut app = setup_app();
        app.schedule_git_dialog_active = true;
        app.schedule_git_inputs = vec![
            ratatui_textarea::TextArea::default(),
            ratatui_textarea::TextArea::default(),
            ratatui_textarea::TextArea::default(),
        ];
        app.schedule_git_focus = 0;

        app.handle_schedule_git_dialog_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.schedule_git_inputs[0].lines()[0], "x");

        app.handle_schedule_git_dialog_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .await;
        assert_eq!(app.schedule_git_focus, 1);

        app.handle_schedule_git_dialog_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.schedule_git_focus, 0);

        app.handle_schedule_git_dialog_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .await;
        assert!(!app.schedule_git_dialog_active);
    }

    #[tokio::test]
    async fn test_handle_storage_backend_dialog_key() {
        let mut app = setup_app();
        app.storage_backend_dialog_active = true;
        app.storage_backend_inputs = vec![
            ratatui_textarea::TextArea::default(),
            ratatui_textarea::TextArea::default(),
            ratatui_textarea::TextArea::default(),
            ratatui_textarea::TextArea::default(),
        ];
        app.storage_backend_focus = 0;

        app.handle_storage_backend_dialog_key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        ))
        .await;
        assert_eq!(app.storage_backend_inputs[0].lines()[0], "y");

        app.handle_storage_backend_dialog_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .await;
        assert_eq!(app.storage_backend_focus, 1);

        app.handle_storage_backend_dialog_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.storage_backend_focus, 0);

        // Focus 4: Type Options
        app.storage_backend_focus = 4;
        app.storage_backend_type_index = 0;
        app.handle_storage_backend_dialog_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
            .await;
        assert_eq!(app.storage_backend_type_index, 1);
        app.handle_storage_backend_dialog_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
            .await;
        assert_eq!(app.storage_backend_type_index, 0);

        // Focus 5: Is Default
        app.storage_backend_focus = 5;
        app.storage_backend_is_default = false;
        app.handle_storage_backend_dialog_key(KeyEvent::new(
            KeyCode::Char(' '),
            KeyModifiers::NONE,
        ))
        .await;
        assert!(app.storage_backend_is_default);
        app.handle_storage_backend_dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .await;
        assert!(!app.storage_backend_is_default);

        app.handle_storage_backend_dialog_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .await;
        assert!(!app.storage_backend_dialog_active);
    }

    #[tokio::test]
    async fn test_handle_webhook_dialog_key() {
        let mut app = setup_app();
        app.webhook_dialog_active = true;
        app.webhook_inputs = vec![
            ratatui_textarea::TextArea::default(),
            ratatui_textarea::TextArea::default(),
            ratatui_textarea::TextArea::default(),
        ];
        app.webhook_focus = 0;

        app.handle_webhook_dialog_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.webhook_inputs[0].lines()[0], "z");

        app.handle_webhook_dialog_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .await;
        assert_eq!(app.webhook_focus, 1);

        app.handle_webhook_dialog_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.webhook_focus, 0);

        // Focus 3: Source Type
        app.webhook_focus = 3;
        app.webhook_source_type_index = 0;
        app.handle_webhook_dialog_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
            .await;
        assert_eq!(app.webhook_source_type_index, 1);
        app.handle_webhook_dialog_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
            .await;
        assert_eq!(app.webhook_source_type_index, 0);

        // Focus 4: Is Active
        app.webhook_focus = 4;
        app.webhook_is_active = false;
        app.handle_webhook_dialog_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
            .await;
        assert!(app.webhook_is_active);

        app.handle_webhook_dialog_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .await;
        assert!(!app.webhook_dialog_active);
    }

    #[tokio::test]
    async fn test_handle_event_rule_dialog_key() {
        let mut app = setup_app();
        app.event_rule_dialog_active = true;
        app.event_rule_inputs = vec![ratatui_textarea::TextArea::default()];
        app.event_rule_focus = 0;

        app.handle_event_rule_dialog_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.event_rule_inputs[0].lines()[0], "w");

        app.handle_event_rule_dialog_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .await;
        assert_eq!(app.event_rule_focus, 1);

        // Focus 1: Is Active (length of inputs)
        app.event_rule_is_active = false;
        app.handle_event_rule_dialog_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
            .await;
        assert!(app.event_rule_is_active);

        app.handle_event_rule_dialog_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .await;
        assert!(!app.event_rule_dialog_active);
    }

    #[tokio::test]
    async fn test_handle_cron_dialog_key() {
        let mut app = setup_app();
        app.cron_dialog_active = true;
        app.cron_inputs = vec![ratatui_textarea::TextArea::default()];
        app.cron_focus = 0;

        app.handle_cron_dialog_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.cron_inputs[0].lines()[0], "v");

        app.handle_cron_dialog_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .await;
        assert_eq!(app.cron_focus, 1);

        // Focus 1: Is Active
        app.cron_is_active = false;
        app.handle_cron_dialog_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
            .await;
        assert!(app.cron_is_active);

        app.handle_cron_dialog_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .await;
        assert!(!app.cron_dialog_active);
    }

    #[tokio::test]
    async fn test_handle_file_browser_key() {
        let mut app = setup_app();
        app.file_browser_active = true;

        app.handle_file_browser_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .await;
        assert!(!app.file_browser_active);
    }

    #[tokio::test]
    async fn test_handle_approval_dialog_key() {
        let mut app = setup_app();
        app.approval_dialog_active = true;

        app.handle_approval_dialog_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.approval_inputs.lines()[0], "b");

        app.handle_approval_dialog_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .await;
        assert!(!app.approval_dialog_active);
    }

    #[tokio::test]
    async fn test_handle_default_key_scroll_logs() {
        let mut app = setup_app();

        // Right / Shift+Right
        app.handle_default_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.log_scroll_x, 1);

        app.handle_default_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.log_scroll_x, 2);

        // Left / Shift+Left
        app.handle_default_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.log_scroll_x, 1);

        app.handle_default_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.log_scroll_x, 0);

        app.handle_default_key(KeyEvent::new(KeyCode::Char('<'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.log_scroll_x, 0); // shouldn't underflow

        app.handle_default_key(KeyEvent::new(KeyCode::Char('>'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.log_scroll_x, 1);
    }
}
