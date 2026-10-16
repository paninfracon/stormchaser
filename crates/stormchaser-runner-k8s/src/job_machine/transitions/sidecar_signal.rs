//! Telling a step pod's agent sidecar that the step's own container has finished.
//!
//! The sidecar has no access to the Kubernetes API or to NATS. The runner, which has
//! both, watches the pod and reports the step container's exit code by setting a pod
//! annotation the sidecar sees through a downward API volume. If the step publishes
//! artifacts to a registry, the runner first asks the engine for push credentials and
//! leaves them in a Secret only the sidecar mounts.

use super::{job_owner_reference, k8s_utils, state, K8sJobMachine};
use futures::StreamExt;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::api::{Api, Patch, PatchParams, PostParams, WatchEvent, WatchParams};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::convert::Infallible;
use std::time::Duration;
use stormchaser_model::nats::request_artifact_auth;
use tokio::time::sleep;
use tracing::{info, warn};

/// The exit code to report to a pod's agent sidecar, if it is time to report one.
///
/// It is time once the step's container has terminated for good: with exit code 0, or
/// with any code when the pod never restarts containers. A failed container that the
/// kubelet is about to restart is not reported.
pub(crate) fn sidecar_signal(pod: &Pod) -> Option<i32> {
    let spec = pod.spec.as_ref()?;
    if !spec
        .containers
        .iter()
        .any(|c| c.name == k8s_utils::AGENT_SIDECAR_CONTAINER)
    {
        return None;
    }
    let already_signalled = pod
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(k8s_utils::WORKER_EXIT_ANNOTATION))
        .is_some_and(|v| !v.is_empty());
    if already_signalled {
        return None;
    }

    let exit_code = pod
        .status
        .as_ref()?
        .container_statuses
        .as_ref()?
        .iter()
        .find(|cs| cs.name == k8s_utils::WORKER_CONTAINER)?
        .state
        .as_ref()?
        .terminated
        .as_ref()?
        .exit_code;

    let restarts = spec.restart_policy.as_deref() != Some("Never");
    (exit_code == 0 || !restarts).then_some(exit_code)
}

/// True when the pod's sidecar is waiting for registry push credentials.
fn wants_push_auth(pod: &Pod, job_name: &str) -> bool {
    let secret_name = k8s_utils::push_auth_secret_name(job_name);
    pod.spec
        .as_ref()
        .and_then(|spec| spec.volumes.as_ref())
        .is_some_and(|volumes| {
            volumes.iter().any(|v| {
                v.secret
                    .as_ref()
                    .is_some_and(|s| s.secret_name.as_deref() == Some(secret_name.as_str()))
            })
        })
}

/// Builds the Secret holding a step's registry push credentials, owned by the step's Job.
pub(crate) fn build_push_auth_secret(
    job_name: &str,
    docker_config: &serde_json::Value,
    owner: Option<&Job>,
) -> Secret {
    let mut secret = Secret::default();
    secret.metadata.name = Some(k8s_utils::push_auth_secret_name(job_name));
    secret.metadata.labels = Some(BTreeMap::from([(
        "managed-by".to_string(),
        "stormchaser".to_string(),
    )]));
    secret.metadata.owner_references = owner.and_then(|job| job_owner_reference(job_name, job));
    secret.data = Some(BTreeMap::from([(
        k8s_utils::PUSH_AUTH_SECRET_KEY.to_string(),
        k8s_openapi::ByteString(
            serde_json::to_vec(docker_config).expect("a JSON value always serializes"),
        ),
    )]));
    secret
}

impl K8sJobMachine<state::Running> {
    /// Watches the job's pods and signals each agent sidecar once. Never returns; the
    /// caller drops it when the job is over.
    pub(crate) async fn signal_sidecars(&self) -> Infallible {
        let job_name = &self.state.job_name;
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.metadata.namespace);
        let selector = format!("job-name={}", job_name);
        let mut signalled = HashSet::new();

