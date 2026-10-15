#![allow(unused_variables)]
use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;

use super::utils::fail_step_instance;

#[cfg(feature = "email")]
use crate::handler::handle_email_send;

/// Attempts to dispatch an email step instance.
pub async fn try_dispatch(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    step_type: &str,
    resolved_spec: &Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    #[allow(unused_variables)] tls_reloader: Arc<TlsReloader>,
) -> Result<bool> {
    if step_type == "EmailSend" {
        #[cfg(feature = "email")]
        {
            let pool = pool.clone();
            let nats_client = nats_client.clone();
            let spec = resolved_spec.clone();
            let tls_reloader = tls_reloader.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_email_send(
                    run_id,
                    step_instance_id,
                    spec,
                    pool.clone(),
                    nats_client.clone(),
                    tls_reloader,
                )
                .await
                {
                    if let Err(fail_err) = fail_step_instance(
                        run_id,
                        step_instance_id,
                        &pool,
                        nats_client,
                        format!("Email send failed: {:?}", e),
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
        #[cfg(not(feature = "email"))]
        {
            let _ = resolved_spec;
            if let Err(e) = fail_step_instance(
                run_id,
                step_instance_id,
                &pool,
                nats_client,
                "Email support is not enabled in this engine build.".to_string(),
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
