use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[cfg(feature = "aws-lambda")]
use crate::handler::fetch_step_instance;
#[cfg(feature = "aws-lambda")]
use anyhow::Context;
#[cfg(feature = "aws-lambda")]
use chrono::Utc;
#[cfg(feature = "aws-lambda")]
use stormchaser_model::dsl::{self};
#[cfg(feature = "aws-lambda")]
use tracing::info;

#[cfg(feature = "aws-lambda")]
use aws_sdk_lambda::primitives::Blob;
#[cfg(feature = "aws-lambda")]
use aws_sdk_lambda::types::InvocationType;

#[cfg(feature = "aws-lambda")]
/// Handle lambda invoke.
pub async fn handle_lambda_invoke(
    run_id: Uuid,
    step_id: Uuid,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    use stormchaser_model::dsl::LambdaInvokeSpec;

    let spec: LambdaInvokeSpec = serde_json::from_value(spec)?;

    info!(
        "Invoking Lambda function {} for run {}",
        spec.function_name, run_id
    );

    // 1. Mark as Running
    let instance = fetch_step_instance(step_id, &pool).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
            instance,
        );
    let _ = machine
        .start("aws-lambda".to_string(), &mut *pool.acquire().await?)
        .await?;

    let client = build_lambda_client(&spec, run_id).await?;

    // 3. Prepare Payload
    let payload_bytes = if let Some(payload) = spec.payload {
        serde_json::to_vec(&payload)?
    } else {
        Vec::new()
    };

    // 4. Invoke
    let mut request = client
        .invoke()
        .function_name(spec.function_name)
        .payload(Blob::new(payload_bytes));

    if let Some(inv_type) = spec.invocation_type {
        let it = match inv_type.as_str() {
            "Event" => InvocationType::Event,
            "DryRun" => InvocationType::DryRun,
            _ => InvocationType::RequestResponse,
        };
        request = request.invocation_type(it);
    }

    if let Some(qualifier) = spec.qualifier {
        request = request.qualifier(qualifier);
    }

    let response = request.send().await?;

    // 5. Handle Response
    handle_lambda_response(run_id, step_id, response, pool, nats_client).await
}

#[cfg(feature = "aws-lambda")]
async fn build_lambda_client(
    spec: &dsl::LambdaInvokeSpec,
    run_id: Uuid,
) -> Result<aws_sdk_lambda::Client> {
    let mut config_loader = aws_config::defaults(aws_config::BehaviorVersion::v2026_01_12());
    if let Some(region) = &spec.region {
        config_loader = config_loader.region(aws_config::Region::new(region.clone()));
    }
    let config = config_loader.load().await;

    if let Some(role_arn) = &spec.assume_role_arn {
        let sts_client = aws_sdk_sts::Client::new(&config);
        let session_name = spec
            .role_session_name
            .clone()
            .unwrap_or_else(|| format!("stormchaser-run-{}", run_id));

        let assume_role_res = sts_client
            .assume_role()
            .role_arn(role_arn)
            .role_session_name(session_name)
            .send()
            .await?;

        let credentials = assume_role_res
            .credentials()
            .context("Missing credentials from assume_role")?;

        let assumed_credentials = aws_sdk_lambda::config::Credentials::new(
            credentials.access_key_id(),
            credentials.secret_access_key(),
            Some(credentials.session_token().to_string()),
            None,
            "StsAssumedRole",
        );

        let provider = aws_sdk_lambda::config::SharedCredentialsProvider::new(assumed_credentials);

        let assumed_config = aws_sdk_lambda::config::Builder::from(&config)
            .credentials_provider(provider)
            .build();

        Ok(aws_sdk_lambda::Client::from_conf(assumed_config))
    } else {
        Ok(aws_sdk_lambda::Client::new(&config))
    }
}

#[cfg(feature = "aws-lambda")]
async fn handle_lambda_response(
    run_id: Uuid,
    step_id: Uuid,
    response: aws_sdk_lambda::operation::invoke::InvokeOutput,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    let status_code = response.status_code();
    let payload = if let Some(payload) = response.payload() {
        let s = String::from_utf8_lossy(payload.as_ref());
        serde_json::from_str::<Value>(&s).unwrap_or(Value::String(s.to_string()))
    } else {
        Value::Null
    };

    if (200..300).contains(&status_code) {
        // Success
        let instance = fetch_step_instance(step_id, &pool).await?;
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
                instance,
            );
        let _ = machine.succeed(&mut *pool.acquire().await?).await?;

        // Save output
        crate::db::upsert_step_output(&pool, step_id, "response", &payload).await?;

        let mut outputs_map = std::collections::HashMap::new();
        outputs_map.insert("response".to_string(), payload.clone());

        let event = stormchaser_model::events::StepCompletedEvent {
            run_id,
            step_id,
            event_type: "stormchaser.v1.step.completed".to_string(),
            outputs: Some(outputs_map),
            exit_code: Some(0),
            runner_id: None,
            storage_hashes: None,
            artifacts: None,
            test_reports: None,
            timestamp: Utc::now(),
        };
        let js = async_nats::jetstream::new(nats_client);
        stormchaser_model::nats::publish_cloudevent(
            &js,
            "stormchaser.v1.step.completed",
            "stormchaser.v1.step.completed",
            "/stormchaser",
            serde_json::to_value(event).unwrap(),
            Some("1.0"),
            None,
        )
        .await?;
    } else {
        // Failure
        let error_msg = format!(
            "Lambda returned status code {}: {:?}",
            status_code,
            response.function_error()
        );
        let instance = fetch_step_instance(step_id, &pool).await?;
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
                instance,
            );
        let _ = machine
            .fail(error_msg.clone(), None, &mut *pool.acquire().await?)
            .await?;

        let event = serde_json::json!({
            "run_id": run_id,
            "step_id": step_id,
            "event_type": "step_failed",
            "error": error_msg,
            "timestamp": Utc::now(),
        });
        let js = async_nats::jetstream::new(nats_client);
        stormchaser_model::nats::publish_cloudevent(
            &js,
            "stormchaser.v1.step.failed",
            "stormchaser.v1.step.failed",
            "/stormchaser",
            serde_json::to_value(event).unwrap(),
            Some("1.0"),
            None,
        )
        .await?;
    }

    Ok(())
}

#[cfg(not(feature = "aws-lambda"))]
/// Handle lambda invoke.
pub async fn handle_lambda_invoke(
    _run_id: Uuid,
    _step_id: Uuid,
    _spec: Value,
    _pool: PgPool,
    _nats_client: async_nats::Client,
) -> Result<()> {
    anyhow::bail!("AWS Lambda support is not enabled. Enable 'aws-lambda' feature.")
}
