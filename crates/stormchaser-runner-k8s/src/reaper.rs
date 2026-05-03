use anyhow::Result;
use chrono::DateTime;
use k8s_openapi::api::batch::v1::Job;
use kube::{
    api::{Api, DeleteParams, ListParams, PropagationPolicy},
    ResourceExt,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{info, warn};

use crate::cluster::ClusterPool;

pub async fn run_reaper(cluster_pool: Arc<ClusterPool>) -> Result<()> {
    let mut interval = time::interval(Duration::from_secs(3600)); // Check every hour
    loop {
        interval.tick().await;
        info!("Running garbage collection (reaper) on all clusters...");

        for cluster_name in cluster_pool.cluster_names() {
            if let Ok((client, _)) = cluster_pool.get_client(&cluster_name).await {
                let jobs: Api<Job> = Api::all(client.clone());
                let lp = ListParams::default().labels("managed-by=stormchaser");

                if let Ok(job_list) = jobs.list(&lp).await {
                    for job in job_list {
                        let now = chrono::Utc::now();
                        let creation_time = job.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                        if let Some(created) = creation_time {
                            let Some(created_chrono) =
                                DateTime::from_timestamp(created.as_second(), 0)
                            else {
                                warn!(
                                    "Job {} has an unconvertible creation timestamp; skipping reap check",
                                    job.name_any()
                                );
                                continue;
                            };
                            let age = now - created_chrono;
                            // If job is older than 24 hours, clean it up
                            if age.num_hours() >= 24 {
                                let job_name = job.name_any();
                                let namespace =
                                    job.namespace().unwrap_or_else(|| "default".to_string());
                                info!(
                                    "Reaping old orphaned job {} in namespace {} (age: {}h)",
                                    job_name,
                                    namespace,
                                    age.num_hours()
                                );

                                let dp = DeleteParams {
                                    propagation_policy: Some(PropagationPolicy::Background),
                                    ..Default::default()
                                };
                                let _ = Api::<Job>::namespaced(client.clone(), &namespace)
                                    .delete(&job_name, &dp)
                                    .await;
                            }
                        }
                    }
                }
            }
        }
    }
}
