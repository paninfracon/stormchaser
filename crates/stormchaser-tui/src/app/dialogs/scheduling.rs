use crate::app::refused;
use anyhow::Result;
use futures::StreamExt;
use ratatui_textarea::TextArea;
use stormchaser_dsl::StormchaserParser;
use stormchaser_model::api::{
    DirectRunRequest, EnqueueRequest, HydrateDirectRequest, HydrationStatus,
};

impl<'a> crate::app::App<'a> {
    /// Opens the file browser for selecting a local `.storm` file.
    pub fn open_file_browser(&mut self) {
        self.file_browser_active = true;
    }

    /// Opens the dialog to schedule a workflow from a Git repository.
    pub fn open_schedule_git_dialog(&mut self) {
        self.schedule_git_dialog_active = true;
        self.schedule_git_focus = 0;
        self.schedule_git_inputs = vec![
            TextArea::default(), // repo_url
            TextArea::default(), // workflow_path
            TextArea::default(), // git_ref
        ];
    }

    /// Submits the data from the schedule git dialog to start a workflow run.
    pub async fn submit_schedule_git(&mut self) -> Result<()> {
        if self.schedule_git_inputs.len() == 3 {
            let connection = self.schedule_git_inputs[0].lines()[0].trim().to_string();
            let workflow_path = self.schedule_git_inputs[1].lines()[0].trim().to_string();
            let git_ref = self.schedule_git_inputs[2].lines()[0].trim().to_string();

            if connection.is_empty() || workflow_path.is_empty() || git_ref.is_empty() {
                self.error = Some("All fields must be provided".to_string());
                return Ok(());
            }

            let workflow_name = workflow_path
                .split('/')
                .next_back()
                .unwrap_or("manual_run")
                .to_string();

            let request = EnqueueRequest {
                workflow_name,
                connection,
                workflow_path,
                git_ref,
                inputs: serde_json::json!({}),
                overrides: None,
                sops_file: None,
                sops_role_arn: None,
            };
            match self.api().enqueue(&request).await {
                Ok(_) => {
                    self.schedule_git_dialog_active = false;
                    self.refresh_runs().await?;
                }
                Err(e) => {
                    self.error = Some(format!("Failed to schedule workflow: {}", refused(e)?));
                }
            }
        }
        Ok(())
    }

    pub async fn hydrate_schema_blocking(
        &self,
        schema: &serde_json::Value,
        inputs: &serde_json::Value,
        dsl: &str,
    ) -> Result<(serde_json::Value, String)> {
        // The service reads the schema and queries out of the workflow itself; `schema`
        // is only what to fall back on if it sends nothing.
        let request = HydrateDirectRequest {
            dsl: dsl.to_string(),
            inputs: inputs.clone(),
        };

        let mut final_schema = schema.clone();
        let mut final_status = "Update pending".to_string();

        let Ok(events) = self.api().hydrate_direct(&request).await else {
            return Ok((final_schema, final_status));
        };
        let mut events = std::pin::pin!(events);
        while let Some(event) = events.next().await {
            let Ok(event) = event else { continue };
            final_status = serde_json::to_value(&event.status)?
                .as_str()
                .unwrap_or_default()
                .to_string();
            final_schema = event.hydrated_schema;
            if event.status != HydrationStatus::UpdatePending {
                break;
            }
        }

        Ok((final_schema, final_status))
    }

    pub fn strip_required(schema: &mut serde_json::Value) {
        if let serde_json::Value::Object(obj) = schema {
            obj.remove("required");
            if let Some(serde_json::Value::Object(props)) = obj.get_mut("properties") {
                for (_, prop) in props {
                    Self::strip_required(prop);
                }
            }
            if let Some(serde_json::Value::Array(items)) = obj.get_mut("items") {
                for item in items {
                    Self::strip_required(item);
                }
            } else if let Some(item) = obj.get_mut("items") {
                Self::strip_required(item);
            }
        }
    }

