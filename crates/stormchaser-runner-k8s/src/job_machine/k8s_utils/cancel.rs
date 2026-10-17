use k8s_openapi::api::batch::v1::Job;
use kube::api::{Api, DeleteParams, ListParams, PropagationPolicy};
use kube::ResourceExt;
use tracing::warn;
use uuid::Uuid;

/// Deletes every Job this runner started for a run, and with them their pods and the
/// Secrets they own. Returns how many were deleted.
///
/// The Jobs are found by the run id label the runner sets itself, which a step's own
/// labels cannot replace, and only in the namespace the runner starts steps in.
pub async fn stop_run_jobs(client: &kube::Client, namespace: &str, run_id: Uuid) -> usize {
    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
    let selector = stormchaser_model::run_cancel::run_label_selector(run_id);
    let list = match jobs.list(&ListParams::default().labels(&selector)).await {
        Ok(list) => list,
        Err(e) => {
            warn!("Could not list the Jobs of cancelled run {run_id}: {e}");
            return 0;
        }
    };

    let delete = DeleteParams {
        propagation_policy: Some(PropagationPolicy::Background),
        ..Default::default()
    };
    let mut stopped = 0;
    for job in list {
        let name = job.name_any();
        match jobs.delete(&name, &delete).await {
            Ok(_) => stopped += 1,
            Err(kube::Error::Api(e)) if e.code == 404 => {}
            Err(e) => warn!("Could not delete Job {name} of cancelled run {run_id}: {e}"),
        }
    }
    stopped
}
