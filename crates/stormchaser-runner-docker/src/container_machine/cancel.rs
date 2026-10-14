use bollard::query_parameters::{
    ListContainersOptions, RemoveContainerOptions, StopContainerOptions,
};
use bollard::Docker;
use std::collections::HashMap;
use tracing::warn;
use uuid::Uuid;

/// Seconds a container gets to exit after SIGTERM before it is killed.
const STOP_GRACE_SECS: i32 = 10;

/// Stops and removes every container this runner started for a run, including a shared
/// one that several steps ran in. Returns how many were stopped.
pub async fn stop_run_containers(docker: &Docker, run_id: Uuid) -> usize {
    let filters = HashMap::from([(
        "label".to_string(),
        vec![
            "managed-by=stormchaser".to_string(),
            format!("stormchaser-run-id={run_id}"),
        ],
    )]);
    let containers = match docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters: Some(filters),
            ..Default::default()
        }))
        .await
    {
        Ok(containers) => containers,
        Err(e) => {
            warn!("Could not list the containers of cancelled run {run_id}: {e}");
            return 0;
        }
    };

    // Together, so a run with many containers takes one grace period, not one each
    let outcomes = futures::future::join_all(
        containers
            .into_iter()
            .filter_map(|container| container.id)
            .map(|id| stop_and_remove(docker, id, run_id)),
    )
    .await;
    outcomes.into_iter().filter(|stopped| *stopped).count()
}

async fn stop_and_remove(docker: &Docker, id: String, run_id: Uuid) -> bool {
    let _ = docker
        .stop_container(
            &id,
            Some(StopContainerOptions {
                t: Some(STOP_GRACE_SECS),
                ..Default::default()
            }),
        )
        .await;
    match docker
        .remove_container(
            &id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
    {
        Ok(()) => true,
        // Already removed by the step's own handler, which saw the container stop
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404 | 409,
            ..
        }) => true,
        Err(e) => {
            warn!("Could not remove container {id} of cancelled run {run_id}: {e}");
            false
        }
    }
}
