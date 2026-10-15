use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;

use super::utils::fail_step_instance;

#[cfg(feature = "aws-lambda")]
use crate::handler::handle_lambda_invoke;

/// Attempts to dispatch an AWS Lambda execution step instance.
pub async fn try_dispatch(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    step_type: &str,
    resolved_spec: &Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<bool> {
    if step_type == "LambdaInvoke" {
        #[cfg(feature = "aws-lambda")]
        {
            let pool = pool.clone();
            let nats_client = nats_client.clone();
            let spec = resolved_spec.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_lambda_invoke(
                    run_id,
                    step_instance_id,
                    spec,
                    pool.clone(),
                    nats_client.clone(),
                )
                .await
                {
                    if let Err(fail_err) = fail_step_instance(
                        run_id,
                        step_instance_id,
                        &pool,
                        nats_client,
                        format!("Lambda invoke failed: {:?}", e),
                    )
                    .await
                    {
                        tracing::error!(
                            "failed to report step {} as failed: {}",
                            step_instance_id,
                            fail_err
                        );
                    }
                }
            });
            return Ok(true);
        }
        #[cfg(not(feature = "aws-lambda"))]
        {
            let _ = resolved_spec;
            if let Err(e) = fail_step_instance(
                run_id,
                step_instance_id,
                &pool,
                nats_client,
                "AWS Lambda support is not enabled in this engine build.".to_string(),
            )
            .await
            {
                tracing::error!(
                    "failed to report step {} as failed: {}",
                    step_instance_id,
                    e
                );
            }
            return Ok(true);
        }
    }

    Ok(false)
}
