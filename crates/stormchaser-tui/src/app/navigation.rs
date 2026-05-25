use super::*;
use crate::AppEvent;

impl<'a> App<'a> {
    /// Selects the next run in the list, wrapping to the start if at the end.
    pub fn next_run(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        let i = match self.runs_state.selected() {
            Some(i) => {
                if i >= self.runs.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.runs_state.select(Some(i));
        self.selected_step_index = 0;
        self.selected_run = None;

        // Trigger fetch of new details
        let id = self.runs[i].id;
        let tx = self.status_tx.clone();
        tokio::spawn(async move {
            let _ = tx
                .send(AppEvent::StatusUpdate(id, "force_refresh".to_string()))
                .await;
            let _ = tx.send(AppEvent::StartWatching(id)).await;
        });
    }

    /// Selects the previous run in the list, wrapping to the end if at the start.
    pub fn previous_run(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        let i = match self.runs_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.runs.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.runs_state.select(Some(i));
        self.selected_step_index = 0;
        self.selected_run = None;

        // Trigger fetch of new details
        let id = self.runs[i].id;
        let tx = self.status_tx.clone();
        tokio::spawn(async move {
            let _ = tx
                .send(AppEvent::StatusUpdate(id, "force_refresh".to_string()))
                .await;
            let _ = tx.send(AppEvent::StartWatching(id)).await;
        });
    }

    pub fn next_pending_approval(&mut self) {
        if self.pending_approvals.is_empty() {
            return;
        }
        let i = match self.pending_approvals_state.selected() {
            Some(i) => {
                if i >= self.pending_approvals.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.pending_approvals_state.select(Some(i));
    }

    pub fn previous_pending_approval(&mut self) {
        if self.pending_approvals.is_empty() {
            return;
        }
        let i = match self.pending_approvals_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.pending_approvals.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.pending_approvals_state.select(Some(i));
    }

    /// Selects the next storage backend in the list.
    pub fn next_connection(&mut self) {
        if self.connections.is_empty() {
            return;
        }
        let i = match self.connections_state.selected() {
            Some(i) => {
                if i >= self.connections.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.connections_state.select(Some(i));
        self.selected_connection = Some(self.connections[i].clone());
    }

    /// Selects the previous storage backend in the list.
    pub fn previous_connection(&mut self) {
        if self.connections.is_empty() {
            return;
        }
        let i = match self.connections_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.connections.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.connections_state.select(Some(i));
        self.selected_connection = Some(self.connections[i].clone());
    }

    /// Selects the next webhook in the list.
    pub fn next_webhook(&mut self) {
        if self.webhooks.is_empty() {
            return;
        }
        let i = match self.webhooks_state.selected() {
            Some(i) => {
                if i >= self.webhooks.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.webhooks_state.select(Some(i));
        self.selected_webhook = Some(self.webhooks[i].clone());
    }

    /// Selects the previous webhook in the list.
    pub fn previous_webhook(&mut self) {
        if self.webhooks.is_empty() {
            return;
        }
        let i = match self.webhooks_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.webhooks.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.webhooks_state.select(Some(i));
        self.selected_webhook = Some(self.webhooks[i].clone());
    }

    /// Selects the next event rule in the list.
    pub fn next_event_rule(&mut self) {
        if self.event_rules.is_empty() {
            return;
        }
        let i = match self.event_rules_state.selected() {
            Some(i) => {
                if i >= self.event_rules.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.event_rules_state.select(Some(i));
        self.selected_event_rule = Some(self.event_rules[i].clone());
    }

    /// Selects the previous event rule in the list.
    pub fn previous_event_rule(&mut self) {
        if self.event_rules.is_empty() {
            return;
        }
        let i = match self.event_rules_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.event_rules.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.event_rules_state.select(Some(i));
        self.selected_event_rule = Some(self.event_rules[i].clone());
    }

    /// Selects the next cron workflow in the list.
    pub fn next_cron_workflow(&mut self) {
        if self.cron_workflows.is_empty() {
            return;
        }
        let i = match self.cron_workflows_state.selected() {
            Some(i) => {
                if i >= self.cron_workflows.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.cron_workflows_state.select(Some(i));
        self.selected_cron_workflow = Some(self.cron_workflows[i].clone());
    }

    /// Selects the previous cron workflow in the list.
    pub fn previous_cron_workflow(&mut self) {
        if self.cron_workflows.is_empty() {
            return;
        }
        let i = match self.cron_workflows_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.cron_workflows.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.cron_workflows_state.select(Some(i));
        self.selected_cron_workflow = Some(self.cron_workflows[i].clone());
    }

    /// Selects the next step within the currently selected workflow run.
    pub fn next_step(&mut self) {
        if let Some(run) = &self.selected_run {
            if !run.steps.is_empty() {
                self.selected_step_index = (self.selected_step_index + 1) % run.steps.len();
                self.refresh_step_logs(true);
            }
        }
    }

    /// Selects the previous step within the currently selected workflow run.
    pub fn previous_step(&mut self) {
        if let Some(run) = &self.selected_run {
            if !run.steps.is_empty() {
                if self.selected_step_index == 0 {
                    self.selected_step_index = run.steps.len() - 1;
                } else {
                    self.selected_step_index -= 1;
                }
                self.refresh_step_logs(true);
            }
        }
    }

    /// Scrolls the log view upwards, pausing automatic scrolling.
    pub fn scroll_logs_up(&mut self) {
        self.log_auto_scroll = false;
        if self.log_scroll > 0 {
            self.log_scroll -= 1;
        }
    }

    /// Scrolls the log view downwards.
    pub fn scroll_logs_down(&mut self) {
        self.log_scroll += 1;
    }

    /// Scrolls the log view to the left.
    pub fn scroll_logs_left(&mut self) {
        if self.log_scroll_x > 0 {
            self.log_scroll_x -= 1;
        }
    }

    /// Scrolls the log view to the right.
    pub fn scroll_logs_right(&mut self) {
        self.log_scroll_x += 1;
    }

    /// Scrolls the overview pane upwards.
    pub fn scroll_overview_up(&mut self) {
        if self.overview_scroll > 0 {
            self.overview_scroll -= 1;
        }
    }

    /// Scrolls the overview pane downwards.
    pub fn scroll_overview_down(&mut self) {
        self.overview_scroll += 1;
    }

    /// Quickly scrolls the log view to the very top, pausing auto-scroll.
    pub fn scroll_logs_to_top(&mut self) {
        self.log_auto_scroll = false;
        self.log_scroll = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_scroll_logic() {
        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            "http://localhost".to_string(),
            "http://localhost".to_string(),
            None,
            tx,
        );

        app.scroll_logs_down();
        assert_eq!(app.log_scroll, 1);

        app.scroll_logs_up();
        assert_eq!(app.log_scroll, 0);
        assert!(!app.log_auto_scroll);

        app.scroll_logs_right();
        assert_eq!(app.log_scroll_x, 1);

        app.scroll_logs_left();
        assert_eq!(app.log_scroll_x, 0);

        // Ensure we don't underflow
        app.scroll_logs_left();
        assert_eq!(app.log_scroll_x, 0);

        app.scroll_overview_down();
        assert_eq!(app.overview_scroll, 1);

        app.scroll_overview_up();
        assert_eq!(app.overview_scroll, 0);

        app.scroll_logs_down();
        app.scroll_logs_down();
        app.scroll_logs_to_top();
        assert_eq!(app.log_scroll, 0);
        assert!(!app.log_auto_scroll);
    }

    #[tokio::test]
    async fn test_run_navigation_empty() {
        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            "http://localhost".to_string(),
            "http://localhost".to_string(),
            None,
            tx,
        );

        app.next_run();
        assert_eq!(app.runs_state.selected(), None);

        app.previous_run();
        assert_eq!(app.runs_state.selected(), None);
    }

    #[tokio::test]
    async fn test_run_navigation() {
        let (tx, _rx) = mpsc::channel(100);
        let mut app = App::new(
            "http://localhost".to_string(),
            "http://localhost".to_string(),
            None,
            tx,
        );

        app.runs = vec![
            WorkflowRunDetail {
                id: RunId::new_v4(),
                workflow_name: "1".to_string(),
                initiating_user: "u".to_string(),
                status: RunStatus::Succeeded,
                created_at: chrono::Utc::now(),
                finished_at: None,
            },
            WorkflowRunDetail {
                id: RunId::new_v4(),
                workflow_name: "2".to_string(),
                initiating_user: "u".to_string(),
                status: RunStatus::Succeeded,
                created_at: chrono::Utc::now(),
                finished_at: None,
            },
        ];

        app.next_run();
        assert_eq!(app.runs_state.selected(), Some(0));

        app.next_run();
        assert_eq!(app.runs_state.selected(), Some(1));

        app.next_run();
        assert_eq!(app.runs_state.selected(), Some(0));

        app.previous_run();
        assert_eq!(app.runs_state.selected(), Some(1));

        app.previous_run();
        assert_eq!(app.runs_state.selected(), Some(0));
    }

    #[tokio::test]
    async fn test_pending_approval_navigation_empty() {
        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            "http://localhost".to_string(),
            "http://localhost".to_string(),
            None,
            tx,
        );

        app.next_pending_approval();
        assert_eq!(app.pending_approvals_state.selected(), None);

        app.previous_pending_approval();
        assert_eq!(app.pending_approvals_state.selected(), None);
    }

    #[tokio::test]
    async fn test_pending_approval_navigation() {
        let (tx, _rx) = mpsc::channel(100);
        let mut app = App::new(
            "http://localhost".to_string(),
            "http://localhost".to_string(),
            None,
            tx,
        );

        app.pending_approvals = vec![
            WorkflowRunDetail {
                id: RunId::new_v4(),
                workflow_name: "1".to_string(),
                initiating_user: "u".to_string(),
                status: RunStatus::Running,
                created_at: chrono::Utc::now(),
                finished_at: None,
            },
            WorkflowRunDetail {
                id: RunId::new_v4(),
                workflow_name: "2".to_string(),
                initiating_user: "u".to_string(),
                status: RunStatus::Running,
                created_at: chrono::Utc::now(),
                finished_at: None,
            },
        ];

        app.next_pending_approval();
        assert_eq!(app.pending_approvals_state.selected(), Some(0));

        app.next_pending_approval();
        assert_eq!(app.pending_approvals_state.selected(), Some(1));

        app.next_pending_approval();
        assert_eq!(app.pending_approvals_state.selected(), Some(0));

        app.previous_pending_approval();
        assert_eq!(app.pending_approvals_state.selected(), Some(1));

        app.previous_pending_approval();
        assert_eq!(app.pending_approvals_state.selected(), Some(0));
    }
}
