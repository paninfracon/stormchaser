use crate::app::App;
use crate::app::Pane;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

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
                    Pane::ConnectionsList => self.next_connection(),
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
                    Pane::ConnectionsList => self.previous_connection(),
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
                self.active_pane = Pane::ConnectionsList;
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
                    Pane::TestResults => Pane::ConnectionsList,
                    Pane::ConnectionsList => Pane::ConnectionDetail,
                    Pane::ConnectionDetail => Pane::WebhooksList,
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
            KeyCode::Char('h') | KeyCode::Left if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.active_pane = match self.active_pane {
                    Pane::RunDetail | Pane::TestResults => Pane::RunsList,
                    Pane::ConnectionDetail => Pane::ConnectionsList,
                    Pane::WebhookDetail => Pane::WebhooksList,
                    Pane::EventRuleDetail => Pane::EventRulesList,
                    Pane::CronWorkflowDetail => Pane::CronWorkflowsList,
                    other => other,
                };
                true
            }
            KeyCode::Char('l') | KeyCode::Right if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.active_pane = match self.active_pane {
                    Pane::RunsList => Pane::RunDetail,
                    Pane::ConnectionsList => Pane::ConnectionDetail,
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
            KeyCode::Char('h') | KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll_logs_left();
                true
            }
            KeyCode::Char('l') | KeyCode::Right if key.modifiers.contains(KeyModifiers::SHIFT) => {
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
                Pane::ConnectionsList | Pane::ConnectionDetail => {
                    self.open_connection_dialog(false)
                }
                Pane::WebhooksList | Pane::WebhookDetail => self.open_webhook_dialog(false),
                Pane::EventRulesList | Pane::EventRuleDetail => self.open_event_rule_dialog(false),
                Pane::CronWorkflowsList | Pane::CronWorkflowDetail => self.open_cron_dialog(false),
                Pane::RunsList | Pane::RunDetail => self.open_schedule_git_dialog(),
                _ => {}
            },
            KeyCode::Char('e') => match self.active_pane {
                Pane::ConnectionsList | Pane::ConnectionDetail => self.open_connection_dialog(true),
                Pane::WebhooksList | Pane::WebhookDetail => self.open_webhook_dialog(true),
                Pane::EventRulesList | Pane::EventRuleDetail => self.open_event_rule_dialog(true),
                Pane::CronWorkflowsList | Pane::CronWorkflowDetail => self.open_cron_dialog(true),
                _ => {}
            },
            KeyCode::Char('d') => match self.active_pane {
                Pane::ConnectionsList | Pane::ConnectionDetail => {
                    let _ = self.delete_selected_connection().await;
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
    use crate::app::handlers::test_utils::setup_app;
    use ratatui::crossterm::event::KeyModifiers;
    use stormchaser_model::workflow::RunStatus;
    use stormchaser_model::RunId;

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

    #[tokio::test]
    async fn test_handle_action_keys() {
        let mut app = setup_app();

        // r opens file browser
        app.active_pane = Pane::RunsList;
        app.handle_action_keys(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
            .await;
        assert!(app.file_browser_active);
        app.file_browser_active = false;

        // c on RunsList opens schedule_git_dialog
        app.handle_action_keys(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
            .await;
        assert!(app.schedule_git_dialog_active);
        app.schedule_git_dialog_active = false;

        // d on RunsList opens delete_run_dialog_active if a run is selected
        let run_id = RunId::new_v4();
        let run = crate::app::handlers::test_utils::mock_run_detail(run_id, RunStatus::Succeeded);
        app.runs.push(run);
        app.runs_state.select(Some(0));
        app.handle_action_keys(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .await;
        assert!(app.delete_run_dialog_active);
        app.delete_run_dialog_active = false;

        // e on RunsList does nothing
        app.handle_action_keys(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .await;
        assert!(!app.schedule_git_dialog_active);
        assert!(!app.file_browser_active);
        assert!(!app.delete_run_dialog_active);

        // c on ConnectionsList opens connection_dialog in create mode
        app.active_pane = Pane::ConnectionsList;
        app.handle_action_keys(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
            .await;
        assert!(app.connection_dialog_active);
        assert_eq!(app.connection_edit_id, None);
        app.connection_dialog_active = false;

        // e on ConnectionsList opens connection_dialog in edit mode
        app.handle_action_keys(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .await;
        assert!(app.connection_dialog_active);
        app.connection_dialog_active = false;

        // d on ConnectionsList deletes selected (just check no crash since we can't easily mock the API call in this unit test without server mock)
        // Note: the delete method requires the selected item to exist to make API calls, so if none is selected, it should safely do nothing.
        app.handle_action_keys(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .await;
    }
}
