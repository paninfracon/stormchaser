use chrono::Utc;
use serde_json::Value;
use stormchaser_dsl::ast;
use stormchaser_model::dsl;
use stormchaser_model::step::StepStatus;
use stormchaser_model::{EventId, RunId, StepInstanceId};

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::handler::{
    fetch_outputs, fetch_quotas, fetch_run_context, handle_approval_notification,
};

/// Schedules a step for execution, creating a new instance and managing its initial state transitions based on dependencies and quotas.
pub async fn schedule_step(
    run_id: RunId,
    step_dsl: &ast::Step,
    executor: &mut sqlx::PgConnection,
    nats_client: async_nats::Client,
    hcl_ctx: &hcl::eval::Context<'_>,
    pool: PgPool,
    workflow: &ast::Workflow,
) -> Result<()> {
    let mut resolved_type = step_dsl.r#type.clone();
    let mut resolved_spec = step_dsl.spec.clone();
    let mut resolved_params = serde_json::to_value(&step_dsl.params).unwrap_or(Value::Null);

    // Merge Step Library if it exists
    if let Some(library) = workflow
        .step_libraries
        .iter()
        .find(|l| l.name == resolved_type)
    {
        resolved_type = library.r#type.clone();

        // Merge specs
        if let (Value::Object(mut lib_spec), Value::Object(step_spec)) =
            (library.spec.clone(), resolved_spec.clone())
        {
            for (k, v) in step_spec {
                lib_spec.insert(k, v);
            }
            resolved_spec = Value::Object(lib_spec);
        } else if resolved_spec.is_null() {
            resolved_spec = library.spec.clone();
        }

        // Merge params
        if let (Value::Object(mut lib_params), Value::Object(step_params)) = (
            serde_json::to_value(&library.params).unwrap_or(Value::Null),
            resolved_params.clone(),
        ) {
            for (k, v) in step_params {
                lib_params.insert(k, v);
            }
            resolved_params = Value::Object(lib_params);
        } else if resolved_params.is_null() {
            resolved_params = serde_json::to_value(&library.params).unwrap_or(Value::Null);
        }
    }

    // Secrets fetched while this step's expressions are evaluated, recorded for the run
    // so that the API masks them wherever they turn up
    let mut fetched_secrets = crate::hcl_eval::take_alias_secrets(run_id);

    if let Some(condition_expr) = &step_dsl.condition {
        let (condition, fetched) = crate::hcl_eval::capturing_secrets(|| {
            crate::hcl_eval::evaluate_raw_expr(condition_expr, hcl_ctx)
        });
        fetched_secrets.extend(fetched);
        crate::db::add_sensitive_values(&mut *executor, run_id, &fetched_secrets).await?;
        match condition {
            Ok(Value::Bool(true)) => {}
            Ok(Value::Bool(false)) => {
                crate::db::insert_step_instance(
                    executor,
                    StepInstanceId::new_v4(),
                    run_id,
                    &step_dsl.name,
                    &step_dsl.r#type,
                    StepStatus::Skipped,
                    Utc::now(),
                )
                .await?;
                return Ok(());
            }
            Ok(_) => return Err(anyhow::anyhow!("Condition must evaluate to a boolean")),
            Err(e) => return Err(e),
        }
    }

    if let Some(iterate_expr) = &step_dsl.iterate {
        let (items, fetched) = crate::hcl_eval::capturing_secrets(|| {
            crate::hcl_eval::evaluate_raw_expr(iterate_expr, hcl_ctx)
        });
        fetched_secrets.extend(fetched);
        crate::db::add_sensitive_values(&mut *executor, run_id, &fetched_secrets).await?;
        let items = match items {
            Ok(Value::Array(arr)) => arr,
            Ok(_) => return Err(anyhow::anyhow!("Iterate must evaluate to an array")),
            Err(e) => return Err(e),
        };
        // Each item becomes a step instance and, in time, a container. The list comes
        // from the workflow, or from whatever a query or an input put in it.
        ensure_iteration_count_allowed(&step_dsl.name, items.len())?;

        schedule_iterated_steps(
            executor,
            run_id,
            step_dsl,
            workflow,
            &resolved_type,
            &resolved_spec,
            &resolved_params,
            items,
        )
        .await?;
    } else {
        let ((), fetched) = crate::hcl_eval::capturing_secrets(|| {
            let _ = crate::hcl_eval::resolve_step_spec(&resolved_type, &mut resolved_spec, hcl_ctx);
            let _ = crate::hcl_eval::resolve_expressions(&mut resolved_params, hcl_ctx, true);
        });
        fetched_secrets.extend(fetched);
        crate::db::add_sensitive_values(&mut *executor, run_id, &fetched_secrets).await?;

        // A Terraform approval is an approval from the moment it exists. Left as its own
        // type it would start as pending and be handed to a runner, which could then
        // report it complete without anyone approving.
        crate::handler::step::intrinsic::terraform::mutate_if_terraform_approval(
            &mut resolved_type,
            &mut resolved_spec,
        );

        let step_instance_id = StepInstanceId::new(Uuid::new_v4());
        let initial_status = match resolved_type.as_str() {
            "Approval" | "Wait" => StepStatus::WaitingForEvent,
            _ => StepStatus::Pending,
        };

        let affinity_context = step_dsl.strategy.as_ref().and_then(|s| s.affinity.clone());

        let insert_result = crate::db::insert_step_instance_with_spec_on_conflict_do_nothing(
            &mut *executor,
            step_instance_id,
            run_id,
            &step_dsl.name,
            &resolved_type,
            initial_status.clone(),
            None::<i32>,
            affinity_context,
            resolved_spec.clone(),
            resolved_params.clone(),
            Utc::now(),
        )
        .await?;

        if insert_result.rows_affected() > 0 && initial_status == StepStatus::WaitingForEvent {
            if resolved_type == "Wait" {
                if let Ok(wait_spec) =
                    serde_json::from_value::<dsl::WaitEventSpec>(resolved_spec.clone())
                {
                    let _ = crate::db::insert_event_correlation(
                        &mut *executor,
                        EventId::new_v4(),
                        step_instance_id,
                        run_id,
                        &wait_spec.correlation_key,
                        &wait_spec.correlation_value,
                    )
                    .await;
                }
            } else if resolved_type == "Approval" {
                if let Ok(approval_spec) =
                    serde_json::from_value::<dsl::ApprovalSpec>(resolved_spec.clone())
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

#[allow(clippy::too_many_arguments)]
async fn schedule_iterated_steps(
    executor: &mut sqlx::PgConnection,
    run_id: RunId,
    step_dsl: &ast::Step,
    workflow: &ast::Workflow,
    resolved_type: &str,
    resolved_spec: &Value,
    resolved_params: &Value,
    items: Vec<Value>,
) -> Result<()> {
    if items.is_empty() {
        crate::db::insert_step_instance(
            executor,
            StepInstanceId::new_v4(),
            run_id,
            &step_dsl.name,
            &step_dsl.r#type,
            StepStatus::Skipped,
            Utc::now(),
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
        let step_instance_id = StepInstanceId::new(Uuid::new_v4());
        let mut iteration_ctx = crate::hcl_eval::create_context(
            run_context.inputs.clone(),
            run_id,
            run_context.secrets.clone(),
            steps_outputs.clone(),
            Some(workflow),
            Some(step_dsl),
        );
        let iter_var_name = step_dsl.iterate_as.as_deref().unwrap_or("item");
        iteration_ctx.declare_var(iter_var_name, crate::hcl_eval::json_to_hcl(item));

        let mut resolved_spec_iter = resolved_spec.clone();
        let mut resolved_params_iter = resolved_params.clone();
        let ((), fetched) = crate::hcl_eval::capturing_secrets(|| {
            let _ = crate::hcl_eval::resolve_step_spec(
                resolved_type,
                &mut resolved_spec_iter,
                &iteration_ctx,
            );
            let _ = crate::hcl_eval::resolve_expressions(
                &mut resolved_params_iter,
                &iteration_ctx,
                true,
            );
        });
        let mut fetched_secrets = crate::hcl_eval::take_alias_secrets(run_id);
        fetched_secrets.extend(fetched);
        crate::db::add_sensitive_values(&mut *executor, run_id, &fetched_secrets).await?;

        let mut resolved_type = resolved_type.to_string();
        crate::handler::step::intrinsic::terraform::mutate_if_terraform_approval(
            &mut resolved_type,
            &mut resolved_spec_iter,
        );
        let resolved_type = resolved_type.as_str();
        let status = match resolved_type {
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

        let affinity_context = step_dsl.strategy.as_ref().and_then(|s| s.affinity.clone());

        crate::db::insert_step_instance_with_spec(
            &mut *executor,
            step_instance_id,
            run_id,
            &step_dsl.name,
            resolved_type,
            status.clone(),
            Some(idx as i32),
            affinity_context,
            resolved_spec_iter.clone(),
            resolved_params_iter.clone(),
            Utc::now(),
        )
        .await?;

        if status == StepStatus::WaitingForEvent && resolved_type == "Wait" {
            if let Ok(wait_spec) =
                serde_json::from_value::<dsl::WaitEventSpec>(resolved_spec_iter.clone())
            {
                let _ = crate::db::insert_event_correlation(
                    &mut *executor,
                    EventId::new_v4(),
                    step_instance_id,
                    run_id,
                    &wait_spec.correlation_key,
                    &wait_spec.correlation_value,
                )
                .await;
            }
        }
    }

    Ok(())
}

/// Most step instances one `iterate` may create. Defaults to 1000.
pub const MAX_ITERATIONS_ENV: &str = "STORMCHASER_MAX_ITERATIONS";

fn max_iterations() -> usize {
    std::env::var(MAX_ITERATIONS_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1000)
}

/// Refuses an `iterate` over more items than the operator allows.
fn ensure_iteration_count_allowed(step_name: &str, count: usize) -> Result<()> {
    let limit = max_iterations();
    if count > limit {
        anyhow::bail!(
            "Step '{}' iterates over {} items; the limit is {} ({})",
            step_name,
            count,
            limit,
            MAX_ITERATIONS_ENV
        );
    }
    Ok(())
}

#[cfg(test)]
mod iteration_limit_tests {
    use super::*;

    #[test]
    fn an_iteration_over_too_many_items_is_refused() {
        assert!(ensure_iteration_count_allowed("fan_out", 1000).is_ok());
        let err = ensure_iteration_count_allowed("fan_out", 1001).unwrap_err();
        assert!(err.to_string().contains("fan_out"));
        assert!(err.to_string().contains("1001"));
    }
}
