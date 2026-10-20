//! A typed client for the Stormchaser API and query service.
//!
//! Every call takes and returns the types in [`stormchaser_model::api`] (and the model
//! types the API returns directly), so the CLI, the TUI and the web UI all read and write
//! them as the API does. Streams come back as server-sent events ([`Client::events`]).

use eventsource_stream::Eventsource;
use futures::{Stream, StreamExt};
use reqwest::{Method, StatusCode};
use reqwest_middleware::{ClientWithMiddleware, RequestBuilder};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use stormchaser_model::api::*;
use stormchaser_model::connections::{ArtifactRegistry, Connection};
use stormchaser_model::cron::CronWorkflow;
use stormchaser_model::event_rules::{EventRule, WebhookConfig};
use stormchaser_model::{
    ConnectionId, CronWorkflowId, RuleId, RunId, StepInstanceId, TestReportId, WebhookId,
};

pub use eventsource_stream::Event;

/// What went wrong with a call.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request could not be sent, or its response read.
    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The request could not be sent through the client's middleware.
    #[error("request failed: {0}")]
    Middleware(#[from] reqwest_middleware::Error),
    /// The server answered with an error status.
    #[error("the server answered {status}: {body}")]
    Status {
        /// The status.
        status: StatusCode,
        /// The response body, as text.
        body: String,
    },
    /// The response body was not what the call expects.
    #[error("unexpected response: {0}")]
    Decode(#[from] serde_json::Error),
    /// A CA certificate could not be read.
    #[error("{0}")]
    Certificate(String),
    /// A stream broke off.
    #[error("stream failed: {0}")]
    Stream(String),
}

impl Error {
    /// The status the server answered with, if it answered.
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Error::Status { status, .. } => Some(*status),
            Error::Http(e) => e.status(),
            _ => None,
        }
    }
}

/// The result of a call.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Builds a [`Client`].
#[derive(Debug, Clone)]
pub struct ClientBuilder {
    api_url: String,
    query_url: Option<String>,
    token: Option<String>,
    ca_pem: Option<Vec<u8>>,
    retries: u32,
}

impl ClientBuilder {
    /// The query service's URL, for [`Client::hydrate`] and [`Client::hydrate_direct`].
    /// Without it they go to the API's URL.
    pub fn query_url(mut self, url: impl Into<String>) -> Self {
        self.query_url = Some(url.into());
        self
    }

    /// The bearer token every call carries.
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// A PEM bundle of CA certificates to trust beside the system's.
    pub fn ca_pem(mut self, pem: Vec<u8>) -> Self {
        self.ca_pem = Some(pem);
        self
    }

    /// How many times a call is retried after a transient failure (none by default).
    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// The client.
    #[expect(
        clippy::disallowed_methods,
        reason = "the client of Stormchaser's own API, at the URL its user gives"
    )]
    pub fn build(self) -> Result<Client> {
        let mut builder = reqwest::Client::builder();
        if let Some(pem) = &self.ca_pem {
            let certs = reqwest::Certificate::from_pem_bundle(pem)
                .map_err(|e| Error::Certificate(format!("not a PEM bundle: {e}")))?;
            if certs.is_empty() {
                return Err(Error::Certificate(
                    "the PEM bundle holds no certificate".into(),
                ));
            }
            builder = builder.tls_certs_merge(certs);
        }
        let mut http = reqwest_middleware::ClientBuilder::new(builder.build()?);
        if self.retries > 0 {
            let policy = ExponentialBackoff::builder().build_with_max_retries(self.retries);
            http = http.with(RetryTransientMiddleware::new_with_policy(policy));
        }
        Ok(Client {
            http: http.build(),
            api_url: self.api_url.trim_end_matches('/').to_string(),
            query_url: self
                .query_url
                .map(|url| url.trim_end_matches('/').to_string()),
            token: self.token,
        })
    }
}

/// A client of the Stormchaser API, and of its query service.
#[derive(Debug, Clone)]
pub struct Client {
    http: ClientWithMiddleware,
    api_url: String,
    query_url: Option<String>,
    token: Option<String>,
}

