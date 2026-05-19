use anyhow::Result;
use serde_json::Value;

/// Releases the quota acquired for a specific step instance, marking it as completed or failed in the quota system.
pub async fn release_step_quota_for_instance(
    executor: &mut sqlx::PgConnection,
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
) -> Result<()> {
    let row: Option<(String, Value)> =
        crate::db::steps::get_step_type_and_spec(&mut *executor, step_id)
            .await
            .ok();

    if let Some((step_type, spec)) = row {
        let req = crate::resource_utils::get_step_resource_requirements(&step_type, &spec);
        if req.cpu > 0.0 || req.memory > 0 {
            let _ =
                crate::db::release_step_quota(&mut *executor, run_id, req.cpu, req.memory).await;
        }
    }

    Ok(())
}
