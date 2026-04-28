use super::*;
use crate::AppEvent;

impl<'a> App<'a> {
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

    pub fn next_step(&mut self) {
        if let Some(run) = &self.selected_run {
            if !run.steps.is_empty() {
                self.selected_step_index = (self.selected_step_index + 1) % run.steps.len();
                self.refresh_step_logs(true);
            }
        }
    }

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

    pub fn scroll_logs_up(&mut self) {
        self.log_auto_scroll = false;
        if self.log_scroll > 0 {
            self.log_scroll -= 1;
        }
    }

    pub fn scroll_logs_down(&mut self) {
        self.log_scroll += 1;
    }

    pub fn scroll_overview_up(&mut self) {
        if self.overview_scroll > 0 {
            self.overview_scroll -= 1;
        }
    }

    pub fn scroll_overview_down(&mut self) {
        self.overview_scroll += 1;
    }

    pub fn scroll_logs_to_top(&mut self) {
        self.log_auto_scroll = false;
        self.log_scroll = 0;
    }
}