impl Client {
    /// A builder for a client of the API at `api_url` (such as `https://stormchaser.example.com`).
    pub fn builder(api_url: impl Into<String>) -> ClientBuilder {
        ClientBuilder {
            api_url: api_url.into(),
            query_url: None,
            token: None,
            ca_pem: None,
            retries: 0,
        }
    }

    /// The same client, carrying `token` instead.
    pub fn with_token(&self, token: impl Into<String>) -> Self {
        Client {
            token: Some(token.into()),
            ..self.clone()
        }
    }

    /// The API's URL.
    pub fn api_url(&self) -> &str {
        &self.api_url
    }

    fn request(&self, method: Method, base: &str, path: &str) -> RequestBuilder {
        let request = self.http.request(method, format!("{base}/api/v1{path}"));
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    fn api(&self, method: Method, path: &str) -> RequestBuilder {
        self.request(method, &self.api_url, path)
    }

    /// Sends `request` and returns its response, or the error status it answered with.
    async fn send(request: RequestBuilder) -> Result<reqwest::Response> {
        let response = request.send().await?;
        let status = response.status();
        if status.is_success() {
            Ok(response)
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(Error::Status { status, body })
        }
    }

    async fn json<T: DeserializeOwned>(request: RequestBuilder) -> Result<T> {
        let bytes = Self::send(request).await?.bytes().await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn unit(request: RequestBuilder) -> Result<()> {
        Self::send(request).await.map(|_| ())
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Self::json(self.api(Method::GET, path)).await
    }

    async fn post<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        Self::json(self.api(Method::POST, path).json(body)).await
    }

    /// Any call the typed methods do not cover: sends `body` as JSON, if given, and
    /// returns the response's JSON, or `null` for an empty body.
    pub async fn call(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        let mut request = self.api(method, path);
        if let Some(body) = body {
            request = request.json(body);
        }
        let bytes = Self::send(request).await?.bytes().await?;
        if bytes.is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_slice(&bytes)?)
        }
    }

    // Authentication

    /// Exchanges an authorization code (or an identity provider's token) for an API token.
    pub async fn exchange_token(
        &self,
        request: &AuthExchangeRequest,
    ) -> Result<AuthExchangeResponse> {
        self.post("/auth/exchange", request).await
    }

    /// Exchanges a refresh token for a new API token.
    pub async fn refresh_token(
        &self,
        request: &AuthRefreshRequest,
    ) -> Result<AuthExchangeResponse> {
        self.post("/auth/refresh", request).await
    }

    /// Who the client's token belongs to; an error if it is not valid.
    pub async fn session(&self) -> Result<SessionResponse> {
        self.get("/auth/session").await
    }

    // Runs

    /// The runs `query` selects.
    pub async fn list_runs(&self, query: &ListRunsQuery) -> Result<Vec<WorkflowRunDetail>> {
        Self::json(self.api(Method::GET, "/runs").query(query)).await
    }

    /// A run, with its steps, artifacts and test results.
    pub async fn get_run(&self, id: RunId) -> Result<WorkflowRunFullDetail> {
        self.get(&format!("/runs/{id}")).await
    }

    /// Deletes a run.
    pub async fn delete_run(&self, id: RunId) -> Result<()> {
        Self::unit(self.api(Method::DELETE, &format!("/runs/{id}"))).await
    }

    /// Starts a run of a workflow held in a Git connection's repository.
    pub async fn enqueue(&self, request: &EnqueueRequest) -> Result<EnqueueResponse> {
        self.post("/runs", request).await
    }

    /// Starts a run of a workflow sent with the request.
    pub async fn run_direct(&self, request: &DirectRunRequest) -> Result<EnqueueResponse> {
        self.post("/runs/direct", request).await
    }

    /// The artifacts a run's steps pushed.
    pub async fn run_artifacts(&self, id: RunId) -> Result<Vec<ArtifactRegistry>> {
        self.get(&format!("/runs/{id}/artifacts")).await
    }

    /// The test reports a run's steps stored.
    pub async fn run_test_reports(&self, id: RunId) -> Result<Vec<TestReportSummary>> {
        self.get(&format!("/runs/{id}/reports")).await
    }

    /// The test results of a run, a summary per report.
    pub async fn run_test_summaries(&self, id: RunId) -> Result<Vec<TestSummaryResponse>> {
        self.get(&format!("/runs/{id}/summaries")).await
    }

    /// A test report's file.
    pub async fn test_report(&self, run: RunId, report: TestReportId) -> Result<TestReportContent> {
        self.get(&format!("/runs/{run}/reports/{report}")).await
    }

    /// A step's log lines, at most `limit` of them.
    pub async fn step_logs(
        &self,
        run: RunId,
        step: StepInstanceId,
        limit: Option<u32>,
    ) -> Result<Vec<String>> {
        let mut request = self.api(Method::GET, &format!("/runs/{run}/steps/{step}/logs"));
        if let Some(limit) = limit {
            request = request.query(&[("limit", limit)]);
        }
        Self::json(request).await
    }

    /// Approves an approval step, giving it `outputs` (`{}` for none).
    pub async fn approve(&self, run: RunId, step: StepInstanceId, outputs: &Value) -> Result<()> {
        Self::unit(
            self.api(Method::POST, &format!("/runs/{run}/steps/{step}/approve"))
                .json(outputs),
        )
        .await
    }

    /// Rejects an approval step.
    pub async fn reject(&self, run: RunId, step: StepInstanceId) -> Result<()> {
        Self::unit(self.api(Method::POST, &format!("/runs/{run}/steps/{step}/reject"))).await
    }

    /// Acts on an approval link: approves or rejects the step it was sealed for.
    pub async fn approve_link(&self, token: &str) -> Result<()> {
        Self::unit(self.api(Method::POST, &format!("/approve-link/{token}"))).await
    }

    /// Resumes the `Wait` step the event's `key` and `value` name.
    pub async fn correlate(&self, event: &Value) -> Result<()> {
        Self::unit(self.api(Method::POST, "/events/correlate").json(event)).await
    }

    // Streams

    /// Changes to the runs the caller may read, as they happen.
    pub async fn stream_runs(&self) -> Result<impl Stream<Item = Result<Event>> + use<>> {
        self.events(self.api(Method::GET, "/runs/stream")).await
    }

    /// A run's status, as it changes.
    pub async fn stream_run_status(
        &self,
        id: RunId,
    ) -> Result<impl Stream<Item = Result<Event>> + use<>> {
        self.events(self.api(Method::GET, &format!("/runs/{id}/status/stream")))
            .await
    }

    /// The logs of all a run's steps, as they are written.
    pub async fn stream_run_logs(
        &self,
        id: RunId,
    ) -> Result<impl Stream<Item = Result<Event>> + use<>> {
        self.events(self.api(Method::GET, &format!("/runs/{id}/logs/stream")))
            .await
    }

    /// A step's logs, as they are written.
    pub async fn stream_step_logs(
        &self,
        run: RunId,
        step: StepInstanceId,
    ) -> Result<impl Stream<Item = Result<Event>> + use<>> {
        self.events(self.api(
            Method::GET,
            &format!("/runs/{run}/steps/{step}/logs/stream"),
        ))
        .await
    }

    /// The server-sent events of `request`'s response.
    pub async fn events(
        &self,
        request: RequestBuilder,
    ) -> Result<impl Stream<Item = Result<Event>> + use<>> {
        let response = Self::send(request.header("Accept", "text/event-stream")).await?;
        Ok(response
            .bytes_stream()
            .eventsource()
            .map(|event| event.map_err(|e| Error::Stream(e.to_string()))))
    }

    // Query service

    fn query(&self, method: Method, path: &str) -> RequestBuilder {
        let base = self.query_url.as_deref().unwrap_or(&self.api_url);
        self.request(method, base, path)
    }

    /// Fills in the input form of a workflow in a Git connection's repository, as its
    /// queries complete.
    pub async fn hydrate(
        &self,
        request: &HydrateSchemaRequest,
    ) -> Result<impl Stream<Item = Result<HydrationEvent>> + use<>> {
        let events = self
            .events(self.query(Method::POST, "/schema/hydrate").json(request))
            .await?;
        Ok(events.map(|event| Ok(serde_json::from_str(&event?.data)?)))
    }

    /// Fills in the input form of a workflow sent with the request.
    pub async fn hydrate_direct(
        &self,
        request: &HydrateDirectRequest,
    ) -> Result<impl Stream<Item = Result<HydrationEvent>> + use<>> {
        let events = self
            .events(
                self.query(Method::POST, "/schema/hydrate-direct")
                    .json(request),
            )
            .await?;
        Ok(events.map(|event| Ok(serde_json::from_str(&event?.data)?)))
    }

    // Workflows

    /// The DSL's JSON Schema.
    pub async fn schema(&self) -> Result<Value> {
        self.get("/schema").await
    }

    /// A workflow file read from a Git connection's repository.
    pub async fn parse_git(&self, request: &ParseGitRequest) -> Result<String> {
        Ok(
            Self::send(self.api(Method::POST, "/schema/parse-git").json(request))
                .await?
                .text()
                .await?,
        )
    }

    // Connections

    /// Every connection.
    pub async fn list_connections(&self) -> Result<Vec<Connection>> {
        self.get("/connections").await
    }

    /// A connection.
    pub async fn get_connection(&self, id: ConnectionId) -> Result<Connection> {
        self.get(&format!("/connections/{id}")).await
    }

    /// Creates a connection.
    pub async fn create_connection(
        &self,
        request: &CreateStorageBackendRequest,
    ) -> Result<CreatedResponse> {
        self.post("/connections", request).await
    }

    /// Changes a connection.
    pub async fn update_connection(
        &self,
        id: ConnectionId,
        request: &UpdateStorageBackendRequest,
    ) -> Result<()> {
        Self::unit(
            self.api(Method::PATCH, &format!("/connections/{id}"))
                .json(request),
        )
        .await
    }

    /// Deletes a connection.
    pub async fn delete_connection(&self, id: ConnectionId) -> Result<()> {
        Self::unit(self.api(Method::DELETE, &format!("/connections/{id}"))).await
    }

    /// Tries a connection's settings without storing them.
    pub async fn test_connection(
        &self,
        request: &TestConnectionRequest,
    ) -> Result<TestConnectionResponse> {
        self.post("/connections/test", request).await
    }

    // Webhooks

    /// Every webhook, without secrets.
    pub async fn list_webhooks(&self) -> Result<Vec<WebhookConfig>> {
        self.get("/webhooks").await
    }

    /// A webhook, without its secret.
    pub async fn get_webhook(&self, id: WebhookId) -> Result<WebhookConfig> {
        self.get(&format!("/webhooks/{id}")).await
    }

    /// Creates a webhook.
    pub async fn create_webhook(&self, request: &CreateWebhookRequest) -> Result<CreatedResponse> {
        self.post("/webhooks", request).await
    }

    /// Changes a webhook.
    pub async fn update_webhook(
        &self,
        id: WebhookId,
        request: &UpdateWebhookRequest,
    ) -> Result<()> {
        Self::unit(
            self.api(Method::PATCH, &format!("/webhooks/{id}"))
                .json(request),
        )
        .await
    }

    /// Deletes a webhook.
    pub async fn delete_webhook(&self, id: WebhookId) -> Result<()> {
        Self::unit(self.api(Method::DELETE, &format!("/webhooks/{id}"))).await
    }

    // Event rules

    /// Every event rule.
    pub async fn list_event_rules(&self) -> Result<Vec<EventRule>> {
        self.get("/rules").await
    }

    /// Creates an event rule.
    pub async fn create_event_rule(
        &self,
        request: &CreateEventRuleRequest,
    ) -> Result<CreatedResponse> {
        self.post("/rules", request).await
    }

    /// Deletes an event rule.
    pub async fn delete_event_rule(&self, id: RuleId) -> Result<()> {
        Self::unit(self.api(Method::DELETE, &format!("/rules/{id}"))).await
    }

    // Cron schedules

    /// Every cron schedule.
    pub async fn list_cron_workflows(&self) -> Result<Vec<CronWorkflow>> {
        self.get("/cron-workflows").await
    }

    /// Creates a cron schedule. Its trigger token is shown only here.
    pub async fn create_cron_workflow(
        &self,
        request: &CreateCronWorkflowRequest,
    ) -> Result<CronWorkflowResponse> {
        self.post("/cron-workflows", request).await
    }

    /// Deletes a cron schedule.
    pub async fn delete_cron_workflow(&self, id: CronWorkflowId) -> Result<()> {
        Self::unit(self.api(Method::DELETE, &format!("/cron-workflows/{id}"))).await
    }
}
