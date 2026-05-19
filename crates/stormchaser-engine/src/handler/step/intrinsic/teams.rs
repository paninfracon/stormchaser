use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;

#[cfg(feature = "chatops-teams")]
use crate::handler::fetch_step_instance;
#[cfg(feature = "chatops-teams")]
use crate::handler::handle_teams_message;

/// Attempts to dispatch a Teams message step instance.
pub async fn try_dispatch(
    _run_id: RunId,
    _step_instance_id: StepInstanceId,
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
                    spec,
                    pool.clone(),
                    nats_client.clone(),
                )
                .await
                {
                    if let Ok(instance) = fetch_step_instance(_step_instance_id, &pool).await {
                        let machine = crate::step_machine::StepMachine::<
                            crate::step_machine::state::Pending,
                        >::from_instance(instance);
                        if let Ok(mut conn) = pool.acquire().await {
                            if let Ok(_machine) = machine
                                .start("error-recovery".to_string(), &mut *conn)
                                .await
                            {
                                if let Ok(instance) =
                                    fetch_step_instance(_step_instance_id, &pool).await
                                {
                                    let machine = crate::step_machine::StepMachine::<
                                        crate::step_machine::state::Running,
                                    >::from_instance(
                                        instance
                                    );
                                    let _ = machine
                                        .fail(
                                            format!("TeamsMessage error: {:?}", e),
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
        #[cfg(not(feature = "chatops-teams"))]
        {
            anyhow::bail!("ChatOps Teams integration is not enabled in this engine build.");
        }
    }
    Ok(false)
}
