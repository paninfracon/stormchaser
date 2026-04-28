use anyhow::Result;
use sqlx::PgPool;
use stormchaser_model::step::StepStatus;
use uuid::Uuid;

use crate::handler::{
    fetch_outputs, fetch_quotas, fetch_run_context, handle_approval_notification,
};

pub async fn schedule_step(
    run_id: Uuid,
    step_dsl: &stormchaser_dsl::ast::Step,
    executor: &mut sqlx::PgConnection,
    nats_client: async_nats::Client,
    hcl_ctx: &hcl::eval::Context<'_>,
    pool: PgPool,
    workflow: &stormchaser_dsl::ast::Workflow,
) -> Result<()> {
    let mut resolved_type = step_dsl.r#type.clone();
    let mut resolved_spec = step_dsl.spec.clone();
    let mut resolved_params =
        serde_json::to_value(&step_dsl.params).unwrap_or(serde_json::Value::Null);

    // Merge Step Library if it exists
    if let Some(library) = workflow
        .step_libraries
        .iter()
        .find(|l| l.name == resolved_type)
    {
        resolved_type = library.r#type.clone();

        // Merge specs
        if let (serde_json::Value::Object(mut lib_spec), serde_json::Value::Object(step_spec)) =
            (library.spec.clone(), resolved_spec.clone())
        {
            for (k, v) in step_spec {
                lib_spec.insert(k, v);
            }
            resolved_spec = serde_json::Value::Object(lib_spec);
        } else if resolved_spec.is_null() {
            resolved_spec = library.spec.clone();
        }

        // Merge params
        if let (serde_json::Value::Object(mut lib_params), serde_json::Value::Object(step_params)) = (
            serde_json::to_value(&library.params).unwrap_or(serde_json::Value::Null),
            resolved_params.clone(),
        ) {
            for (k, v) in step_params {
                lib_params.insert(k, v);
            }
            resolved_params = serde_json::Value::Object(lib_params);
        } else if resolved_params.is_null() {
            resolved_params =
                serde_json::to_value(&library.params).unwrap_or(serde_json::Value::Null);
        }
    }

    if let Some(condition_expr) = &step_dsl.condition {
        match crate::hcl_eval::evaluate_raw_expr(condition_expr, hcl_ctx) {
            Ok(serde_json::Value::Bool(true)) => {}
            Ok(serde_json::Value::Bool(false)) => {
                crate::db::insert_step_instance(
                    executor,
                    Uuid::new_v4(),
                    run_id,
                    &step_dsl.name,
                    &step_dsl.r#type,
                    StepStatus::Skipped,
                    chrono::Utc::now(),
                )
                .await?;
                return Ok(());
            }
            Ok(_) => return Err(anyhow::anyhow!("Condition must evaluate to a boolean")),
            Err(e) => return Err(e),
        }
    }

    if let Some(iterate_expr) = &step_dsl.iterate {
        let items = match crate::hcl_eval::evaluate_raw_expr(iterate_expr, hcl_ctx) {
            Ok(serde_json::Value::Array(arr)) => arr,
            Ok(_) => return Err(anyhow::anyhow!("Iterate must evaluate to an array")),
            Err(e) => return Err(e),
        };

        if items.is_empty() {
            crate::db::insert_step_instance(
                executor,
                Uuid::new_v4(),
                run_id,
                &step_dsl.name,
                &step_dsl.r#type,
                StepStatus::Skipped,
                chrono::Utc::now(),
            )
            .await?;
            return Ok(());
        }

        let max_parallel = step_dsl
            .strategy
            .as_ref()
            .and_then(|s| s.max_parallel)
            .unwrap_or(u32::MAX);

        let run_context = fetch_run_context(run_id, &mut *executor).await?;
        let steps_outputs = fetch_outputs(run_id, &mut *executor).await?;
        let quotas = fetch_quotas(run_id, &mut *executor).await?;
        let current_running_count: i64 =
            crate::db::count_running_steps_for_run(&mut *executor, run_id).await?;

        for (idx, item) in items.into_iter().enumerate() {
            let step_instance_id = Uuid::new_v4();
            let status = match step_dsl.r#type.as_str() {
                "Approval" | "Wait" => StepStatus::WaitingForEvent,
                _ => {
                    if (idx as u32) < max_parallel
                        && current_running_count < quotas.max_concurrency as i64
                    {
                        StepStatus::Pending
                    } else {
                        StepStatus::WaitingForEvent
                    }
                }
            };

            let mut iteration_ctx = crate::hcl_eval::create_context(
                run_context.inputs.clone(),
                run_id,
                steps_outputs.clone(),
            );
            let iter_var_name = step_dsl.iterate_as.as_deref().unwrap_or("item");
            iteration_ctx.declare_var(iter_var_name, crate::hcl_eval::json_to_hcl(item));

            let mut resolved_spec_iter = resolved_spec.clone();
            let _ = crate::hcl_eval::resolve_expressions(&mut resolved_spec_iter, &iteration_ctx);

            let mut resolved_params_iter = resolved_params.clone();
            let _ = crate::hcl_eval::resolve_expressions(&mut resolved_params_iter, &iteration_ctx);

            crate::db::insert_step_instance_with_spec(
                &mut *executor,
                step_instance_id,
                run_id,
                &step_dsl.name,
                &resolved_type,
                status.clone(),
                Some(idx as i32),
                resolved_spec_iter.clone(),
                resolved_params_iter.clone(),
                chrono::Utc::now(),
            )
            .await?;

            if status == StepStatus::WaitingForEvent && resolved_type == "Wait" {
                if let Ok(wait_spec) = serde_json::from_value::<stormchaser_model::dsl::WaitEventSpec>(
                    resolved_spec_iter.clone(),
                ) {
                    let _ = crate::db::insert_event_correlation(
                        &mut *executor,
                        Uuid::new_v4(),
                        step_instance_id,
                        run_id,
                        &wait_spec.correlation_key,
                        &wait_spec.correlation_value,
                    )
                    .await;
                }
            }
        }
    } else {
        let _ = crate::hcl_eval::resolve_expressions(&mut resolved_spec, hcl_ctx);
        let _ = crate::hcl_eval::resolve_expressions(&mut resolved_params, hcl_ctx);

        let step_instance_id = Uuid::new_v4();
        let initial_status = match resolved_type.as_str() {
            "Approval" | "Wait" => StepStatus::WaitingForEvent,
            _ => StepStatus::Pending,
        };

        println!(
            "inserting step {} with type {}",
            step_dsl.name, resolved_type
        );
        let insert_result = crate::db::insert_step_instance_with_spec_on_conflict_do_nothing(
            &mut *executor,
            step_instance_id,
            run_id,
            &step_dsl.name,
            &resolved_type,
            initial_status.clone(),
            None::<i32>,
            resolved_spec.clone(),
            resolved_params.clone(),
            chrono::Utc::now(),
        )
        .await?;

        if insert_result.rows_affected() > 0 && initial_status == StepStatus::WaitingForEvent {
            if resolved_type == "Wait" {
                if let Ok(wait_spec) = serde_json::from_value::<stormchaser_model::dsl::WaitEventSpec>(
                    resolved_spec.clone(),
                ) {
                    let _ = crate::db::insert_event_correlation(
                        &mut *executor,
                        Uuid::new_v4(),
                        step_instance_id,
                        run_id,
                        &wait_spec.correlation_key,
                        &wait_spec.correlation_value,
                    )
                    .await;
                }
            } else if step_dsl.r#type == "Approval" {
                if let Ok(approval_spec) = serde_json::from_value::<
                    stormchaser_model::dsl::ApprovalSpec,
                >(resolved_spec.clone())
                {
                    if let Some(notify_spec) = approval_spec.notify {
                        let pool = pool.clone();
                        let nats_client = nats_client.clone();
                        let spec_val = serde_json::to_value(notify_spec)?;
                        tokio::spawn(async move {
                            let _ = handle_approval_notification(
                                run_id,
                                step_instance_id,
                                spec_val,
                                pool,
                                nats_client,
                            )
                            .await;
                        });
                    }
                }
            }
        }
    }

    Ok(())
}
