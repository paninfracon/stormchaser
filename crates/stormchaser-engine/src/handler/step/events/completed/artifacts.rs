use anyhow::Result;
use stormchaser_dsl::ast::Workflow;
use stormchaser_model::{RunId, StepInstanceId};
use uuid::Uuid;

pub async fn persist_artifact_metadata(
    tx: &mut sqlx::PgConnection,
    run_id: RunId,
    step_id: StepInstanceId,
    artifacts: Option<&serde_json::Map<String, serde_json::Value>>,
    workflow: &Workflow,
) -> Result<()> {
    let Some(artifacts) = artifacts else {
        return Ok(());
    };

    for (name, meta) in artifacts {
        let storage = workflow
            .storage
            .iter()
            .find(|s| s.artifacts.iter().any(|a| a.name == *name));

        if let Some(s) = storage {
            let connection_id: Option<Uuid> = if let Some(ref backend_name) = s.backend {
                crate::db::get_storage_backend_id_by_name(&mut *tx, backend_name).await?
            } else {
                crate::db::get_default_sfs_backend_id(&mut *tx).await?
            };

            if let Some(bid) = connection_id {
                let remote_path = format!("artifacts/{}/{}/{}", run_id, s.name, name);
                crate::db::insert_artifact_registry(
                    &mut *tx,
                    run_id,
                    step_id,
                    name,
                    stormchaser_model::ConnectionId::new(bid),
                    remote_path,
                    meta.clone(),
                )
                .await?;
            }
        }
    }
    Ok(())
}
