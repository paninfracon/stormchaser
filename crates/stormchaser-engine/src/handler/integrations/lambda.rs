use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;

#[cfg(feature = "aws-lambda")]
use super::utils::{report_step_completed, report_step_failed};
#[cfg(feature = "aws-lambda")]
use crate::handler::fetch_step_instance;
#[cfg(feature = "aws-lambda")]
use anyhow::Context;
#[cfg(feature = "aws-lambda")]
use std::collections::HashMap;
#[cfg(feature = "aws-lambda")]
use stormchaser_model::dsl::{self};
#[cfg(feature = "aws-lambda")]
use tracing::info;

#[cfg(feature = "aws-lambda")]
use aws_sdk_lambda::primitives::Blob;
#[cfg(feature = "aws-lambda")]
use aws_sdk_lambda::types::InvocationType;

/// Handle lambda invoke.
#[cfg(feature = "aws-lambda")]
pub async fn handle_lambda_invoke(
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    spec: Value,
    pool: PgPool,
) -> Result<()> {
    let spec: stormchaser_model::dsl::LambdaInvokeSpec = serde_json::from_value(spec)?;

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

    // With no role to assume the function is invoked as the engine itself.
    if spec.assume_role_arn.is_none() {
        crate::aws_guard::ensure_lambda_allowed(&spec.function_name)?;
    }

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
    handle_lambda_response(run_id, step_id, response, pool).await
}

#[cfg(feature = "aws-lambda")]
async fn build_lambda_client(
    spec: &dsl::LambdaInvokeSpec,
    run_id: stormchaser_model::RunId,
) -> Result<aws_sdk_lambda::Client> {
    let mut config_loader = aws_config::defaults(aws_config::BehaviorVersion::v2026_01_12());
    if let Some(region) = &spec.region {
        config_loader = config_loader.region(aws_config::Region::new(region.clone()));
    }
    let config = config_loader.load().await;

    if let Some(role_arn) = &spec.assume_role_arn {
        crate::aws_guard::ensure_role_allowed(role_arn)?;
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
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    response: aws_sdk_lambda::operation::invoke::InvokeOutput,
    pool: PgPool,
) -> Result<()> {
    let status_code = response.status_code();
    let payload = if let Some(payload) = response.payload() {
        let s = String::from_utf8_lossy(payload.as_ref());
        serde_json::from_str::<Value>(&s).unwrap_or(Value::String(s.to_string()))
    } else {
        Value::Null
    };

    if (200..300).contains(&status_code) {
        let mut tx = pool.begin().await?;
        crate::db::upsert_step_output(&mut *tx, step_id, "response", &payload).await?;
        let outputs = HashMap::from([("response".to_string(), payload)]);
        report_step_completed(&mut *tx, run_id, step_id, 0, Some(outputs)).await?;
        tx.commit().await?;
        Ok(())
    } else {
        let error = format!(
            "Lambda returned status code {}: {:?}",
            status_code,
            response.function_error()
        );
        report_step_failed(&mut *pool.acquire().await?, run_id, step_id, 0, error).await
    }
}

#[cfg(not(feature = "aws-lambda"))]
/// Handle lambda invoke.
pub async fn handle_lambda_invoke(
    _run_id: stormchaser_model::RunId,
    _step_id: stormchaser_model::StepInstanceId,
    _spec: Value,
    _pool: PgPool,
) -> Result<()> {
    anyhow::bail!("AWS Lambda support is not enabled. Enable 'aws-lambda' feature.")
}

#[cfg(test)]
mod tests {}