    /// Handles the submission of a selected local `.storm` file, optionally prompting for inputs.
    pub async fn submit_file(&mut self) -> Result<()> {
        let path = self
            .file_explorer
            .current_entry()
            .map(|e| e.path.clone())
            .unwrap_or_default();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("storm") {
            let dsl = std::fs::read_to_string(path)?;

            if let Ok(workflow) = StormchaserParser.parse(&dsl) {
                let inputs_schema = if let Some(mut schema) = workflow.inputs_schema {
                    if schema.get("properties").is_none() && schema.get("input").is_some() {
                        let mut properties = serde_json::Map::new();
                        if let Some(inputs_obj) = schema.get("input").and_then(|v| v.as_object()) {
                            for (k, v) in inputs_obj {
                                properties.insert(k.clone(), v.clone());
                            }
                        }
                        if let Some(obj) = schema.as_object_mut() {
                            obj.insert(
                                "properties".to_string(),
                                serde_json::Value::Object(properties),
                            );
                            obj.insert("type".to_string(), serde_json::json!("object"));
                        }
                    }
                    Some(schema)
                } else {
                    None
                };

                if let Some(schema_val) = inputs_schema {
                    let initial_inputs = serde_json::json!({});
                    let (hydrated_schema, status) = self
                        .hydrate_schema_blocking(&schema_val, &initial_inputs, &dsl)
                        .await
                        .unwrap_or((schema_val, "".to_string()));

                    let mut dialog = crate::app::schema_dialog::SchemaDialog::new(
                        hydrated_schema,
                        dsl,
                        initial_inputs,
                        workflow.inputs_view,
                    );
                    dialog.hydration_status = status;
                    self.pending_schema_ui = Some(dialog);
                    self.file_browser_active = false;
                    return Ok(());
                } else if !workflow.inputs.is_empty() {
                    let mut properties = serde_json::Map::new();
                    for input in workflow.inputs {
                        let mut prop = serde_json::Map::new();
                        prop.insert("type".to_string(), serde_json::json!("string"));
                        prop.insert(
                            "description".to_string(),
                            serde_json::json!(input.description.as_deref().unwrap_or(&input.name)),
                        );
                        if let Some(default) = &input.default {
                            if let Some(s) = default.as_str() {
                                prop.insert("default".to_string(), serde_json::json!(s));
                            } else {
                                prop.insert(
                                    "default".to_string(),
                                    serde_json::json!(default.to_string()),
                                );
                            }
                        }
                        properties.insert(input.name, serde_json::Value::Object(prop));
                    }
                    let schema = serde_json::json!({
                    "type": "object",
                    "title": "Workflow Inputs",
                    "properties": properties
                    });
                    self.pending_schema_ui = Some(crate::app::schema_dialog::SchemaDialog::new(
                        schema,
                        dsl,
                        serde_json::json!({}),
                        None,
                    ));
                    self.file_browser_active = false;
                    return Ok(());
                }
            }

            let request = DirectRunRequest {
                dsl,
                inputs: serde_json::json!({}),
            };
            match self.api().run_direct(&request).await {
                Ok(_) => {
                    self.file_browser_active = false;
                    self.refresh_runs().await?;
                }
                Err(e) => {
                    self.error = Some(format!("Failed to submit workflow: {}", refused(e)?));
                }
            }
        }
        Ok(())
    }

    /// Submits the dynamically generated form with inputs for a local workflow file.
    pub async fn submit_direct_form(&mut self, inputs_json: serde_json::Value) -> Result<()> {
        if let Some(dsl) = self.direct_submit_dsl.take() {
            let request = DirectRunRequest {
                dsl,
                inputs: inputs_json,
            };
            match self.api().run_direct(&request).await {
                Ok(response) => {
                    self.force_select_run_id = Some(response.run_id);
                    self.runs_state.select(Some(0));
                    self.selected_run = None;
                }
                Err(e) => {
                    self.error = Some(format!("Failed to submit workflow: {}", refused(e)?));
                }
            }
        }
        Ok(())
    }

