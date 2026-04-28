use anyhow::Result;
use serde_json::Value;
use uuid::Uuid;

pub async fn release_step_quota_for_instance(
    executor: &mut sqlx::PgConnection,
    run_id: Uuid,
    step_id: Uuid,
) -> Result<()> {
    let row: Option<(String, Value)> =
        crate::db::steps::get_step_type_and_spec(&mut *executor, step_id)
            .await
            .ok();

    if let Some((step_type, spec)) = row {
        let (cpu, mem) = crate::resource_utils::get_step_resource_requirements(&step_type, &spec);
        if cpu > 0.0 || mem > 0 {
            let _ = crate::db::release_step_quota(&mut *executor, run_id, cpu, mem).await;
        }
    }

    Ok(())
}
