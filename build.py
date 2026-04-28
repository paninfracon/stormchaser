import os

with open("crates/stormchaser-engine/src/handler/step.rs", "r") as f:
    text = f.read()

funcs = [
    "pub async fn handle_step_running(",
    "pub async fn release_step_quota_for_instance(",
    "pub async fn handle_step_completed(",
    "pub async fn handle_step_failed(",
    "pub async fn schedule_step(",
    "#[allow(clippy::too_many_arguments)]\npub async fn dispatch_step_instance(",
    "pub async fn handle_step_query("
]

idx = [text.find(f) for f in funcs]

header = text[:idx[0]]
parts = []
for i in range(len(idx)):
    start = idx[i]
    end = idx[i+1] if i+1 < len(idx) else len(text)
    parts.append(text[start:end])

os.makedirs("crates/stormchaser-engine/src/handler/step/intrinsic", exist_ok=True)

with open("crates/stormchaser-engine/src/handler/step/mod.rs", "w") as f:
    f.write('''pub mod dispatch;
pub mod events;
pub mod intrinsic;
pub mod quota;
pub mod scheduling;

pub use dispatch::*;
pub use events::*;
pub use intrinsic::*;
pub use quota::*;
pub use scheduling::*;
''')

with open("crates/stormchaser-engine/src/handler/step/intrinsic/mod.rs", "w") as f:
    f.write('''pub mod jq;
pub mod lambda;
pub mod wasm;

pub use jq::*;
pub use lambda::*;
pub use wasm::*;
''')

with open("crates/stormchaser-engine/src/handler/step/events.rs", "w") as f:
    imports = '''#![allow(clippy::explicit_auto_deref)]
use crate::handler::{
    archive_workflow, dispatch_pending_steps, fetch_inputs, fetch_outputs, fetch_quotas, fetch_run,
    fetch_run_context, fetch_step_instance, handle_approval_notification,
};
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_dsl::ast::Workflow;
use stormchaser_model::step::{StepInstance, StepStatus};
use tracing::{debug, error, info};
use uuid::Uuid;

use super::scheduling::schedule_step;
use super::quota::release_step_quota_for_instance;
use super::dispatch::dispatch_step_instance;

'''
    f.write(imports)
    f.write(parts[0])
    f.write(parts[2])
    f.write(parts[3])
    f.write(parts[6])

with open("crates/stormchaser-engine/src/handler/step/quota.rs", "w") as f:
    imports = '''use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

'''
    f.write(imports)
    f.write(parts[1])

with open("crates/stormchaser-engine/src/handler/step/scheduling.rs", "w") as f:
    imports = '''use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;
use stormchaser_model::step::StepStatus;

use crate::handler::{fetch_outputs, fetch_run_context, handle_approval_notification};
use super::dispatch::dispatch_step_instance;

'''
    f.write(imports)
    f.write(parts[4])

