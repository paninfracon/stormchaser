use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};
use sqlx::PgPool;
use std::collections::HashSet;
use stormchaser_dsl::ast::Workflow;
use stormchaser_model::connections::{Connection, ConnectionType};
use stormchaser_model::step::StepStatus;
use stormchaser_model::{RunId, StepInstance, StepInstanceId};

use crate::registry_token::{
    basic_docker_config, mint_push_token, static_credentials_allowed, token_docker_config,
    RegistryLocation, TokenError,
};

pub use stormchaser_model::nats::ARTIFACT_AUTH_SUBJECT;

/// Answers a runner that is about to push a step's OCI artifacts.
///
/// The reply is a Docker client configuration holding a token for the run's own
/// repositories. It goes to the request's reply inbox only, which the stream does not
/// capture.
pub async fn handle_artifact_auth_request(
    payload: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    reply: Option<String>,
) -> Result<()> {
    let Some(reply) = reply else {
        return Ok(());
    };

    let response = match artifact_push_auth(&payload, &pool).await {
        Ok(docker_config) => json!({ "ok": true, "docker_config": docker_config }),
        Err(e) => {
            tracing::warn!("Refusing artifact registry credentials: {:#}", e);
            json!({ "ok": false, "error": e.to_string() })
        }
    };
    nats_client
        .publish(reply, serde_json::to_vec(&response)?.into())
        .await?;
    Ok(())
}

/// Names of the storage volumes a step mounts, from its resolved spec.
fn mounted_storage_names(spec: &Value) -> HashSet<String> {
    spec.get("storage_mounts")
        .and_then(|m| m.as_array())
        .into_iter()
        .flatten()
        .filter_map(|mount| mount.get("name").and_then(|n| n.as_str()))
        .map(str::to_owned)
        .collect()
}

/// Builds the Docker client configuration a step's helper needs to push its artifacts.
pub async fn artifact_push_auth(payload: &Value, pool: &PgPool) -> Result<Value> {
    let run_id: RunId = serde_json::from_value(payload["run_id"].clone()).context("run_id")?;
    let step_id: StepInstanceId =
        serde_json::from_value(payload["step_id"].clone()).context("step_id")?;

    let instance: StepInstance = crate::db::get_step_instance_by_id(pool, step_id)
        .await?
        .ok_or_else(|| anyhow!("step {} does not exist", step_id))?;
    if instance.run_id != run_id {
        bail!("step {} does not belong to run {}", step_id, run_id);
    }
    // Only a step that is with a runner right now pushes artifacts
    if !matches!(
        instance.status,
        StepStatus::Initializing
            | StepStatus::UnpackingSfs
            | StepStatus::Running
            | StepStatus::PackingSfs
    ) || crate::db::step_completion_token(pool, step_id)
        .await
        .is_some()
    {
        bail!("step {} is not running on a runner", step_id);
    }

    let context = crate::handler::fetch_run_context(run_id, pool).await?;
    let workflow: Workflow = serde_json::from_value(context.workflow_definition)
        .context("Failed to parse workflow definition from DB")?;
    let mounted = mounted_storage_names(&instance.spec);

    let mut auths = Map::new();
    let mut seen_backends = HashSet::new();
    for storage in &workflow.storage {
        if storage.artifacts.is_empty() || !mounted.contains(&storage.name) {
            continue;
        }
        let backend: Option<Connection> = match &storage.backend {
            Some(name) => crate::db::get_storage_backend_by_name(pool, name).await?,
            None => crate::db::get_default_sfs_backend(pool).await?,
        };
        let Some(backend) = backend else { continue };
        if backend.connection_type != ConnectionType::Oci || !seen_backends.insert(backend.id) {
            continue;
        }
        let (Some(username), Some(password)) = (
            backend.config["username"].as_str(),
            backend.config["password"].as_str(),
        ) else {
            continue;
        };

        // One token for every repository of this run on this registry
        let location = RegistryLocation::of(&backend)?;
        let repositories: Vec<String> = workflow
            .storage
            .iter()
            .filter(|s| {
                !s.artifacts.is_empty() && mounted.contains(&s.name) && s.backend == storage.backend
            })
            .map(|s| location.artifact_repository(run_id.into_inner(), &s.name))
            .collect();

        let config = match mint_push_token(&location, username, password, &repositories).await {
            Ok(token) => token_docker_config(&location.host, &token),
            Err(TokenError::NotSupported) if static_credentials_allowed() => {
                tracing::warn!(
                    "Registry {} cannot issue scoped tokens; sending its stored credentials to the runner's helper for step {}",
                    location.host,
                    step_id
                );
                basic_docker_config(&location.host, username, password)
            }
            Err(e) => bail!(
                "no push credentials for registry {} (backend '{}'): {}",
                location.host,
                backend.name,
                e
            ),
        };
        if let Some(Value::Object(entries)) = config.get("auths") {
            auths.extend(entries.clone());
        }
    }

    Ok(json!({ "auths": auths }))
}

#[cfg(test)]
mod tests {
    use super::mounted_storage_names;
    use serde_json::json;

    #[test]
    fn reads_mounted_storage_names_from_a_spec() {
        let spec =
            json!({"storage_mounts": [{"name": "ws", "mount_path": "/w"}, {"name": "cache"}, {}]});
        let names = mounted_storage_names(&spec);
        assert!(names.contains("ws") && names.contains("cache"));
        assert_eq!(names.len(), 2);
        assert!(mounted_storage_names(&json!({})).is_empty());
    }
}
