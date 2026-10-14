use crate::hydration::run_hydration_loop;
use crate::limits::{hydration_slots, Slot};
use crate::models::{HydrateDirectRequest, HydrateSchemaRequest};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value;
use stormchaser_api::authz::Caller;
use stormchaser_api::AppState;
use tokio_stream::StreamExt;

/// Largest workflow text accepted for a direct hydration.
const MAX_DIRECT_DSL_BYTES: usize = 1024 * 1024;

/// Hydrates the input form of a workflow in a repository.
///
/// The workflow is read from the Git connection the caller names, which the caller must
/// be allowed to use, and its schema and queries are taken from there. Nothing in the
/// request decides what is run.
pub async fn hydrate_schema(
    // Caller rejects with 401 before the body is read.
    caller: Caller,
    State(state): State<AppState>,
    Json(payload): Json<HydrateSchemaRequest>,
) -> Result<Response, StatusCode> {
    // Taken before the repository is read, which is work too
    let slot = take_slot(&caller)?;
    let source = stormchaser_api::routes::ParseGitRequest {
        connection: payload.connection,
        workflow_path: payload.workflow_path,
        git_ref: payload.git_ref,
    };
    let dsl =
        stormchaser_api::routes::schema::read_workflow_from_git(&state, &caller, &source).await?;
    hydrate(dsl, payload.inputs, state, &caller, slot).await
}

/// Hydrates the input form of a workflow the caller supplies as text.
pub async fn hydrate_schema_direct(
    caller: Caller,
    State(state): State<AppState>,
    Json(payload): Json<HydrateDirectRequest>,
) -> Result<Response, StatusCode> {
    if payload.dsl.len() > MAX_DIRECT_DSL_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let slot = take_slot(&caller)?;
    hydrate(payload.dsl, payload.inputs, state, &caller, slot).await
}

/// A place for one more hydration by `caller`, or 429 if they or the service are busy.
fn take_slot(caller: &Caller) -> Result<Slot, StatusCode> {
    hydration_slots()
        .try_acquire(&caller.owner_key())
        .ok_or_else(|| {
            tracing::warn!(
                "Refusing a hydration for {}: too many in flight",
                caller.owner_key()
            );
            StatusCode::TOO_MANY_REQUESTS
        })
}