    /// Lints the currently selected local `.storm` file.
    pub async fn lint_file(&mut self) -> Result<()> {
        let path = self
            .file_explorer
            .current_entry()
            .map(|e| e.path.clone())
            .unwrap_or_default();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("storm") {
            let dsl = std::fs::read_to_string(path)?;

            match stormchaser_dsl::StormchaserParser.parse(&dsl) {
                Ok(_) => {
                    self.error = Some("Lint Successful: The file is valid.".to_string());
                }
                Err(e) => {
                    self.error = Some(format!("Lint Error: {}", e));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use crate::app::App;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
    fn test_open_file_browser() {
        let mut app = setup_app();
        app.open_file_browser();
        assert!(app.file_browser_active);
    }

    #[test]
    fn test_open_schedule_git_dialog() {
        let mut app = setup_app();
        app.open_schedule_git_dialog();
        assert!(app.schedule_git_dialog_active);
        assert_eq!(app.schedule_git_focus, 0);
        assert_eq!(app.schedule_git_inputs.len(), 3);
    }

    #[tokio::test]
    async fn test_submit_schedule_git_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "run_id": stormchaser_model::RunId::new_v4(),
                "status": "queued",
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let (tx, _) = tokio::sync::mpsc::channel(1);
        let mut app = App::new(server.uri(), server.uri(), Some("token".to_string()), tx);
        app.open_schedule_git_dialog();
        app.schedule_git_inputs[0].insert_str("http://repo");
        app.schedule_git_inputs[1].insert_str("wf.yaml");
        app.schedule_git_inputs[2].insert_str("main");

        let res = app.submit_schedule_git().await;
        res.unwrap();
        assert!(app.error.is_none());
        assert!(!app.schedule_git_dialog_active);
    }

    #[tokio::test]
    async fn test_submit_direct_form_success() {
        let server = MockServer::start().await;
        let run_id = stormchaser_model::RunId::new_v4();
        Mock::given(method("POST"))
            .and(path("/api/v1/runs/direct"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "run_id": run_id,
                "status": "queued",
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let (tx, _) = tokio::sync::mpsc::channel(1);
        let mut app = App::new(server.uri(), server.uri(), Some("token".to_string()), tx);
        app.direct_submit_dsl = Some("test dsl".to_string());

        let res = app.submit_direct_form(json!({"key": "val"})).await;
        res.unwrap();
        assert!(app.error.is_none());
        assert!(app.direct_submit_dsl.is_none()); // Taken
        assert_eq!(app.force_select_run_id, Some(run_id));
    }

    #[tokio::test]
    async fn test_submit_direct_form_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/runs/direct"))
            .respond_with(ResponseTemplate::new(400).set_body_string("Bad Request"))
            .mount(&server)
            .await;

        let (tx, _) = tokio::sync::mpsc::channel(1);
        let mut app = App::new(server.uri(), server.uri(), Some("token".to_string()), tx);
        app.direct_submit_dsl = Some("test dsl".to_string());

        let res = app.submit_direct_form(json!({})).await;
        res.unwrap();
        assert!(app.error.is_some());
        assert!(app.error.unwrap().contains("Failed to submit workflow"));
    }

    #[tokio::test]
    async fn test_hydrate_schema_blocking_success() {
        let server = MockServer::start().await;
        let query = MockServer::start().await;
        let event = |status: &str, schema: serde_json::Value| {
            let event = json!({
                "status": status,
                "hydrated_schema": schema,
                "query_status": {},
                "validation_errors": [],
            });
            format!("data: {event}\n\n")
        };
        let sse_body = event("Update pending", json!({"type": "object"}))
            + &event("Completed", json!({"type": "string"}));
        Mock::given(method("POST"))
            .and(path("/api/v1/schema/hydrate-direct"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body))
            .mount(&query)
            .await;

        // Hydration goes to the query service, not the API.
        let (tx, _) = tokio::sync::mpsc::channel(1);
        let app = App::new(server.uri(), query.uri(), Some("token".to_string()), tx);

        let schema = json!({"type": "object"});
        let inputs = json!({});
        let (new_schema, status) = app
            .hydrate_schema_blocking(&schema, &inputs, "workflow \"w\" {}")
            .await
            .unwrap();
        assert_eq!(status, "Completed");
        assert_eq!(new_schema, json!({"type": "string"}));
    }

    #[test]
    fn test_strip_required() {
        let mut schema = json!({
            "type": "object",
            "required": ["a"],
            "properties": {
                "a": { "type": "string" },
                "b": {
                    "type": "object",
                    "required": ["c"],
                    "properties": { "c": { "type": "string" } }
                }
            },
            "items": [
                {
                    "type": "object",
                    "required": ["d"],
                    "properties": { "d": { "type": "string" } }
                }
            ]
        });

        App::strip_required(&mut schema);

        assert!(schema.get("required").is_none());
        assert!(schema["properties"]["b"].get("required").is_none());
        assert!(schema["items"][0].get("required").is_none());
    }
}
