#![allow(unused_variables)]
use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;

#[cfg(feature = "email")]
use crate::handler::fetch_step_instance;
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
                    if let Ok(instance) = fetch_step_instance(step_instance_id, &pool).await {
                        let machine = crate::step_machine::StepMachine::<
                            crate::step_machine::state::Pending,
                        >::from_instance(instance);
                        if let Ok(mut conn) = pool.acquire().await {
                            if let Ok(_machine) = machine
                                .start("error-recovery".to_string(), &mut *conn)
                                .await
                            {
                                if let Ok(instance) =
                                    fetch_step_instance(step_instance_id, &pool).await
                                {
                                    let machine = crate::step_machine::StepMachine::<
                                        crate::step_machine::state::Running,
                                    >::from_instance(
                                        instance
                                    );
                                    let _ = machine
                                        .fail(
                                            format!("Email send failed: {:?}", e),
                                            None,
                                            &mut *conn,
                                        )
                                        .await;
                                }
                            }
                        }
                    }
                }
            });
            return Ok(true);
        }
        #[cfg(not(feature = "email"))]
        {
            let _ = (run_id, step_instance_id, resolved_spec, pool, nats_client);
            return Ok(true); // Ignore if feature is not enabled
        }
    }

    Ok(false)
}
