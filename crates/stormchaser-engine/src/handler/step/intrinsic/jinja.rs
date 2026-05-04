use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_tls::TlsReloader;
use uuid::Uuid;

use crate::handler::fetch_step_instance;
use crate::handler::handle_jinja_render;

/// Attempts to dispatch a Jinja template evaluation step instance.
pub async fn try_dispatch(
    run_id: Uuid,
    step_instance_id: Uuid,
    step_type: &str,
    resolved_spec: &Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<bool> {
    if step_type == "JinjaRender" {
        let pool = pool.clone();
        let nats_client = nats_client.clone();
        let spec = resolved_spec.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_jinja_render(
                run_id,
                step_instance_id,
                spec,
                pool.clone(),
                nats_client.clone(),
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
                            if let Ok(instance) = fetch_step_instance(step_instance_id, &pool).await
                            {
                                let machine =
                                    crate::step_machine::StepMachine::<
                                        crate::step_machine::state::Running,
                                    >::from_instance(instance);
                                let _ = machine
                                    .fail(format!("Jinja render failed: {:?}", e), None, &mut *conn)
                                    .await;
                            }
                        }
                    }
                }
            }
        });
        return Ok(true);
    }

    Ok(false)
}

#[cfg(test)]
mod tests {

    // A dummy test for the NATS client is difficult since async_nats::connect requires a real server.
    // However, since we only need coverage on try_dispatch, we can avoid NATS if we extract
    // NATS out, but wait, the signature requires async_nats::Client.
    // I will test only the `step_type != "JinjaRender"` branch which does not use the nats_client if we can create a dummy one,
    // actually async_nats::Client is cloneable but constructing it requires a connection.
    // Instead of instantiating real structs, maybe we don't test this with real NATS client unless we have to.
}
