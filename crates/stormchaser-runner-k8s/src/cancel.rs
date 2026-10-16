use crate::cluster::ClusterPool;
use crate::job_machine::k8s_utils::stop_run_jobs;
use std::sync::Arc;
use stormchaser_model::run_cancel;
use tracing::{info, warn};
use uuid::Uuid;

/// Acts on a cancellation from the engine: deletes the run's Jobs now, and looks again
/// shortly for any that were still being created.
pub async fn handle_cancel(
    message: async_nats::jetstream::Message,
    cluster_pool: Arc<ClusterPool>,
    step_namespace: String,
) {
    let Some(event) = run_cancel::parse(&message) else {
        warn!("Ignoring a message on the cancel subject that is not a cancellation");
        return;
    };
    let run_id = event.run_id.into_inner();
    run_cancel::mark_cancelled(run_id);

    let stopped = stop_everywhere(&cluster_pool, &step_namespace, run_id).await;
    if stopped > 0 {
        info!(
            "Run {} was cancelled ({}): deleted {} Job(s)",
            run_id, event.reason, stopped
        );
    }
    for delay in run_cancel::RESWEEP_AFTER {
        tokio::time::sleep(delay).await;
        let late = stop_everywhere(&cluster_pool, &step_namespace, run_id).await;
        if late > 0 {
            info!(
                "Run {} was cancelled: deleted {} Job(s) that started late",
                run_id, late
            );
        }
    }
}

async fn stop_everywhere(cluster_pool: &ClusterPool, namespace: &str, run_id: Uuid) -> usize {
    let mut stopped = 0;
    for cluster_name in cluster_pool.cluster_names() {
        match cluster_pool.get_client(&cluster_name).await {
            Ok((client, _)) => stopped += stop_run_jobs(&client, namespace, run_id).await,
            Err(e) => warn!("No client for cluster {cluster_name} to cancel run {run_id}: {e}"),
        }
    }
    stopped
}
