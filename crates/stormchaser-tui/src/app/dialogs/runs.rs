use crate::app::FILTER_STATUS_OPTIONS;
use anyhow::Result;
use ratatui_textarea::TextArea;

impl<'a> crate::app::App<'a> {
    /// Opens the filter dialog and initializes input fields with current values.
    pub fn open_filter_dialog(&mut self) {
        self.filter_dialog_active = true;
        self.filter_focus = 0;
        self.filter_inputs = vec![
            TextArea::from(vec![self.filter_owner.clone().unwrap_or_default()]),
            TextArea::from(vec![self.filter_name.clone().unwrap_or_default()]),
            TextArea::from(vec![self.filter_repo_url.clone().unwrap_or_default()]),
            TextArea::from(vec![self.filter_workflow_path.clone().unwrap_or_default()]),
            TextArea::from(vec![self.filter_created_after.clone().unwrap_or_default()]),
            TextArea::from(vec![self.filter_created_before.clone().unwrap_or_default()]),
        ];

        let current_status = self.filter_status.as_deref().unwrap_or("Any");
        self.filter_status_index = FILTER_STATUS_OPTIONS
            .iter()
            .position(|&s| s.eq_ignore_ascii_case(current_status))
            .unwrap_or(0);
    }

    /// Applies the values from the filter dialog inputs to the active filters and refreshes runs.
    pub async fn apply_filters(&mut self) -> Result<()> {
        if self.filter_inputs.len() == 6 {
            let o = self.filter_inputs[0].lines()[0].trim().to_string();
            self.filter_owner = if o.is_empty() { None } else { Some(o) };

            let n = self.filter_inputs[1].lines()[0].trim().to_string();
            self.filter_name = if n.is_empty() { None } else { Some(n) };

            let r = self.filter_inputs[2].lines()[0].trim().to_string();
            self.filter_repo_url = if r.is_empty() { None } else { Some(r) };

            let w = self.filter_inputs[3].lines()[0].trim().to_string();
            self.filter_workflow_path = if w.is_empty() { None } else { Some(w) };

            let ca = self.filter_inputs[4].lines()[0].trim().to_string();
            self.filter_created_after = if ca.is_empty() { None } else { Some(ca) };

            let cb = self.filter_inputs[5].lines()[0].trim().to_string();
            self.filter_created_before = if cb.is_empty() { None } else { Some(cb) };

            let s = FILTER_STATUS_OPTIONS[self.filter_status_index];
            self.filter_status = if s == "Any" {
                None
            } else {
                Some(s.to_lowercase())
            };
        }
        self.filter_dialog_active = false;
        self.refresh_runs().await
    }

    /// Opens the step approval dialog.
    pub fn open_approval_dialog(&mut self) {
        self.approval_dialog_active = true;
        self.approval_inputs = TextArea::from(vec!["{}".to_string()]);
    }

    /// Opens the delete run dialog.
    pub fn open_delete_run_dialog(&mut self) {
        if self.runs_state.selected().is_some() {
            self.delete_run_dialog_active = true;
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::app::App;

    fn setup_app() -> App<'static> {
        let (tx, _) = tokio::sync::mpsc::channel(1);
        App::new(
            "http://test".to_string(),
            "http://test".to_string(),
            Some("token".to_string()),
            tx,
        )
    }

    #[test]
    fn test_open_filter_dialog() {
        let mut app = setup_app();
        app.filter_owner = Some("owner".to_string());
        app.filter_status = Some("failed".to_string());

        app.open_filter_dialog();

        assert!(app.filter_dialog_active);
        assert_eq!(app.filter_focus, 0);
        assert_eq!(app.filter_inputs.len(), 6);
        assert_eq!(app.filter_inputs[0].lines()[0], "owner");
        assert_eq!(app.filter_status_index, 5); // Failed is index 5 in FILTER_STATUS_OPTIONS
    }
}