with open("crates/stormchaser-engine/src/handler/step/intrinsic/jq.rs", "w") as f:
    f.write('''use anyhow::Result;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;
use crate::handler::fetch_step_instance;

pub fn mutate_if_has_files(step_type: &mut String, resolved_spec: &mut serde_json::Value) {
    if step_type == "JQ" {
        let jq_spec: Result<stormchaser_model::dsl::JqSpec, _> =
            serde_json::from_value(resolved_spec.get("spec").unwrap_or(&*resolved_spec).clone());

        let has_files = match &jq_spec {
            Ok(jq) => jq.input_file.is_some() || jq.output_file.is_some(),
            Err(_) => false,
        };

        if has_files {
            let jq = jq_spec.unwrap();
            let input_file = jq.input_file.unwrap_or_default();
            let output_file = jq.output_file.unwrap_or_default();

            let mut script = format!("jq -c '{}'", jq.program.replace('\\'', "'\\\\''"));
            if !input_file.is_empty() {
                script.push_str(&format!(" {}", input_file));
            }
            if !output_file.is_empty() {
                script.push_str(&format!(" > {}", output_file));
            }

            let container_spec = stormchaser_model::dsl::CommonContainerSpec {
                image: "ghcr.io/jqlang/jq:latest".to_string(),
                command: Some(vec!["sh".to_string(), "-c".to_string(), script]),
                args: None,
                env: None,
                cpu: None,
                memory: None,
                privileged: None,
                storage_mounts: jq.storage_mounts,
            };

            *step_type = "RunContainer".to_string();
            if let Ok(val) = serde_json::to_value(container_spec) {
                *resolved_spec = val;
            }
        }
    }
}

pub async fn try_dispatch(
    run_id: Uuid,
    step_instance_id: Uuid,
    step_type: &str,
    resolved_spec: &serde_json::Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<bool> {
    if step_type == "JQ" {
        let pool = pool.clone();
        let nats_client = nats_client.clone();
        let spec = resolved_spec.clone();

        tokio::spawn(async move {
            // 1. Transition to running
            if let Ok(instance) = fetch_step_instance(step_instance_id, &pool).await {
                let machine = crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(instance);
                if let Ok(mut conn) = pool.acquire().await {
                    let _ = machine.start("intrinsic-jq".to_string(), &mut *conn).await;
                }
            }

            let actual_spec = spec.get("spec").unwrap_or(&spec).clone();
            let jq_spec: Result<stormchaser_model::dsl::JqSpec, _> =
                serde_json::from_value(actual_spec.clone());

            let result = match jq_spec {
                Ok(jq) => {
                    use jaq_core::load::{Arena, File, Loader};
                    use jaq_core::{Ctx, RcIter};
                    use jaq_json::Val;

                    let input_value = jq.input.unwrap_or(serde_json::Value::Null);

                    let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));
                    let arena = Arena::default();
                    let program = File {
                        code: jq.program.as_str(),
                        path: (),
                    };

                    let modules = loader.load(&arena, program);

                    match modules {
                        Ok(mods) => {
                            let filter = jaq_core::Compiler::default()
                                .with_funs(jaq_std::funs().chain(jaq_json::funs()))
                                .compile(mods);

                            match filter {
                                Ok(f) => {
                                    let input = Val::from(input_value);
                                    let inputs = RcIter::new(core::iter::empty());
                                    let out = f.run((Ctx::new([], &inputs), input));

                                    let mut results = Vec::new();
                                    let mut execution_err = None;

                                    for res in out {
                                        match res {
                                            Ok(v) => {
                                                results.push(serde_json::Value::from(v));
                                            }
                                            Err(e) => {
                                                execution_err = Some(anyhow::anyhow!(
                                                    "JQ execution error: {:?}",
                                                    e
                                                ));
                                                break;
                                            }
                                        }
                                    }

                                    if let Some(e) = execution_err {
                                        Err(e)
                                    } else {
                                        let final_result = if results.len() == 1 {
                                            results.remove(0)
                                        } else {
                                            serde_json::Value::Array(results)
                                        };

                                        Ok(final_result)
                                    }
                                }
                                Err(e) => Err(anyhow::anyhow!("JQ compile error: {:?}", e)),
                            }
                        }
                        Err(e) => Err(anyhow::anyhow!("JQ load/parse error: {:?}", e)),
                    }
                }
                Err(e) => Err(anyhow::anyhow!("Invalid JQ spec: {:?}", e)),
            };

            match result {
                Ok(outputs) => {
                    let event = serde_json::json!({
                        "run_id": run_id,
                        "step_id": step_instance_id,
                        "event_type": "step_completed",
                        "outputs": { "result": outputs },
                        "timestamp": Utc::now(),
                    });
                    let _ = nats_client
                        .publish("stormchaser.step.completed", event.to_string().into())
                        .await;
                }
                Err(e) => {
                    let event = serde_json::json!({
                        "run_id": run_id,
                        "step_id": step_instance_id,
                        "event_type": "step_failed",
                        "error": format!("JQ execution failed: {:?}", e),
                        "timestamp": Utc::now(),
                    });
                    let _ = nats_client
                        .publish("stormchaser.step.failed", event.to_string().into())
                        .await;
                }
            }

            Ok::<(), anyhow::Error>(())
        });
        return Ok(true);
    }

    Ok(false)
}
''')

