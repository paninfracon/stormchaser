use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

impl<'a> App<'a> {
    pub async fn handle_filter_dialog_key(&mut self, key: KeyEvent) {
        // Focus layout: 0..=5 are the 6 text inputs, 6 is the status selector.
        const FOCUS_COUNT: usize = 7;
        const STATUS_FOCUS: usize = 6;
        match key.code {
            KeyCode::Esc => {
                self.filter_dialog_active = false;
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
            {
                let _ = self.apply_filters().await;
            }
            KeyCode::BackTab => {
                self.filter_focus = (self.filter_focus + FOCUS_COUNT - 1) % FOCUS_COUNT;
            }
            KeyCode::Tab => {
                self.filter_focus = (self.filter_focus + 1) % FOCUS_COUNT;
            }
            KeyCode::Left if self.filter_focus == STATUS_FOCUS => {
                let opts_len = crate::app::FILTER_STATUS_OPTIONS.len();
                self.filter_status_index = (self.filter_status_index + opts_len - 1) % opts_len;
            }
            KeyCode::Right if self.filter_focus == STATUS_FOCUS => {
                let opts_len = crate::app::FILTER_STATUS_OPTIONS.len();
                self.filter_status_index = (self.filter_status_index + 1) % opts_len;
            }
            _ => {
                if self.filter_focus < STATUS_FOCUS {
                    self.filter_inputs[self.filter_focus].input(key);
                }
            }
        }
    }

    pub async fn handle_schedule_git_dialog_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.schedule_git_dialog_active = false;
            }
            KeyCode::Enter => {
                let _ = self.submit_schedule_git().await;
            }
            KeyCode::BackTab => {
                self.schedule_git_focus = (self.schedule_git_focus + 2) % 3;
            }
            KeyCode::Tab => {
                self.schedule_git_focus = (self.schedule_git_focus + 1) % 3;
            }
            _ => {
                self.schedule_git_inputs[self.schedule_git_focus].input(key);
            }
        }
    }

    pub async fn handle_storage_backend_dialog_key(&mut self, key: KeyEvent) {
        let focus_count = self.storage_backend_inputs.len() + 2; // +2 for type dropdown and is_default
        match key.code {
            KeyCode::Esc => {
                self.storage_backend_dialog_active = false;
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
            {
                let _ = self.submit_storage_backend_form().await;
            }
            KeyCode::BackTab => {
                self.storage_backend_focus =
                    (self.storage_backend_focus + focus_count - 1) % focus_count;
            }
            KeyCode::Tab => {
                self.storage_backend_focus = (self.storage_backend_focus + 1) % focus_count;
            }
            KeyCode::Left if self.storage_backend_focus == 4 => {
                let opts_len = crate::app::BACKEND_TYPE_OPTIONS.len();
                self.storage_backend_type_index =
                    (self.storage_backend_type_index + opts_len - 1) % opts_len;
            }
            KeyCode::Right if self.storage_backend_focus == 4 => {
                let opts_len = crate::app::BACKEND_TYPE_OPTIONS.len();
                self.storage_backend_type_index = (self.storage_backend_type_index + 1) % opts_len;
            }
            KeyCode::Char(' ') | KeyCode::Enter if self.storage_backend_focus == 5 => {
                self.storage_backend_is_default = !self.storage_backend_is_default;
            }
            _ => {
                if self.storage_backend_focus < 4 {
                    self.storage_backend_inputs[self.storage_backend_focus].input(key);
                }
            }
        }
    }

    pub async fn handle_webhook_dialog_key(&mut self, key: KeyEvent) {
        let focus_count = self.webhook_inputs.len() + 2; // name, desc, token, type, is_active
        match key.code {
            KeyCode::Esc => {
                self.webhook_dialog_active = false;
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
            {
                let _ = self.submit_webhook_form().await;
            }
            KeyCode::BackTab => {
                self.webhook_focus = (self.webhook_focus + focus_count - 1) % focus_count;
            }
            KeyCode::Tab => {
                self.webhook_focus = (self.webhook_focus + 1) % focus_count;
            }
            KeyCode::Left if self.webhook_focus == 3 => {
                let opts_len = crate::app::WEBHOOK_SOURCE_TYPE_OPTIONS.len();
                self.webhook_source_type_index =
                    (self.webhook_source_type_index + opts_len - 1) % opts_len;
            }
            KeyCode::Right if self.webhook_focus == 3 => {
                let opts_len = crate::app::WEBHOOK_SOURCE_TYPE_OPTIONS.len();
                self.webhook_source_type_index = (self.webhook_source_type_index + 1) % opts_len;
            }
            KeyCode::Char(' ') | KeyCode::Enter if self.webhook_focus == 4 => {
                self.webhook_is_active = !self.webhook_is_active;
            }
            _ => {
                if self.webhook_focus < 3 {
                    self.webhook_inputs[self.webhook_focus].input(key);
                }
            }
        }
    }

    pub async fn handle_event_rule_dialog_key(&mut self, key: KeyEvent) {
        let focus_count = self.event_rule_inputs.len() + 1; // inputs + is_active
        match key.code {
            KeyCode::Esc => {
                self.event_rule_dialog_active = false;
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
            {
                let _ = self.submit_event_rule_form().await;
            }
            KeyCode::BackTab => {
                self.event_rule_focus = (self.event_rule_focus + focus_count - 1) % focus_count;
            }
            KeyCode::Tab => {
                self.event_rule_focus = (self.event_rule_focus + 1) % focus_count;
            }
            KeyCode::Char(' ') | KeyCode::Enter
                if self.event_rule_focus == self.event_rule_inputs.len() =>
            {
                self.event_rule_is_active = !self.event_rule_is_active;
            }
            _ => {
                if self.event_rule_focus < self.event_rule_inputs.len() {
                    self.event_rule_inputs[self.event_rule_focus].input(key);
                }
            }
        }
    }

    pub async fn handle_cron_dialog_key(&mut self, key: KeyEvent) {
        let focus_count = self.cron_inputs.len() + 1; // inputs + is_active
        match key.code {
            KeyCode::Esc => {
                self.cron_dialog_active = false;
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
            {
                let _ = self.submit_cron_workflow_form().await;
            }
            KeyCode::BackTab => {
                self.cron_focus = (self.cron_focus + focus_count - 1) % focus_count;
            }
            KeyCode::Tab => {
                self.cron_focus = (self.cron_focus + 1) % focus_count;
            }
            KeyCode::Char(' ') | KeyCode::Enter if self.cron_focus == self.cron_inputs.len() => {
                self.cron_is_active = !self.cron_is_active;
            }
            _ => {
                if self.cron_focus < self.cron_inputs.len() {
                    self.cron_inputs[self.cron_focus].input(key);
                }
            }
        }
    }

    pub async fn handle_approval_dialog_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.approval_dialog_active = false;
            }
            KeyCode::Char('a') | KeyCode::Char('A')
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
            {
                let inputs_str = self.approval_inputs.lines().join("\n");
                let inputs_json =
                    serde_json::from_str(&inputs_str).unwrap_or(serde_json::json!({}));
                let _ = self.approve_selected_step(inputs_json).await;
            }
            _ => {
                self.approval_inputs.input(key);
            }
        }
    }

    pub async fn handle_direct_submit_form_key(&mut self, key: KeyEvent) {
        if let Some(ref mut form) = self.direct_submit_form {
            form.handle_input(key);
            match form.result() {
                ratatui_form::FormResult::Submitted => {
                    let _ = self.submit_file().await;
                }
                ratatui_form::FormResult::Cancelled => {
                    self.direct_submit_form = None;
                    self.direct_submit_dsl = None;
                }
                _ => {}
            }
        }
    }

    pub async fn handle_file_browser_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.file_browser_active = false;
            }
            KeyCode::Enter => {
                if self.file_explorer.current_entry().is_some_and(|e| e.is_dir) {
                    let _ = self.file_explorer.handle_key(key);
                } else {
                    let _ = self.submit_file().await;
                }
            }
            _ => {
                let _ = self.file_explorer.handle_key(key);
            }
        }
    }
}
