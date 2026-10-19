//! The Kubernetes runner as a [`Backend`] of the shared runner.

use crate::cancel::handle_cancel;
use crate::cluster::ClusterPool;
use crate::reaper::run_reaper;
use crate::scanner::scan_for_orphans;
use crate::task::handle_task;
use anyhow::Result;
use async_nats::jetstream::Message;
use async_trait::async_trait;
use std::sync::Arc;
use stormchaser_model::dsl;
use stormchaser_model::events::RunnerStepTypeSchema;
use stormchaser_model::runner::{
    CONTAINER_TASKS_CONSUMER, CONTAINER_TASKS_SUBJECT, K8S_JOB_TASKS_CONSUMER,
    K8S_JOB_TASKS_SUBJECT,
};
use stormchaser_runner_core::{Backend, RunnerContext};
use tracing::error;

/// Runs steps as Kubernetes Jobs in the step namespace.
pub struct K8sBackend {
    /// The clusters the runner can start Jobs in
    pub cluster_pool: Arc<ClusterPool>,
    /// The namespace steps run in
    pub step_namespace: String,
}

#[async_trait]
impl Backend for K8sBackend {
    fn kind(&self) -> &'static str {
        "k8s"
    }

    fn health_port(&self) -> u16 {
        8080
    }

    fn capabilities(&self) -> Vec<String> {
        ["k8s", "docker", "linux", "container"]
            .map(String::from)
            .to_vec()
    }

    fn step_types(&self) -> Result<Vec<RunnerStepTypeSchema>> {
        let common =
            stormchaser_model::schema_gen::draft07_schema_for::<dsl::CommonContainerSpec>();
        let k8s_job = stormchaser_model::schema_gen::draft07_schema_for::<dsl::K8sJobSpec>();
        Ok(vec![
            RunnerStepTypeSchema {
                step_type: "RunContainer".to_string(),
                schema: Some(serde_json::to_value(common)?),
                documentation: Some("Runs a container using a minimal common set of parameters portable across different runners.".to_string()),
            },
            RunnerStepTypeSchema {
                step_type: "RunK8sJob".to_string(),
                schema: Some(serde_json::to_value(k8s_job)?),
                documentation: Some("Runs a native Kubernetes Job with full access to all Job and Pod spec options.".to_string()),
            },
        ])
    }

    /// Container tasks come from the consumer Docker runners share, so each goes to one
    /// runner; Kubernetes jobs only this kind of runner can run.
    fn task_consumers(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            (CONTAINER_TASKS_CONSUMER, CONTAINER_TASKS_SUBJECT),
            (K8S_JOB_TASKS_CONSUMER, K8S_JOB_TASKS_SUBJECT),
        ]
    }

    async fn reap(self: Arc<Self>) {
        if let Err(e) = run_reaper(self.cluster_pool.clone(), self.step_namespace.clone()).await {
            error!("Reaper error: {:?}", e);
        }
    }

    async fn adopt_orphans(&self, ctx: &RunnerContext) -> Result<()> {
        scan_for_orphans(
            self.cluster_pool.clone(),
            ctx.nats.clone(),
            ctx.runner_id.clone(),
            ctx.encryption_key.clone(),
            &self.step_namespace,
        )
        .await
    }

    async fn handle_task(self: Arc<Self>, msg: Message, ctx: RunnerContext) {
        handle_task(
            msg,
            self.cluster_pool.clone(),
            ctx.nats,
            ctx.runner_id,
            ctx.encryption_key,
            self.step_namespace.clone(),
        )
        .await
    }

    async fn handle_cancel(self: Arc<Self>, msg: Message) {
        handle_cancel(msg, self.cluster_pool.clone(), self.step_namespace.clone()).await
    }
}