with open("crates/stormchaser-engine/src/handler/step/intrinsic/wasm.rs", "w") as f:
    f.write('''use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;
use crate::handler::{fetch_inputs, fetch_step_instance};

pub async fn try_dispatch(
    run_id: Uuid,
    step_instance_id: Uuid,
    step_type: &str,
    resolved_spec: &serde_json::Value,
    resolved_params: &serde_json::Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<bool> {
    let wasm_def: Option<(String, String, serde_json::Value)> =
        crate::db::get_wasm_step_definition(&pool, step_type).await?;

    let (module, function, wasm_config) = if let Some((m, f, c)) = wasm_def {
        (m, f, c)
    } else if step_type == "Wasm" {
        let m = resolved_spec["module"]
            .as_str()
            .context("Missing module in Wasm step spec")?
            .to_string();
        let f = resolved_spec["function"]
            .as_str()
            .unwrap_or("run")
            .to_string();
        (m, f, serde_json::Value::Null)
    } else {
        ("".to_string(), "".to_string(), serde_json::Value::Null)
    };

    if !module.is_empty() {
        let pool = pool.clone();
        let nats_client = nats_client.clone();
        let spec = resolved_spec.clone();
        let params = resolved_params.clone();
        let inputs = fetch_inputs(run_id, &pool).await?;

        tokio::spawn(async move {
            let executor = crate::wasm::WasmExecutor::new();
            if let Ok(instance) = fetch_step_instance(step_instance_id, &pool).await {
                let machine = crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(instance);
                if let Ok(mut conn) = pool.acquire().await {
                    let _ = machine.start("wasm".to_string(), &mut *conn).await;
                }
            }

            let input = serde_json::json!({
                "spec": spec,
                "params": params,
                "inputs": inputs,
                "config": wasm_config,
            });

            match executor.execute(&module, &function, input).await {
                Ok(outputs) => {
                    let event = serde_json::json!({
                        "run_id": run_id,
                        "step_id": step_instance_id,
                        "event_type": "step_completed",
                        "outputs": outputs,
                        "timestamp": Utc::now(),
                    });
                    let _ = nats_client
                        .publish("stormchaser.step.completed", event.to_string().into())
                        .await;
                }
                Err(e) => {
                    let event = serde_json::json!({
                        "run_id": run_id,
                        "step_id": step_instance_id,
                        "event_type": "step_failed",
                        "error": format!("WASM execution failed: {:?}", e),
                        "timestamp": Utc::now(),
                    });
                    let _ = nats_client
                        .publish("stormchaser.step.failed", event.to_string().into())
                        .await;
                }
            }
        });
        return Ok(true);
    }

    Ok(false)
}
''')

with open("crates/stormchaser-engine/src/handler/step/intrinsic/lambda.rs", "w") as f:
    f.write('''use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

#[cfg(feature = "aws-lambda")]
use crate::handler::fetch_step_instance;
#[cfg(feature = "aws-lambda")]
use crate::handler::handle_lambda_invoke;

pub async fn try_dispatch(
    run_id: Uuid,
    step_instance_id: Uuid,
    step_type: &str,
    resolved_spec: &serde_json::Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<bool> {
    if step_type == "LambdaInvoke" {
        #[cfg(feature = "aws-lambda")]
        {
            let pool = pool.clone();
            let nats_client = nats_client.clone();
            let spec = resolved_spec.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_lambda_invoke(
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
                                            format!("Lambda invoke failed: {:?}", e),
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
        #[cfg(not(feature = "aws-lambda"))]
        {
            let _ = run_id;
            let _ = step_instance_id;
            let _ = resolved_spec;
            let _ = pool;
            let _ = nats_client;
            return Ok(true); // Ignore if feature is not enabled
        }
    }

    Ok(false)
}
''')

