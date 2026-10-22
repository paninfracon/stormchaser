//! The Docker runner as a [`Backend`] of the shared runner.

use crate::handler::{handle_cancel, handle_task, scan_for_orphans};
use crate::reaper::run_reaper;
use anyhow::Result;
use async_nats::jetstream::Message;
use async_trait::async_trait;
use bollard::Docker;
use std::sync::Arc;
use stormchaser_model::dsl;
use stormchaser_model::events::RunnerStepTypeSchema;
use stormchaser_model::runner::{CONTAINER_TASKS_CONSUMER, CONTAINER_TASKS_SUBJECT};
use stormchaser_runner_core::{Backend, RunnerContext};
use tracing::error;

/// Runs steps as containers on the local Docker daemon.
pub struct DockerBackend {
    /// The Docker daemon
    pub docker: Docker,
    /// Where containers send their logs, through Docker's Loki log driver
    pub loki_url: Option<String>,
}

#[async_trait]
impl Backend for DockerBackend {
    fn kind(&self) -> &'static str {
        "docker"
    }

    fn health_port(&self) -> u16 {
        8081
    }

    fn capabilities(&self) -> Vec<String> {
        ["docker", "linux", "container"].map(String::from).to_vec()
    }

    fn step_types(&self) -> Result<Vec<RunnerStepTypeSchema>> {
        let schema =
            stormchaser_model::schema_gen::draft07_schema_for::<dsl::CommonContainerSpec>();
        Ok(vec![RunnerStepTypeSchema {
            step_type: "RunContainer".to_string(),
            schema: Some(serde_json::to_value(schema)?),
            documentation: Some(
                "Runs a container using Docker with a minimal common set of parameters."
                    .to_string(),
            ),
        }])
    }

    fn task_consumers(&self) -> Vec<(&'static str, &'static str)> {
        vec![(CONTAINER_TASKS_CONSUMER, CONTAINER_TASKS_SUBJECT)]
    }

    async fn reap(self: Arc<Self>) {
        if let Err(e) = run_reaper(self.docker.clone()).await {
            error!("Reaper error: {:?}", e);
        }
    }

    async fn adopt_orphans(&self, ctx: &RunnerContext) -> Result<()> {
        scan_for_orphans(
            self.docker.clone(),
            ctx.nats.clone(),
            ctx.runner_id.clone(),
            ctx.encryption_key.clone(),
        )
        .await
    }

    async fn handle_task(self: Arc<Self>, msg: Message, ctx: RunnerContext) {
        handle_task(
            msg,
            self.docker.clone(),
            ctx.nats,
            ctx.runner_id,
            ctx.encryption_key,
            self.loki_url.clone(),
        )
        .await
    }

    async fn handle_cancel(self: Arc<Self>, msg: Message) {
        handle_cancel(msg, self.docker.clone()).await
    }
}
