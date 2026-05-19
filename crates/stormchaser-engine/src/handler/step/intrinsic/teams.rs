use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;
use tracing::error;

use super::utils::fail_step_instance;
#[cfg(feature = "chatops-teams")]
use crate::handler::handle_teams_message;

/// Attempts to dispatch a Teams message step instance.
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
    if step_type == "TeamsMessage" {
        #[cfg(feature = "chatops-teams")]
        {
            let pool = _pool.clone();
            let nats_client = _nats_client.clone();
            let spec = _resolved_spec.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_teams_message(
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
                        format!("TeamsMessage error: {:?}", e),
                    )
                    .await
                    {
                        error!(
                            "failed to transition TeamsMessage step {} to failed state: {}",
                            _step_instance_id, fail_err
                        );
                    }
                }
            });
            return Ok(true);
        }
        #[cfg(not(feature = "chatops-teams"))]
        {
            if let Err(e) = fail_step_instance(
                _step_instance_id,
                &_pool,
                "ChatOps Teams integration is not enabled in this engine build.".to_string(),
            )
            .await
            {
                error!(
                    "failed to transition disabled TeamsMessage step {} to failed state: {}",
                    _step_instance_id, e
                );
            }
            return Ok(true);
        }
    }
    Ok(false)
}
