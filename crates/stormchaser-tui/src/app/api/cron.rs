use super::*;

impl<'a> App<'a> {
    /// Fetches the latest list of cron workflows from the API.
    pub async fn refresh_cron_workflows(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let res = self
            .api_request(reqwest::Method::GET, "/api/v1/cron-workflows", None)
            .await?;

        if res.status().is_success() {
            self.cron_workflows = res
                .json::<Vec<stormchaser_model::cron::CronWorkflow>>()
                .await?;
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
        } else {
            self.error = Some(format!("Failed to fetch cron workflows: {}", res.status()));
        }
        Ok(())
    }

    /// Submits the cron workflow form for creation or update.
    pub async fn submit_cron_workflow_form(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let name = self.cron_inputs[0].lines().join("\n");
        let description = self.cron_inputs[1].lines().join("\n");
        let cronspec = self.cron_inputs[2].lines().join("\n");
        let workflow_name = self.cron_inputs[3].lines().join("\n");
        let repo_url = self.cron_inputs[4].lines().join("\n");
        let workflow_path = self.cron_inputs[5].lines().join("\n");
        let git_ref = self.cron_inputs[6].lines().join("\n");
        let inputs_str = self.cron_inputs[7].lines().join("\n");

        if name.trim().is_empty()
            || cronspec.trim().is_empty()
            || workflow_name.trim().is_empty()
            || repo_url.trim().is_empty()
            || workflow_path.trim().is_empty()
            || git_ref.trim().is_empty()
        {
            self.error = Some("Required fields are missing.".to_string());
            return Ok(());
        }

        let inputs_json: serde_json::Value = if inputs_str.trim().is_empty() {
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

        let (method, path) = if let Some(id) = self.cron_edit_id {
            (
                reqwest::Method::PATCH,
                format!("/api/v1/cron-workflows/{}", id),
            )
        } else {
            (reqwest::Method::POST, "/api/v1/cron-workflows".to_string())
        };

        let payload = serde_json::json!({
            "name": name,
            "description": if description.trim().is_empty() { None::<String> } else { Some(description) },
            "cronspec": cronspec,
            "workflow_name": workflow_name,
            "repo_url": repo_url,
            "workflow_path": workflow_path,
            "git_ref": git_ref,
            "inputs": inputs_json,
            "is_active": self.cron_is_active,
        });

        let res = self.api_request(method, &path, Some(payload)).await?;

        if res.status().is_success() {
            self.cron_dialog_active = false;
            self.refresh_cron_workflows().await?;
        } else {
            self.error = Some(format!("Failed to save cron workflow: {}", res.status()));
        }

        Ok(())
    }

    /// Deletes the currently selected cron workflow.
    pub async fn delete_selected_cron_workflow(&mut self) -> Result<()> {
        if let Some(cron) = &self.selected_cron_workflow {
            if self.token.is_some() {
                let res = self
                    .api_request(
                        reqwest::Method::DELETE,
                        &format!("/api/v1/cron-workflows/{}", cron.id),
                        None,
                    )
                    .await?;

                if res.status().is_success() {
                    self.refresh_cron_workflows().await?;
                } else {
                    self.error = Some(format!("Failed to delete cron workflow: {}", res.status()));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stormchaser_model::cron::CronWorkflow;
    use tokio::sync::mpsc;
    use uuid::Uuid;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_cron_workflow(id: Uuid) -> CronWorkflow {
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
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_refresh_cron_workflows_uses_correct_path() {
        let server = MockServer::start().await;
        let id = Uuid::new_v4();

        Mock::given(method("GET"))
            .and(path("/api/v1/cron-workflows"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(vec![make_cron_workflow(id)]),
            )
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(server.uri(), Some("token".to_string()), tx);

        let result = app.refresh_cron_workflows().await;
        assert!(result.is_ok());
        assert_eq!(app.cron_workflows.len(), 1);
        assert_eq!(app.cron_workflows[0].id, id);
        assert!(app.error.is_none());
    }

    #[tokio::test]
    async fn test_delete_cron_workflow_uses_correct_path() {
        let server = MockServer::start().await;
        let id = Uuid::new_v4();

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
        let mut app = App::new(server.uri(), Some("token".to_string()), tx);
        app.selected_cron_workflow = Some(make_cron_workflow(id));

        let result = app.delete_selected_cron_workflow().await;
        assert!(result.is_ok());
        assert!(app.error.is_none());
    }
}
