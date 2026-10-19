use super::*;
use stormchaser_model::api::CreateCronWorkflowRequest;

impl<'a> App<'a> {
    /// Fetches the latest list of cron workflows from the API.
    pub async fn refresh_cron_workflows(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        match self.api().list_cron_workflows().await {
            Ok(cron_workflows) => {
                self.cron_workflows = cron_workflows;
                if !self.cron_workflows.is_empty() {
                    if self.cron_workflows_state.selected().is_none() {
                        self.cron_workflows_state.select(Some(0));
                    }
                    if self.selected_cron_workflow.is_none() {
                        if let Some(i) = self.cron_workflows_state.selected() {
                            self.selected_cron_workflow = Some(self.cron_workflows[i].clone());
                        }
                    }
                } else {
                    self.cron_workflows_state.select(None);
                    self.selected_cron_workflow = None;
                }
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("Failed to fetch cron workflows: {}", refused(e)?));
            }
        }
        Ok(())
    }

    /// Submits the cron workflow form, creating a schedule. The API cannot change a
    /// schedule.
    pub async fn submit_cron_workflow_form(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let field = |i: usize| self.cron_inputs[i].lines().join("\n");
        let name = field(0);
        let description = field(1);
        let cronspec = field(2);
        let workflow_name = field(3);
        let connection = field(4);
        let workflow_path = field(5);
        let git_ref = field(6);
        let inputs_str = field(7);

        if name.trim().is_empty()
            || cronspec.trim().is_empty()
            || workflow_name.trim().is_empty()
            || connection.trim().is_empty()
            || workflow_path.trim().is_empty()
            || git_ref.trim().is_empty()
        {
            self.error = Some("Required fields are missing.".to_string());
            return Ok(());
        }

        let inputs: Value = if inputs_str.trim().is_empty() {
            serde_json::json!({})
        } else {
            match serde_json::from_str(&inputs_str) {
                Ok(v) => v,
                Err(_) => {
                    self.error = Some("Inputs must be valid JSON.".to_string());
                    return Ok(());
                }
            }
        };

        let request = CreateCronWorkflowRequest {
            name,
            description: if description.trim().is_empty() {
                None
            } else {
                Some(description)
            },
            cronspec,
            workflow_name,
            connection,
            workflow_path,
            git_ref,
            inputs,
        };

        match self.api().create_cron_workflow(&request).await {
            Ok(_) => {
                self.cron_dialog_active = false;
                self.refresh_cron_workflows().await?;
            }
            Err(e) => {
                self.error = Some(format!("Failed to save cron workflow: {}", refused(e)?));
            }
        }

        Ok(())
    }

    /// Deletes the currently selected cron workflow.
    pub async fn delete_selected_cron_workflow(&mut self) -> Result<()> {
        if let Some(cron) = &self.selected_cron_workflow {
            if self.token.is_some() {
                match self.api().delete_cron_workflow(cron.id).await {
                    Ok(()) => self.refresh_cron_workflows().await?,
                    Err(e) => {
                        self.error =
                            Some(format!("Failed to delete cron workflow: {}", refused(e)?));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ratatui_textarea::TextArea;
    use stormchaser_model::cron::CronWorkflow;
    use stormchaser_model::CronWorkflowId;
    use tokio::sync::mpsc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_cron_workflow(id: CronWorkflowId) -> CronWorkflow {
        CronWorkflow {
            id,
            name: "test-cron".to_string(),
            description: None,
            cronspec: "* * * * *".to_string(),
            workflow_name: "test".to_string(),
            repo_url: "https://github.com/org/repo".to_string(),
            workflow_path: "workflow.storm".to_string(),
            git_ref: "main".to_string(),
            inputs: serde_json::json!({}),
            is_active: true,
            secret_token: "secret".to_string(),
            external_job_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_refresh_cron_workflows_uses_correct_path() {
        let server = MockServer::start().await;
        let id = CronWorkflowId::new_v4();

        Mock::given(method("GET"))
            .and(path("/api/v1/cron-workflows"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![make_cron_workflow(id)]))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            server.uri(),
            "http://localhost:3001".to_string(),
            Some("token".to_string()),
            tx,
        );

        let result = app.refresh_cron_workflows().await;
        result.unwrap();
        assert_eq!(app.cron_workflows.len(), 1);
        assert_eq!(app.cron_workflows[0].id, id);
        assert!(app.error.is_none());
    }

    #[tokio::test]
    async fn test_delete_cron_workflow_uses_correct_path() {
        let server = MockServer::start().await;
        let id = CronWorkflowId::new_v4();

        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/cron-workflows/{}", id)))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/v1/cron-workflows"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<CronWorkflow>::new()))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            server.uri(),
            "http://localhost:3001".to_string(),
            Some("token".to_string()),
            tx,
        );
        app.selected_cron_workflow = Some(make_cron_workflow(id));

        let result = app.delete_selected_cron_workflow().await;
        result.unwrap();
        assert!(app.error.is_none());
    }

    #[tokio::test]
    async fn test_submit_cron_workflow_form_creates_a_schedule() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/cron-workflows"))
            .and(wiremock::matchers::body_partial_json(serde_json::json!({
                "cronspec": "0 * * * *",
                "connection": "repo",
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": CronWorkflowId::new_v4(),
                "secret_token": "secret",
                "external_job_id": null,
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/v1/cron-workflows"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<CronWorkflow>::new()))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            server.uri(),
            "http://localhost:3001".to_string(),
            Some("token".to_string()),
            tx,
        );
        app.open_cron_dialog();
        let values = [
            "nightly",
            "",
            "0 * * * *",
            "wf",
            "repo",
            "wf.storm",
            "main",
            "{}",
        ];
        app.cron_inputs = values
            .iter()
            .map(|v| TextArea::from(vec![v.to_string()]))
            .collect();

        app.submit_cron_workflow_form().await.unwrap();
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(!app.cron_dialog_active);
    }
}
