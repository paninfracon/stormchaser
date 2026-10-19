use crate::utils::{for_stderr, print_events};
use anyhow::Result;
use stormchaser_client::Client;
use stormchaser_model::api::WorkflowRunFullDetail;
use stormchaser_model::{RunId, StepInstanceId};

/// Streams a step's logs. `step` is a step instance id, or a step name: the API streams by
/// instance, so a name is looked up in the run, and a step that ran more than once is
/// followed in its most recent instance, the one still running or last to run.
pub async fn stream_logs(client: &Client, id: RunId, step: String) -> Result<()> {
    let instance = match step.parse::<StepInstanceId>() {
        Ok(instance) => instance,
        Err(_) => resolve_step(&client.get_run(id).await?, &step)?,
    };
    print_events(client.stream_step_logs(id, instance).await?, false).await
}

/// The most recent instance of the step named `name` in `run`.
fn resolve_step(run: &WorkflowRunFullDetail, name: &str) -> Result<StepInstanceId> {
    let mut instances: Vec<_> = run
        .steps
        .iter()
        .map(|step| &step.instance)
        .filter(|instance| instance.step_name == name)
        .collect();
    instances.sort_by_key(|instance| (instance.created_at, instance.iteration_index));
    let Some(latest) = instances.pop() else {
        anyhow::bail!(
            "Run {} has no step named '{}'",
            run.detail.id,
            for_stderr(name)
        );
    };
    if !instances.is_empty() {
        let earlier: Vec<String> = instances.iter().map(|i| i.id.to_string()).collect();
        eprintln!(
            "Step '{}' ran {} times; following its latest instance {}. Pass an instance id \
             for another: {}",
            for_stderr(name),
            instances.len() + 1,
            latest.id,
            earlier.join(", ")
        );
    }
    Ok(latest.id)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{json, Value};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A run as `GET /runs/{id}` returns it, with steps of the given ids, names and
    /// creation times.
    pub(crate) fn run_json(id: RunId, steps: &[(StepInstanceId, &str, &str)]) -> Value {
        let steps: Vec<Value> = steps
            .iter()
            .map(|(step_id, name, created_at)| {
                json!({
                    "instance": {
                        "id": step_id,
                        "run_id": id,
                        "step_name": name,
                        "step_type": "RunContainer",
                        "status": "succeeded",
                        "iteration_index": null,
                        "runner_id": null,
                        "affinity_context": null,
                        "started_at": null,
                        "finished_at": null,
                        "exit_code": null,
                        "error": null,
                        "spec": {},
                        "params": {},
                        "created_at": created_at
                    },
                    "outputs": [],
                    "history": [],
                    "logs": []
                })
            })
            .collect();
        json!({
            "detail": {
                "id": id,
                "workflow_name": "wf",
                "initiating_user": "alice",
                "repo_url": "",
                "workflow_path": "",
                "git_ref": "",
                "status": "running",
                "version": 1,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "started_resolving_at": null,
                "started_at": null,
                "finished_at": null,
                "error": null,
                "inputs": {},
                "source_code": "",
                "dsl_version": "0.1"
            },
            "steps": steps,
            "artifacts": [],
            "test_summaries": [],
            "test_cases": []
        })
    }

    async fn mount_stream(server: &MockServer, id: RunId, step: StepInstanceId) {
        let sse_body = "event: message\ndata: Hello from logs\n\n\
                        event: error\ndata: stream ended\n\n";
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/runs/{}/steps/{}/logs/stream",
                id, step
            )))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .expect(1)
            .mount(server)
            .await;
    }

    fn client(server: &MockServer) -> Client {
        Client::builder(server.uri())
            .token("fake_token")
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn a_step_name_is_streamed_by_its_latest_instance() {
        let server = MockServer::start().await;
        let id = RunId::new_v4();
        let (first, latest, other) = (
            StepInstanceId::new_v4(),
            StepInstanceId::new_v4(),
            StepInstanceId::new_v4(),
        );
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}", id)))
            .respond_with(ResponseTemplate::new(200).set_body_json(run_json(
                id,
                &[
                    (latest, "build", "2026-01-01T00:02:00Z"),
                    (first, "build", "2026-01-01T00:01:00Z"),
                    (other, "test", "2026-01-01T00:03:00Z"),
                ],
            )))
            .expect(1)
            .mount(&server)
            .await;
        mount_stream(&server, id, latest).await;

        stream_logs(&client(&server), id, "build".to_string())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_step_instance_id_is_streamed_directly() {
        let server = MockServer::start().await;
        let id = RunId::new_v4();
        let step = StepInstanceId::new_v4();
        mount_stream(&server, id, step).await;

        stream_logs(&client(&server), id, step.to_string())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn an_unknown_step_name_is_an_error() {
        let server = MockServer::start().await;
        let id = RunId::new_v4();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}", id)))
            .respond_with(ResponseTemplate::new(200).set_body_json(run_json(id, &[])))
            .mount(&server)
            .await;

        let err = stream_logs(&client(&server), id, "missing".to_string())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no step named 'missing'"), "{err}");
    }
}
