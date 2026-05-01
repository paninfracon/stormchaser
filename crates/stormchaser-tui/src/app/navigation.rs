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

    /// Selects the next storage backend in the list.
    pub fn next_storage_backend(&mut self) {
        if self.storage_backends.is_empty() {
            return;
        }
        let i = match self.storage_backends_state.selected() {
            Some(i) => {
                if i >= self.storage_backends.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.storage_backends_state.select(Some(i));
        self.selected_storage_backend = Some(self.storage_backends[i].clone());
    }

    /// Selects the previous storage backend in the list.
    pub fn previous_storage_backend(&mut self) {
        if self.storage_backends.is_empty() {
            return;
        }
        let i = match self.storage_backends_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.storage_backends.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.storage_backends_state.select(Some(i));
        self.selected_storage_backend = Some(self.storage_backends[i].clone());
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
        let mut app = App::new("http://localhost".to_string(), None, tx);

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
        let mut app = App::new("http://localhost".to_string(), None, tx);

        app.next_run();
        assert_eq!(app.runs_state.selected(), None);

        app.previous_run();
        assert_eq!(app.runs_state.selected(), None);
    }

    #[tokio::test]
    async fn test_run_navigation() {
        let (tx, _rx) = mpsc::channel(100);
        let mut app = App::new("http://localhost".to_string(), None, tx);

        app.runs = vec![
            WorkflowRunDetail {
                id: Uuid::new_v4(),
                workflow_name: "1".to_string(),
                initiating_user: "u".to_string(),
                status: RunStatus::Succeeded,
                created_at: chrono::Utc::now(),
                finished_at: None,
            },
            WorkflowRunDetail {
                id: Uuid::new_v4(),
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
}
