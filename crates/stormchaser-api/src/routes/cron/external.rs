use axum::http::StatusCode;
use k8s_openapi::api::batch::v1::{CronJob, CronJobSpec, JobSpec, JobTemplateSpec};
use k8s_openapi::api::core::v1::{Container, PodSpec, PodTemplateSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{
    api::{DeleteParams, PostParams},
    Api, Client,
};
use std::collections::HashMap;
use std::env;
use stormchaser_model::CronWorkflowId;

async fn register_ofelia_cron(
    id: CronWorkflowId,
    name: &str,
    cronspec: &str,
    secret_token: &str,
) -> Result<Option<String>, StatusCode> {
    use bollard::container::{Config, CreateContainerOptions};
    use bollard::Docker;

    let docker = Docker::connect_with_local_defaults().map_err(|e| {
        tracing::error!("Failed to connect to Docker: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let system_url = env::var("SYSTEM_URL").unwrap_or_else(|_| "http://localhost:3000".to_string());
    let trigger_url = format!("{}/api/v1/cron-trigger/{}", system_url, id);
    let container_name = format!("stormchaser-cron-{}", id);

    let mut labels = HashMap::new();
    labels.insert("ofelia.enabled".to_string(), "true".to_string());
    labels.insert(
        format!("ofelia.job-run.{}.schedule", name),
        cronspec.to_string(),
    );
    labels.insert(
        format!("ofelia.job-run.{}.image", name),
        "curlimages/curl:latest".to_string(),
    );
    labels.insert(
        format!("ofelia.job-run.{}.command", name),
        format!(
            "-X POST -H \"Authorization: Bearer {}\" {}",
            secret_token, trigger_url
        ),
    );

    let config = Config {
        image: Some("curlimages/curl:latest".to_string()),
        labels: Some(labels),
        entrypoint: Some(vec!["/bin/true".to_string()]),
        ..Default::default()
    };

    docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                ..Default::default()
            }),
            config,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create Ofelia cron container: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Some(container_name))
}

async fn unregister_ofelia_cron(container_name: &str) -> Result<(), StatusCode> {
    use bollard::container::RemoveContainerOptions;
    use bollard::Docker;

    let docker =
        Docker::connect_with_local_defaults().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    docker
        .remove_container(
            container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to remove Ofelia cron container: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(())
}

pub(crate) async fn register_external_cron(
    id: CronWorkflowId,
    name: &str,
    cronspec: &str,
    secret_token: &str,
) -> Result<Option<String>, StatusCode> {
    let engine = env::var("CRON_ENGINE").unwrap_or_else(|_| "kubernetes".to_string());

    if engine == "none" {
        tracing::info!(
            "Cron engine set to 'none', skipping external registration for {}",
            id
        );
        return Ok(None);
    }

    if engine == "ofelia" {
        return register_ofelia_cron(id, name, cronspec, secret_token).await;
    }

    if engine != "kubernetes" {
        tracing::error!("Unsupported CRON_ENGINE: {}", engine);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
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
    let auth_header = format!("Authorization: Bearer {}", secret_token);

    let cron_job = CronJob {
        metadata: ObjectMeta {
            name: Some(format!("stormchaser-{}", id)),
            labels: Some(std::collections::BTreeMap::from([
                (
                    "app.kubernetes.io/managed-by".to_string(),
                    "stormchaser".to_string(),
                ),
                (
                    "stormchaser.paninfracon.net/workflow-name".to_string(),
                    name.to_string(),
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
                                image: Some("curlimages/curl:latest".to_string()),
                                command: Some(vec![
                                    "curl".to_string(),
                                    "-X".to_string(),
                                    "POST".to_string(),
                                    "-H".to_string(),
                                    auth_header,
                                    trigger_url,
                                ]),
                                ..Default::default()
                            }],
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
    };

    cronjobs
        .create(&PostParams::default(), &cron_job)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create K8s CronJob: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Some(format!("stormchaser-{}", id)))
}

pub(crate) async fn unregister_external_cron(external_job_id: &str) -> Result<(), StatusCode> {
    let engine = env::var("CRON_ENGINE").unwrap_or_else(|_| "kubernetes".to_string());

    if engine == "none" {
        return Ok(());
    }

    if engine == "ofelia" {
        return unregister_ofelia_cron(external_job_id).await;
    }

    if engine != "kubernetes" {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
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
