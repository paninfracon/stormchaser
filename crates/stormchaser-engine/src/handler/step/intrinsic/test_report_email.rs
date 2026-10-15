use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;

use super::utils::fail_step_instance;

use crate::handler::handle_test_report_email;

/// Attempts to dispatch a test report email step instance.
pub async fn try_dispatch(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    step_type: &str,
    resolved_spec: &Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<bool> {
    if step_type == "TestReportEmail" {
        let pool = pool.clone();
        let nats_client = nats_client.clone();
        let spec = resolved_spec.clone();
        let tls_reloader = tls_reloader.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_test_report_email(
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
                    format!("Test report email failed: {:?}", e),
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

    Ok(false)
}