/// Parses `dsl` and streams the hydration of its input form. `slot` is held until the
/// hydration ends.
async fn hydrate(
    dsl: String,
    inputs: Value,
    state: AppState,
    caller: &Caller,
    slot: Slot,
) -> Result<Response, StatusCode> {
    // Parsing is CPU work; keep it off the threads that serve requests
    let workflow =
        tokio::task::spawn_blocking(move || stormchaser_dsl::StormchaserParser.parse(&dsl))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .map_err(|e| {
                tracing::warn!(
                    "Refusing to hydrate a workflow that does not parse: {:#}",
                    e
                );
                StatusCode::BAD_REQUEST
            })?;
    let schema = stormchaser_dsl::form::inputs_form_schema(&workflow)
        .unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} }));
    let queries = workflow.queries;

    // Connection use is authorized against the caller, named the same way a run records
    // its initiating user.
    let user = caller.owner_key();

    let (tx, rx) = tokio::sync::mpsc::channel(100);
    tokio::spawn(async move {
        run_hydration_loop(schema, inputs, queries, Some(state), user, tx).await;
        drop(slot);
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|event| {
        Ok::<_, std::convert::Infallible>(
            axum::response::sse::Event::default()
                .json_data(event)
                .unwrap(),
        )
    });

    Ok(axum::response::sse::Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use axum::{routing::post, Router};
    use serde_json::json;
    use std::sync::Arc;
    use tower::ServiceExt;
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A workflow whose form is filled from its own query.
    const WORKFLOW: &str = r#"
workflow "hydration-test" {
  query "env_options" {
    type = "mock"
    params = {
      items = "dev,staging,prod"
    }
  }

  inputs {
    environment = string(enum("${queries.env_options}"))
  }

  steps {}
}
"#;

    /// The service's router over the test database, or `None` when there is no test
    /// stack to run against.
    async fn app() -> Option<(Router, sqlx::PgPool)> {
        let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
        let nats = stormchaser_model::nats::connect(nats_url).await.ok()?;
        let pool = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").ok()?)
            .await
            .ok()?;
        let state = AppState {
            pool: pool.clone(),
            nats,
            opa: Arc::new(stormchaser_model::OpaClient::new(None, None)),
            oidc_config: None,
            jwks: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            log_backend: None,
            api_base_url: "http://localhost:3000".to_string(),
            mcp_allowed_hosts: Vec::new(),
        };
        let router = Router::new()
            .route("/api/v1/schema/hydrate", post(hydrate_schema))
            .route("/api/v1/schema/hydrate-direct", post(hydrate_schema_direct))
            .with_state(state);
        Some((router, pool))
    }

    fn token() -> String {
        let claims = stormchaser_api::Claims {
            sub: "hydration-tester".to_string(),
            email: Some("hydration-tester@paninfracon.net".to_string()),
            exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        };
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(stormchaser_api::JWT_SECRET),
        )
        .unwrap()
    }

    async fn post_json(
        app: &Router,
        path: &str,
        body: Value,
        token: Option<&str>,
    ) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method("POST")
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// A query of the caller's own, pointed at `target`, of the kind the old request
    /// body carried.
    fn smuggled(target: &str) -> Value {
        json!([{
            "name": "env_options",
            "type": "api",
            "params": { "url": target, "method": "GET" }
        }])
    }

    #[tokio::test]
    async fn a_pasted_workflow_is_hydrated_from_its_own_queries_only() {
        let Some((app, _pool)) = app().await else {
            return;
        };
        // Anything that reaches this server is a query the workflow did not contain
        let outside = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_body_json(json!(["stolen"])))
            .expect(0)
            .mount(&outside)
            .await;
        let token = token();

        let (status, body) = post_json(
            &app,
            "/api/v1/schema/hydrate-direct",
            json!({
                "dsl": WORKFLOW,
                "inputs": {},
                "queries": smuggled(&outside.uri()),
                "schema": { "type": "object", "properties": { "x": { "enum": "${queries.env_options}" } } },
            }),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("staging"),
            "form was not filled from the workflow: {body}"
        );
        assert!(!body.contains("stolen"), "{body}");

        // No token, no hydration; and text that is not a workflow is refused
        let (status, _) = post_json(
            &app,
            "/api/v1/schema/hydrate-direct",
            json!({ "dsl": WORKFLOW }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = post_json(
            &app,
            "/api/v1/schema/hydrate-direct",
            json!({ "dsl": "this is not a workflow {" }),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_repository_workflow_is_read_by_the_service_not_sent_by_the_caller() {
        let Some((app, pool)) = app().await else {
            return;
        };
        // The test repository is a local directory
        std::env::set_var("STORMCHASER_GIT_ALLOWED_SCHEMES", "https,ssh,file");
        let outside = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_body_json(json!(["stolen"])))
            .expect(0)
            .mount(&outside)
            .await;
        let token = token();

        let origin = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(origin.path()).unwrap();
        std::fs::write(origin.path().join("flow.storm"), WORKFLOW).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("flow.storm")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        let commit = repo
            .commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap()
            .to_string();

        let connection_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO connections (id, name, connection_type, config) VALUES ($1, $2, 'git', $3)",
        )
        .bind(connection_id)
        .bind(format!("hydration-repo-{connection_id}"))
        .bind(json!({ "repo_url": format!("file://{}", origin.path().display()) }))
        .execute(&pool)
        .await
        .unwrap();

        let (status, body) = post_json(
            &app,
            "/api/v1/schema/hydrate",
            json!({
                "connection": connection_id.to_string(),
                "workflow_path": "flow.storm",
                "git_ref": commit,
                "inputs": {},
                "queries": smuggled(&outside.uri()),
            }),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.contains("staging"),
            "form was not filled from the repository: {body}"
        );
        assert!(!body.contains("stolen"), "{body}");

        // The old request, which carried a schema and queries and named no workflow
        let (status, _) = post_json(
            &app,
            "/api/v1/schema/hydrate",
            json!({ "schema": { "type": "object" }, "queries": smuggled(&outside.uri()) }),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        // A file that is not there, a path out of the repository, a connection that is not
        for (path, connection, expected) in [
            (
                "missing.storm",
                connection_id.to_string(),
                StatusCode::BAD_REQUEST,
            ),
            (
                "../flow.storm",
                connection_id.to_string(),
                StatusCode::BAD_REQUEST,
            ),
            (
                "flow.storm",
                uuid::Uuid::new_v4().to_string(),
                StatusCode::NOT_FOUND,
            ),
        ] {
            let (status, _) = post_json(
                &app,
                "/api/v1/schema/hydrate",
                json!({ "connection": connection, "workflow_path": path, "git_ref": commit }),
                Some(&token),
            )
            .await;
            assert_eq!(status, expected, "{path}");
        }

        sqlx::query("DELETE FROM connections WHERE id = $1")
            .bind(connection_id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