        loop {
            let wp = WatchParams::default().labels(&selector).timeout(60);
            match pods.watch(&wp, "0").await {
                Ok(stream) => {
                    let mut stream = stream.boxed();
                    while let Some(event) = stream.next().await {
                        match event {
                            Ok(WatchEvent::Added(pod)) | Ok(WatchEvent::Modified(pod)) => {
                                self.signal_if_finished(&pods, &pod, &mut signalled).await;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                warn!("Pod watch for job {} failed: {:?}", job_name, e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => warn!("Could not watch the pods of job {}: {:?}", job_name, e),
            }
            sleep(Duration::from_secs(2)).await;
        }
    }

    async fn signal_if_finished(
        &self,
        pods: &Api<Pod>,
        pod: &Pod,
        signalled: &mut HashSet<String>,
    ) {
        let Some(pod_name) = pod.metadata.name.clone() else {
            return;
        };
        let Some(exit_code) = sidecar_signal(pod) else {
            return;
        };
        if signalled.contains(&pod_name) {
            return;
        }
        let job_name = &self.state.job_name;

        // Credentials first, so they are in place when the sidecar wakes
        if exit_code == 0 && wants_push_auth(pod, job_name) {
            self.provide_push_auth().await;
        }

        let patch = json!({
            "metadata": {
                "annotations": { k8s_utils::WORKER_EXIT_ANNOTATION: exit_code.to_string() }
            }
        });
        match pods
            .patch(&pod_name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
        {
            Ok(_) => {
                info!(
                    "Told the agent sidecar of pod {} that its step exited with {}",
                    pod_name, exit_code
                );
                signalled.insert(pod_name);
            }
            // Retried on the pod's next event or the next watch
            Err(e) => warn!(
                "Could not signal the agent sidecar of pod {}: {:?}",
                pod_name, e
            ),
        }
    }

    /// Asks the engine for the step's registry push credentials and leaves them in the
    /// Secret the sidecar mounts.
    ///
    /// On failure an empty configuration is written instead, so that the sidecar does not
    /// sit waiting for credentials that will not come; its push then fails and is logged.
    async fn provide_push_auth(&self) {
        let job_name = &self.state.job_name;
        let docker_config = match &self.nats {
            Some(nats) => {
                request_artifact_auth(
                    nats,
                    self.metadata.run_id,
                    self.metadata.step_id,
                    self.metadata.runner_id.as_deref(),
                )
                .await
            }
            None => Err(anyhow::anyhow!(
                "the runner has no connection to the engine"
            )),
        }
        .unwrap_or_else(|e| {
            warn!(
                "No registry credentials for step {}; its OCI artifacts will not be pushed: {:#}",
                self.metadata.step_id, e
            );
            json!({ "auths": {} })
        });

        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.metadata.namespace);
        let owner = jobs.get_opt(job_name).await.ok().flatten();
        let secret = build_push_auth_secret(job_name, &docker_config, owner.as_ref());
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &self.metadata.namespace);
        match secrets.create(&PostParams::default(), &secret).await {
            Ok(_) => {}
            // Another pod of the same job already got them
            Err(kube::Error::Api(ref err)) if err.code == 409 => {}
            Err(e) => warn!(
                "Could not create the push credentials Secret for job {}: {:?}",
                job_name, e
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(
        with_sidecar: bool,
        restart_policy: &str,
        annotation: Option<&str>,
        worker_state: serde_json::Value,
    ) -> Pod {
        let mut containers = vec![json!({"name": "worker", "image": "alpine"})];
        if with_sidecar {
            containers.push(json!({"name": "stormchaser-agent", "image": "agent"}));
        }
        let mut annotations = serde_json::Map::new();
        if let Some(value) = annotation {
            annotations.insert(k8s_utils::WORKER_EXIT_ANNOTATION.to_string(), json!(value));
        }
        serde_json::from_value(json!({
            "metadata": {"name": "p", "annotations": annotations},
            "spec": {"containers": containers, "restartPolicy": restart_policy},
            "status": {"containerStatuses": [
                {"name": "stormchaser-agent", "ready": true, "restartCount": 0, "image": "", "imageID": "",
                 "state": {"running": {}}},
                {"name": "worker", "ready": false, "restartCount": 0, "image": "", "imageID": "",
                 "state": worker_state},
            ]}
        }))
        .unwrap()
    }

    fn terminated(code: i32) -> serde_json::Value {
        json!({"terminated": {"exitCode": code}})
    }

    #[test]
    fn a_finished_step_is_reported_to_its_sidecar() {
        assert_eq!(
            sidecar_signal(&pod(true, "OnFailure", Some(""), terminated(0))),
            Some(0)
        );
        assert_eq!(
            sidecar_signal(&pod(true, "Never", Some(""), terminated(0))),
            Some(0)
        );
        assert_eq!(
            sidecar_signal(&pod(true, "Never", None, terminated(3))),
            Some(3)
        );
    }

    #[test]
    fn a_step_that_is_still_going_is_not_reported() {
        assert_eq!(
            sidecar_signal(&pod(true, "Never", Some(""), json!({"running": {}}))),
            None
        );
        assert_eq!(
            sidecar_signal(&pod(
                true,
                "Never",
                Some(""),
                json!({"waiting": {"reason": "ErrImagePull"}})
            )),
            None
        );
        // The kubelet restarts a failed container under this policy
        assert_eq!(
            sidecar_signal(&pod(true, "OnFailure", Some(""), terminated(1))),
            None
        );
    }

    #[test]
    fn a_pod_is_reported_once_and_only_if_it_has_a_sidecar() {
        assert_eq!(
            sidecar_signal(&pod(true, "Never", Some("0"), terminated(0))),
            None
        );
        assert_eq!(
            sidecar_signal(&pod(false, "Never", Some(""), terminated(0))),
            None
        );
    }

    #[test]
    fn push_credentials_secret_is_owned_by_the_job() {
        let mut job = Job::default();
        job.metadata.uid = Some("uid-1".to_string());
        let config = json!({"auths": {"r.example.com": {"registrytoken": "t"}}});
        let secret = build_push_auth_secret("storm-build-1234", &config, Some(&job));

        assert_eq!(
            secret.metadata.name.as_deref(),
            Some("storm-build-1234-push-auth")
        );
        assert_eq!(secret.metadata.owner_references.unwrap()[0].uid, "uid-1");
        let data = secret.data.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&data["config.json"].0).unwrap(),
            config
        );
    }
}
