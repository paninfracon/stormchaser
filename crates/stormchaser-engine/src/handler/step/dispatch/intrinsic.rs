use anyhow::Result;
use futures::future::{BoxFuture, FutureExt};

use crate::handler::integrations::utils::report_step_failed;
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
    crate::handler::step::intrinsic::git_checkout::mutate(
        step_type,
        resolved_spec,
        Some(pool),
        Some(run_id),
    )
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
            inject_connection_env_vars(&spec.connections, &mut spec.env, pool, run_id).await?;
            if let Ok(new_spec) = serde_json::to_value(spec) {
                *resolved_spec = new_spec;
            }
        }
    } else if step_type == "RunK8sJob" {
        if let Ok(mut spec) = serde_json::from_value::<dsl::K8sJobSpec>(resolved_spec.clone()) {
            inject_connection_env_vars(&spec.connections, &mut spec.env, pool, run_id).await?;
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
    run_id: RunId,
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
            crate::authz::ensure_run_may_use(pool, run_id, &conn).await?;
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
                // The step can print it; the API masks it wherever it turns up
                crate::db::add_sensitive_values(pool, run_id, std::slice::from_ref(creds)).await?;
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

/// A claimed step, with what the engine needs to run it itself if it is a built-in one.
#[derive(Clone)]
pub struct BuiltInStep {
    pub run_id: RunId,
    pub step_id: StepInstanceId,
    pub fencing_token: i64,
    pub step_type: String,
    pub spec: Value,
    pub params: Value,
    pub pool: PgPool,
    pub tls_reloader: Arc<TlsReloader>,
}

/// A built-in step's work, which reports the step's result itself and returns an error
/// only when it could not
pub(crate) type BuiltInWork = BoxFuture<'static, Result<()>>;

/// Work for a built-in step type that this engine build leaves out
#[cfg(not(all(feature = "chatops-slack", feature = "chatops-teams")))]
fn not_enabled(feature: &'static str) -> BuiltInWork {
    async move { anyhow::bail!("{feature} support is not enabled in this engine build.") }.boxed()
}

/// The work of `step`, if it is a built-in step.
async fn built_in_work(step: &BuiltInStep) -> Result<Option<BuiltInWork>> {
    use crate::handler::step::intrinsic::{jq, rest_api, sql_execute, wasm};
    let s = step.clone();
    let work = match step.step_type.as_str() {
        "RestApi" => {
            rest_api::handle_rest_api_invoke(s.run_id, s.step_id, s.fencing_token, s.spec, s.pool)
                .boxed()
        }
        "SqlExecute" => {
            sql_execute::handle_sql_execute(s.run_id, s.step_id, s.fencing_token, s.spec, s.pool)
                .boxed()
        }
        "JQ" => jq::run_jq(s.run_id, s.step_id, s.fencing_token, s.spec, s.pool).boxed(),
        "JinjaRender" => {
            crate::handler::handle_jinja_render(s.run_id, s.step_id, s.spec, s.pool).boxed()
        }
        "WebhookInvoke" => {
            crate::handler::handle_webhook_invoke(s.run_id, s.step_id, s.spec, s.pool).boxed()
        }
        "TestReportEmail" => crate::handler::handle_test_report_email(
            s.run_id,
            s.step_id,
            s.spec,
            s.pool,
            s.tls_reloader,
        )
        .boxed(),
        // Email and Lambda stand in for themselves in builds without them
        "EmailSend" => {
            crate::handler::handle_email_send(s.run_id, s.step_id, s.spec, s.pool, s.tls_reloader)
                .boxed()
        }
        "LambdaInvoke" => {
            crate::handler::handle_lambda_invoke(s.run_id, s.step_id, s.spec, s.pool).boxed()
        }
        #[cfg(feature = "chatops-slack")]
        "SlackMessage" => crate::handler::handle_slack_message(
            s.run_id,
            s.step_id,
            s.fencing_token,
            s.spec,
            s.pool,
        )
        .boxed(),
        #[cfg(not(feature = "chatops-slack"))]
        "SlackMessage" => not_enabled("ChatOps Slack"),
        #[cfg(feature = "chatops-teams")]
        "TeamsMessage" => crate::handler::handle_teams_message(
            s.run_id,
            s.step_id,
            s.fencing_token,
            s.spec,
            s.pool,
        )
        .boxed(),
        #[cfg(not(feature = "chatops-teams"))]
        "TeamsMessage" => not_enabled("ChatOps Teams"),
        // A Wasm step, or a step type registered with a WASM module
        _ => return wasm::work(step).await,
    };
    Ok(Some(work))
}

/// Starts `step` in this engine if it is a built-in step, and says whether it was.
///
/// The step records this engine as the one running it before its work starts, so that
/// the step is failed if this engine dies (see `engine_instance`). A failure of the
/// work goes to the step-failed handler, as a runner's would.
pub async fn try_dispatch_intrinsic(step: BuiltInStep) -> Result<bool> {
    let Some(work) = built_in_work(&step).await? else {
        return Ok(false);
    };
    crate::db::set_step_run_by_engine(&step.pool, step.step_id, *crate::engine_instance::ENGINE_ID)
        .await?;
    tokio::spawn(async move {
        let Err(e) = work.await else { return };
        if let Err(report_err) = report_failure(&step, e).await {
            tracing::error!(
                "Failed to report step {} as failed: {:?}",
                step.step_id,
                report_err
            );
        }
    });
    Ok(true)
}

async fn report_failure(step: &BuiltInStep, error: anyhow::Error) -> Result<()> {
    report_step_failed(
        &step.pool,
        step.run_id,
        step.step_id,
        step.fencing_token,
        format!("{} failed: {:?}", step.step_type, error),
    )
    .await
}
