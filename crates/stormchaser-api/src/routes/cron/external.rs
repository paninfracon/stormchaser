use axum::http::StatusCode;
use k8s_openapi::api::batch::v1::{CronJob, CronJobSpec, JobSpec, JobTemplateSpec};
use k8s_openapi::api::core::v1::{
    Capabilities, Container, PodSpec, PodTemplateSpec, Secret, SecretVolumeSource, SecurityContext,
    Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use kube::{
    api::{DeleteParams, PostParams},
    Api, Client,
};
use std::env;
use stormchaser_model::CronWorkflowId;

/// Where cron schedules are registered (`CRON_ENGINE`): `kubernetes`, which creates
/// a CronJob per schedule, or `none`, which only stores the schedule and its token
/// for a scheduler of the operator's own to call the trigger URL with.
///
/// `ofelia` was accepted until this release, but Ofelia only reads `job-run` jobs from
/// its own container, so the labels it was given were never scheduled; they only kept
/// the trigger token where anything that lists containers could read it.
fn cron_engine() -> Result<String, StatusCode> {
    let engine = env::var("CRON_ENGINE").unwrap_or_else(|_| "kubernetes".to_string());
    match engine.as_str() {
        "kubernetes" | "none" => Ok(engine),
        other => {
            tracing::error!(
                "Unsupported CRON_ENGINE {:?}: use `kubernetes`, or `none` and a scheduler of your own",
                other
            );
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub(crate) async fn register_external_cron(
    id: CronWorkflowId,
    name: &str,
    cronspec: &str,
    secret_token: &str,
) -> Result<Option<String>, StatusCode> {
    if cron_engine()? == "none" {
        tracing::info!(
            "Cron engine set to 'none', skipping external registration for {}",
            id
        );
        return Ok(None);
    }

    let client = Client::try_default().await.map_err(|e| {
        tracing::error!("Failed to initialize K8s client for cron: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let namespace = env::var("KUBERNETES_NAMESPACE").unwrap_or_else(|_| "default".to_string());
    let cronjobs: Api<CronJob> = Api::namespaced(client, &namespace);

    let system_url = env::var("SYSTEM_URL")
        .unwrap_or_else(|_| "http://stormchaser-api.default.svc.cluster.local".to_string());
    let trigger_url = format!("{}/api/v1/cron-trigger/{}", system_url, id);
    // A private CA the trigger must trust to reach an API that serves TLS itself
    let ca_pem = match env::var("STORMCHASER_CRON_CA_FILE") {
        Ok(path) if !path.is_empty() => Some(std::fs::read_to_string(&path).map_err(|e| {
            tracing::error!("Failed to read STORMCHASER_CRON_CA_FILE {}: {:?}", path, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?),
        _ => None,
    };

    let job_name = format!("stormchaser-{}", id);
    let created = cronjobs
        .create(
            &PostParams::default(),
            &trigger_cron_job(
                &job_name,
                name,
                cronspec,
                &trigger_url,
                &cron_image(),
                ca_pem.is_some(),
            ),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create K8s CronJob: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // The token goes in a Secret the CronJob owns, so it is deleted with it. Until the
    // Secret exists a pod of the CronJob cannot start, so none runs without it.
    let secrets: Api<Secret> = Api::namespaced(cronjobs.clone().into_client(), &namespace);
    let secret = trigger_secret(&job_name, &created, secret_token, ca_pem.as_deref());
    if let Err(e) = secrets.create(&PostParams::default(), &secret).await {
        tracing::error!("Failed to create the Secret of K8s CronJob: {:?}", e);
        let _ = cronjobs.delete(&job_name, &DeleteParams::default()).await;
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Some(job_name))
}

/// The image that calls the trigger URL. Pinned by digest so that what runs with the
/// trigger token is the image that was reviewed, not whatever a tag points at today.
const DEFAULT_CRON_IMAGE: &str = "curlimages/curl:8.14.1@sha256:9a1ed35addb45476afa911696297f8e115993df459278ed036182dd2cd22b67b";

fn cron_image() -> String {
    env::var("STORMCHASER_CRON_IMAGE")
        .ok()
        .filter(|image| !image.is_empty())
        .unwrap_or_else(|| DEFAULT_CRON_IMAGE.to_string())
}

/// The unprivileged user the trigger runs as (`curl_user` in the curl image).
const TRIGGER_UID: i64 = 100;

/// Where the trigger's Secret is mounted in its pod.
const TRIGGER_SECRET_DIR: &str = "/var/run/stormchaser-trigger";

fn trigger_secret_name(job_name: &str) -> String {
    format!("{job_name}-trigger")
}

/// The CronJob that calls a cron workflow's trigger URL.
///
/// The bearer token is not in it. `kubectl get` or `describe` on a CronJob, its Jobs or
/// its pods shows their arguments to anyone who may read workloads, so curl is told to
/// read the header from a file that comes from a Secret.
fn trigger_cron_job(
    job_name: &str,
    workflow_name: &str,
    cronspec: &str,
    trigger_url: &str,
    image: &str,
    with_ca: bool,
) -> CronJob {
    let mut command = vec![
        "curl".to_string(),
        "-X".to_string(),
        "POST".to_string(),
        "-H".to_string(),
        format!("@{TRIGGER_SECRET_DIR}/header"),
    ];
    if with_ca {
        command.push("--cacert".to_string());
        command.push(format!("{TRIGGER_SECRET_DIR}/ca.crt"));
    }
    command.push(trigger_url.to_string());

    CronJob {
        metadata: ObjectMeta {
            name: Some(job_name.to_string()),
            labels: Some(std::collections::BTreeMap::from([
                (
                    "app.kubernetes.io/managed-by".to_string(),
                    "stormchaser".to_string(),
                ),
                (
                    "stormchaser.paninfracon.net/workflow-name".to_string(),
                    workflow_name.to_string(),
                ),
            ])),
            ..Default::default()
        },
        spec: Some(CronJobSpec {
            schedule: cronspec.to_string(),
            job_template: JobTemplateSpec {
                metadata: None,
                spec: Some(JobSpec {
                    template: PodTemplateSpec {
                        spec: Some(PodSpec {
                            containers: vec![Container {
                                name: "trigger".to_string(),
                                image: Some(image.to_string()),
                                command: Some(command),
                                volume_mounts: Some(vec![VolumeMount {
                                    name: "trigger".to_string(),
                                    mount_path: TRIGGER_SECRET_DIR.to_string(),
                                    read_only: Some(true),
                                    ..Default::default()
                                }]),
                                security_context: Some(SecurityContext {
                                    allow_privilege_escalation: Some(false),
                                    read_only_root_filesystem: Some(true),
                                    run_as_non_root: Some(true),
                                    // The curl image names its user rather than
                                    // numbering it, and Kubernetes will not start a
                                    // non-root container whose user it cannot check
                                    run_as_user: Some(TRIGGER_UID),
                                    capabilities: Some(Capabilities {
                                        drop: Some(vec!["ALL".to_string()]),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            }],
                            volumes: Some(vec![Volume {
                                name: "trigger".to_string(),
                                secret: Some(SecretVolumeSource {
                                    secret_name: Some(trigger_secret_name(job_name)),
                                    // Readable by the image's unprivileged user only
                                    default_mode: Some(0o444),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            }]),
                            // The trigger calls one URL and has no business with the cluster
                            automount_service_account_token: Some(false),
                            restart_policy: Some("OnFailure".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The Secret holding the trigger's `Authorization` header, owned by its CronJob.
fn trigger_secret(job_name: &str, owner: &CronJob, token: &str, ca_pem: Option<&str>) -> Secret {
    let mut data = std::collections::BTreeMap::from([(
        "header".to_string(),
        format!("Authorization: Bearer {token}"),
    )]);
    if let Some(ca) = ca_pem {
        data.insert("ca.crt".to_string(), ca.to_string());
    }
    Secret {
        metadata: ObjectMeta {
            name: Some(trigger_secret_name(job_name)),
            labels: Some(std::collections::BTreeMap::from([(
                "app.kubernetes.io/managed-by".to_string(),
                "stormchaser".to_string(),
            )])),
            owner_references: owner.metadata.uid.as_ref().map(|uid| {
                vec![OwnerReference {
                    api_version: "batch/v1".to_string(),
                    kind: "CronJob".to_string(),
                    name: job_name.to_string(),
                    uid: uid.clone(),
                    block_owner_deletion: Some(false),
                    controller: Some(true),
                }]
            }),
            ..Default::default()
        },
        string_data: Some(data),
        type_: Some("Opaque".to_string()),
        ..Default::default()
    }
}

pub(crate) async fn unregister_external_cron(external_job_id: &str) -> Result<(), StatusCode> {
    if cron_engine()? == "none" {
        return Ok(());
    }
    let client = Client::try_default()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let namespace = env::var("KUBERNETES_NAMESPACE").unwrap_or_else(|_| "default".to_string());
    let cronjobs: Api<CronJob> = Api::namespaced(client, &namespace);

    cronjobs
        .delete(external_job_id, &DeleteParams::default())
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete K8s CronJob {}: {:?}", external_job_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trigger_token_is_in_the_secret_and_nowhere_in_the_cron_job() {
        let token = "tok-5ecret-value";
        let job = trigger_cron_job(
            "stormchaser-abc",
            "nightly",
            "0 2 * * *",
            "https://api.example/api/v1/cron-trigger/abc",
            DEFAULT_CRON_IMAGE,
            false,
        );
        let rendered = serde_json::to_string(&job).unwrap();
        assert!(!rendered.contains(token));
        assert!(!rendered.contains("Bearer"));
        // curl reads the header from the mounted file instead
        assert!(rendered.contains("@/var/run/stormchaser-trigger/header"));
        assert!(rendered.contains("stormchaser-abc-trigger"));
        assert!(!rendered.contains("--cacert"));

        let mut owner = job.clone();
        owner.metadata.uid = Some("uid-1".to_string());
        let secret = trigger_secret("stormchaser-abc", &owner, token, None);
        let data = secret.string_data.unwrap();
        assert_eq!(data["header"], format!("Authorization: Bearer {token}"));
        assert!(!data.contains_key("ca.crt"));
        // Deleting the CronJob takes the Secret with it
        let owners = secret.metadata.owner_references.unwrap();
        assert_eq!(owners[0].kind, "CronJob");
        assert_eq!(owners[0].uid, "uid-1");
    }

    #[test]
    fn a_private_ca_reaches_the_trigger_through_the_same_secret() {
        let job = trigger_cron_job(
            "stormchaser-abc",
            "nightly",
            "0 2 * * *",
            "https://x",
            "img",
            true,
        );
        let command = job
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .containers[0]
            .command
            .clone()
            .unwrap();
        let at = command.iter().position(|arg| arg == "--cacert").unwrap();
        assert_eq!(command[at + 1], "/var/run/stormchaser-trigger/ca.crt");
        // The URL stays last
        assert_eq!(command.last().unwrap(), "https://x");

        let secret = trigger_secret("stormchaser-abc", &CronJob::default(), "t", Some("PEM"));
        assert_eq!(secret.string_data.unwrap()["ca.crt"], "PEM");
    }

    #[test]
    fn the_trigger_image_is_pinned_by_digest_and_runs_unprivileged() {
        assert!(DEFAULT_CRON_IMAGE.contains("@sha256:"));
        assert!(!DEFAULT_CRON_IMAGE.contains(":latest"));
        let job = trigger_cron_job(
            "j",
            "n",
            "* * * * *",
            "https://x",
            DEFAULT_CRON_IMAGE,
            false,
        );
        let pod = job
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap();
        assert_eq!(pod.automount_service_account_token, Some(false));
        let security = pod.containers[0].security_context.clone().unwrap();
        assert_eq!(security.run_as_non_root, Some(true));
        assert_eq!(security.run_as_user, Some(100));
        assert_eq!(security.allow_privilege_escalation, Some(false));
    }

    /// Creates the CronJob and its Secret on a real cluster, runs the trigger once and
    /// checks that the token reached the server and shows nowhere in the workload.
    ///
    /// Ignored by default. Needs `KUBECONFIG`, `STORMCHASER_E2E_NAMESPACE` and
    /// `STORMCHASER_E2E_TRIGGER_URL`, the base URL of a server the pods can reach that
    /// answers a POST with `auth=<the Authorization header it received>`.
    #[tokio::test]
    #[ignore = "needs a Kubernetes cluster and a server to call"]
    async fn the_trigger_runs_locked_down_on_a_cluster_and_delivers_its_token() {
        use k8s_openapi::api::batch::v1::Job;
        use k8s_openapi::api::core::v1::Pod;
        use kube::api::{ListParams, LogParams};
        use std::time::Duration;

        let _ = rustls::crypto::ring::default_provider().install_default();
        let namespace = env::var("STORMCHASER_E2E_NAMESPACE").unwrap();
        let base = env::var("STORMCHASER_E2E_TRIGGER_URL").unwrap();
        let client = Client::try_default().await.unwrap();
        let cronjobs: Api<CronJob> = Api::namespaced(client.clone(), &namespace);
        let secrets: Api<Secret> = Api::namespaced(client.clone(), &namespace);
        let jobs: Api<Job> = Api::namespaced(client.clone(), &namespace);
        let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);

        let name = format!("stormchaser-{}", uuid::Uuid::new_v4());
        let token = format!("tok-{}", uuid::Uuid::new_v4().simple());
        let cron_job = trigger_cron_job(
            &name,
            "nightly",
            "0 0 1 1 *",
            &format!("{base}/api/v1/cron-trigger/abc"),
            &cron_image(),
            false,
        );
        let created = cronjobs
            .create(&PostParams::default(), &cron_job)
            .await
            .unwrap();
        secrets
            .create(
                &PostParams::default(),
                &trigger_secret(&name, &created, &token, None),
            )
            .await
            .unwrap();

        // Run it now rather than waiting for the schedule
        let template = created.spec.unwrap().job_template;
        let job = Job {
            metadata: ObjectMeta {
                name: Some(format!("{name}-now")),
                ..Default::default()
            },
            spec: template.spec,
            ..Default::default()
        };
        jobs.create(&PostParams::default(), &job).await.unwrap();

        let selector = ListParams::default().labels(&format!("job-name={name}-now"));
        let mut answer = String::new();
        for _ in 0..180 {
            let done = jobs
                .get(&format!("{name}-now"))
                .await
                .unwrap()
                .status
                .and_then(|status| status.succeeded)
                .unwrap_or(0)
                > 0;
            if done {
                let pod = pods.list(&selector).await.unwrap().items.remove(0);
                let pod_name = pod.metadata.name.clone().unwrap();
                answer = pods.logs(&pod_name, &LogParams::default()).await.unwrap();
                // Nothing a reader of workloads can see holds the token
                let visible = format!(
                    "{}{}",
                    serde_json::to_string(&pod).unwrap(),
                    serde_json::to_string(&cronjobs.get(&name).await.unwrap()).unwrap()
                );
                assert!(!visible.contains(&token));
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        assert!(
            answer.contains(&format!("auth=Bearer {token}")),
            "the trigger did not deliver its token: {answer:?}"
        );

        // Deleting the CronJob takes the Secret with it
        cronjobs
            .delete(&name, &DeleteParams::background())
            .await
            .unwrap();
        for _ in 0..120 {
            if secrets
                .get_opt(&trigger_secret_name(&name))
                .await
                .unwrap()
                .is_none()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        panic!("the trigger's Secret outlived its CronJob");
    }
}
