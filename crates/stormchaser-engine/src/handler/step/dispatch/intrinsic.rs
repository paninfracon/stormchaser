use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;

use stormchaser_model::dsl;

/// Recursively searches for a step by name within a list of steps.
pub(crate) async fn apply_intrinsic_mutations(
    run_id: RunId,
    step_type: &mut String,
    resolved_spec: &mut Value,
    pool: &PgPool,
) -> Result<()> {
    crate::handler::step::intrinsic::git_checkout::mutate(step_type, resolved_spec, Some(pool))
        .await?;
    crate::handler::step::intrinsic::jq::mutate_if_has_files(step_type, resolved_spec);
    crate::handler::step::intrinsic::terraform::mutate_if_terraform(
        run_id.into_inner(),
        step_type,
        resolved_spec,
    )
    .await?;
    crate::handler::step::intrinsic::terraform::mutate_if_terraform_approval(
        step_type,
        resolved_spec,
    );

    // Inject generic connections
    if step_type == "RunContainer" {
        if let Ok(mut spec) =
            serde_json::from_value::<dsl::CommonContainerSpec>(resolved_spec.clone())
        {
            inject_connection_env_vars(&spec.connections, &mut spec.env, pool).await?;
            if let Ok(new_spec) = serde_json::to_value(spec) {
                *resolved_spec = new_spec;
            }
        }
    } else if step_type == "RunK8sJob" {
        if let Ok(mut spec) = serde_json::from_value::<dsl::K8sJobSpec>(resolved_spec.clone()) {
            inject_connection_env_vars(&spec.connections, &mut spec.env, pool).await?;
            if let Ok(new_spec) = serde_json::to_value(spec) {
                *resolved_spec = new_spec;
            }
        }
    }

    Ok(())
}

pub(crate) async fn inject_connection_env_vars(
    connections: &Option<Vec<String>>,
    env: &mut Option<Vec<dsl::EnvVar>>,
    pool: &PgPool,
) -> Result<()> {
    let Some(connection_names) = connections else {
        return Ok(());
    };

    let mut envs = env.clone().unwrap_or_default();
    for conn_name in connection_names {
        if let Some(conn) = crate::db::connections::get_storage_backend_by_name::<
            _,
            stormchaser_model::Connection,
        >(pool, conn_name)
        .await?
        {
            let prefix = format!(
                "STORMCHASER_CONN_{}_",
                conn_name.to_uppercase().replace("-", "_")
            );
            if let Some(url) = conn.config.get("url").and_then(|v| v.as_str()) {
                envs.push(dsl::EnvVar {
                    name: format!("{}URL", prefix),
                    value: url.to_string(),
                });
            }
            if let Some(user) = conn.config.get("username").and_then(|v| v.as_str()) {
                envs.push(dsl::EnvVar {
                    name: format!("{}USERNAME", prefix),
                    value: user.to_string(),
                });
            }
            if let Some(creds) = &conn.encrypted_credentials {
                envs.push(dsl::EnvVar {
                    name: format!("{}PASSWORD", prefix),
                    value: creds.to_string(),
                });
            }
        }
    }
    *env = Some(envs);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn try_dispatch_intrinsic(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    fencing_token: i64,
    step_type: &str,
    resolved_spec: &Value,
    resolved_params: &Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<bool> {
    if crate::handler::step::intrinsic::wasm::try_dispatch(
        run_id,
        step_instance_id,
        fencing_token,
        step_type,
        resolved_spec,
        resolved_params,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if crate::handler::step::intrinsic::lambda::try_dispatch(
        run_id,
        step_instance_id,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if crate::handler::step::intrinsic::rest_api::try_dispatch(
        run_id,
        step_instance_id,
        fencing_token,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if crate::handler::step::intrinsic::sql_execute::try_dispatch(
        run_id,
        step_instance_id,
        fencing_token,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if crate::handler::step::intrinsic::webhook::try_dispatch(
        run_id,
        step_instance_id,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if crate::handler::step::intrinsic::slack::try_dispatch(
        run_id,
        step_instance_id,
        fencing_token,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if crate::handler::step::intrinsic::teams::try_dispatch(
        run_id,
        step_instance_id,
        fencing_token,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if crate::handler::step::intrinsic::jinja::try_dispatch(
        run_id,
        step_instance_id,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if crate::handler::step::intrinsic::email::try_dispatch(
        run_id,
        step_instance_id,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if crate::handler::step::intrinsic::test_report_email::try_dispatch(
        run_id,
        step_instance_id,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    if crate::handler::step::intrinsic::jq::try_dispatch(
        run_id,
        step_instance_id,
        fencing_token,
        step_type,
        resolved_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(true);
    }
    Ok(false)
}