with open("crates/stormchaser-engine/src/handler/step/dispatch.rs", "w") as f:
    f.write('''use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::handler::fetch_run_context;

#[allow(clippy::too_many_arguments)]
pub async fn dispatch_step_instance(
    run_id: Uuid,
    step_instance_id: Uuid,
    step_name: &str,
    step_type: &str,
    resolved_spec: &serde_json::Value,
    resolved_params: &serde_json::Value,
    nats_client: async_nats::Client,
    pool: PgPool,
) -> Result<()> {
    let mut step_type = step_type.to_string();
    let mut resolved_spec = resolved_spec.clone();

    super::intrinsic::jq::mutate_if_has_files(&mut step_type, &mut resolved_spec);

    let run_context = fetch_run_context(run_id, &pool).await?;
    let workflow: stormchaser_model::dsl::Workflow =
        serde_json::from_value(run_context.workflow_definition.clone())
            .context("Failed to parse workflow definition from context")?;

    let mut storage_urls = serde_json::Map::new();

    if !workflow.storage.is_empty() {
        for storage in workflow.storage {
            let backend: Option<stormchaser_model::storage::StorageBackend> =
                if let Some(ref backend_name) = storage.backend {
                    crate::db::get_storage_backend_by_name(&pool, backend_name).await?
                } else {
                    crate::db::get_default_sfs_backend(&pool).await?
                };

            if let Some(backend) = backend {
                let mut get_url = None;
                let mut put_url = None;

                if backend.backend_type == stormchaser_model::storage::BackendType::S3 {
                    let client = crate::s3::get_s3_client(&backend).await?;
                    let bucket = backend.config["bucket"]
                        .as_str()
                        .context("Missing bucket in SFS backend config")?;

                    let key = format!("{}/{}.tar.gz", run_id, storage.name);
                    let expires = std::time::Duration::from_secs(3600);

                    get_url = Some(
                        crate::s3::generate_presigned_url(&client, bucket, &key, false, expires)
                            .await?,
                    );
                    put_url = Some(
                        crate::s3::generate_presigned_url(&client, bucket, &key, true, expires)
                            .await?,
                    );
                }

                let last_hash: Option<(String,)> =
                    crate::db::get_run_storage_last_hash(&pool, run_id, &storage.name).await?;

                let mut artifacts_data = serde_json::Map::new();
                for artifact in storage.artifacts {
                    let instructions = crate::artifact::generate_parking_instructions(
                        &backend,
                        run_id,
                        &storage.name,
                        &artifact,
                    )
                    .await?;
                    artifacts_data.insert(artifact.name.clone(), instructions);
                }

                let mut provision_data = Vec::new();
                for mut prov in storage.provision {
                    if let Some(url) = &prov.url {
                        let mut val = serde_json::Value::String(url.clone());
                        let hcl_ctx = crate::hcl_eval::create_context(
                            run_context.inputs.clone(),
                            run_id,
                            run_context.secrets.clone(),
                        );
                        if crate::hcl_eval::resolve_expressions(&mut val, &hcl_ctx).is_ok() {
                            prov.url = match val {
                                serde_json::Value::String(s) => Some(s),
                                other => Some(other.to_string()),
                            };
                        }
                    }
                    provision_data.push(prov);
                }

                storage_urls.insert(
                    storage.name.clone(),
                    serde_json::json!({
                        "get_url": get_url,
                        "put_url": put_url,
                        "expected_hash": last_hash.map(|h| h.0),
                        "artifacts": artifacts_data,
                        "provision": provision_data,
                    }),
                );
            }
        }
    }

    if super::intrinsic::wasm::try_dispatch(
        run_id,
        step_instance_id,
        &step_type,
        &resolved_spec,
        resolved_params,
        pool.clone(),
        nats_client.clone(),
    ).await? {
        return Ok(());
    }

    if super::intrinsic::lambda::try_dispatch(
        run_id,
        step_instance_id,
        &step_type,
        &resolved_spec,
        pool.clone(),
        nats_client.clone(),
    ).await? {
        return Ok(());
    }

    if super::intrinsic::jq::try_dispatch(
        run_id,
        step_instance_id,
        &step_type,
        &resolved_spec,
        pool.clone(),
        nats_client.clone(),
    ).await? {
        return Ok(());
    }

    let payload = serde_json::json!({
        "run_id": run_id,
        "step_id": step_instance_id,
        "step_name": step_name,
        "step_type": step_type,
        "spec": resolved_spec,
        "params": resolved_params,
        "storage": storage_urls,
        "timestamp": Utc::now(),
    });

    nats_client
        .publish("stormchaser.step.queued", payload.to_string().into())
        .await?;
    Ok(())
}
''')
