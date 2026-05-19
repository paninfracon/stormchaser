use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;
use tracing::error;

use super::utils::fail_step_instance;
#[cfg(feature = "chatops-slack")]
use crate::handler::handle_slack_message;

/// Attempts to dispatch a Slack message step instance.
#[allow(clippy::too_many_arguments)]
pub async fn try_dispatch(
    _run_id: RunId,
    _step_instance_id: StepInstanceId,
    _fencing_token: i64,
    step_type: &str,
    _resolved_spec: &Value,
    _pool: PgPool,
    _nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<bool> {
    if step_type == "SlackMessage" {
        #[cfg(feature = "chatops-slack")]
        {
            let pool = _pool.clone();
            let nats_client = _nats_client.clone();
            let spec = _resolved_spec.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_slack_message(
                    _run_id,
                    _step_instance_id,
                    _fencing_token,
                    spec,
                    pool.clone(),
                    nats_client.clone(),
                )
                .await
                {
                    if let Err(fail_err) = fail_step_instance(
                        _step_instance_id,
                        &pool,
                        format!("SlackMessage error: {:?}", e),
                    )
                    .await
                    {
                        error!(
                            "failed to transition SlackMessage step {} to failed state: {}",
                            _step_instance_id, fail_err
                        );
                    }
                }
            });
            return Ok(true);
        }
        #[cfg(not(feature = "chatops-slack"))]
        {
            if let Err(e) = fail_step_instance(
                _step_instance_id,
                &_pool,
                "ChatOps Slack integration is not enabled in this engine build.".to_string(),
            )
            .await
            {
                error!(
                    "failed to transition disabled SlackMessage step {} to failed state: {}",
                    _step_instance_id, e
                );
            }
            return Ok(true);
        }
    }
    Ok(false)
}
