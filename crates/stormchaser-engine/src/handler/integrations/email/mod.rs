pub mod ses;
pub mod smtp;
pub mod test_report;

pub use test_report::handle_test_report_email;

use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_tls::TlsReloader;

#[cfg(feature = "email")]
use crate::handler::{fetch_outputs, fetch_run_context, fetch_step_instance};
#[cfg(feature = "email")]
use stormchaser_model::dsl::{self, EmailBackend};
#[cfg(feature = "email")]
use stormchaser_model::workflow;
#[cfg(feature = "email")]
use tracing::{error, info};

#[cfg(feature = "email")]
/// Handle email send.
pub async fn handle_email_send(
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    let spec: stormchaser_model::dsl::EmailSpec = serde_json::from_value(spec)?;

    info!(
        "Sending email '{}' from {} for run {}",
        spec.subject, spec.from, run_id
    );

    // 1. Mark as Running
    let instance = fetch_step_instance(step_id, &pool).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
            instance,
        );
    let _ = machine
        .start("email".to_string(), &mut *pool.acquire().await?)
        .await?;

    // 2. Prepare Context for Template Rendering
    let run_context: workflow::RunContext = fetch_run_context(run_id, &pool).await?;
    let outputs: Value = fetch_outputs(run_id, &pool).await?;

    let template_ctx = serde_json::json!({
        "inputs": run_context.inputs,
        "steps": outputs,
        "run": {
            "id": run_id.to_string(),
        }
    });

    // 3. Render Body
    let rendered_body = crate::templating::render_str(&spec.body, &template_ctx)
        .map_err(|e| anyhow::anyhow!("Failed to render email body: {:?}", e))?;

    let is_html = spec.html.unwrap_or(false);
    let backend = spec.backend.clone().unwrap_or(EmailBackend::Smtp);

    match backend {
        EmailBackend::Ses => {
            send_via_ses(
                run_id,
                step_id,
                &spec,
                rendered_body,
                is_html,
                pool,
                nats_client,
            )
            .await
        }
        EmailBackend::Smtp => {
            send_via_smtp(
                run_id,
                step_id,
                &spec,
                rendered_body,
                is_html,
                pool,
                nats_client,
                tls_reloader,
            )
            .await
        }
    }
}

#[cfg(feature = "email")]
async fn send_via_ses(
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    spec: &dsl::EmailSpec,
    rendered_body: String,
    is_html: bool,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    #[cfg(feature = "aws-ses")]
    {
        match self::ses::send_email_ses(
            spec.from.clone(),
            spec.to.clone(),
            spec.cc.clone(),
            spec.bcc.clone(),
            spec.subject.clone(),
            rendered_body,
            is_html,
            spec.ses_region.clone(),
            spec.ses_role_arn.clone(),
            spec.ses_configuration_set_name.clone(),
            run_id,
        )
        .await
        {
            Ok(_) => {
                info!("Email sent via SES successfully for step {}", step_id);
                complete_email_step(run_id, step_id, pool, nats_client).await
            }
            Err(e) => {
                let error_msg = format!("Failed to send email via SES: {:?}", e);
                fail_email_step(run_id, step_id, error_msg, pool, nats_client).await
            }
        }
    }
    #[cfg(not(feature = "aws-ses"))]
    {
        let _ = (
            run_id,
            step_id,
            spec,
            rendered_body,
            is_html,
            pool,
            nats_client,
        );
        anyhow::bail!("SES backend requested but 'aws-ses' feature is not enabled.");
    }
}

#[cfg(feature = "email")]
#[allow(clippy::too_many_arguments)]
async fn send_via_smtp(
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    spec: &dsl::EmailSpec,
    rendered_body: String,
    is_html: bool,
    pool: PgPool,
    nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    use lettre::message::header::ContentType;
    use lettre::{Message, Transport};

    // 4. Build Email
    let mut builder = Message::builder()
        .from(spec.from.parse()?)
        .subject(spec.subject.clone());

    for to in &spec.to {
        builder = builder.to(to.parse()?);
    }

    if let Some(ref ccs) = spec.cc {
        for cc in ccs {
            builder = builder.cc(cc.parse()?);
        }
    }

    if let Some(ref bccs) = spec.bcc {
        for bcc in bccs {
            builder = builder.bcc(bcc.parse()?);
        }
    }
    let message = if is_html {
        builder.header(ContentType::TEXT_HTML).body(rendered_body)?
    } else {
        builder
            .header(ContentType::TEXT_PLAIN)
            .body(rendered_body)?
    };

    // 5. Send Email
    let smtp_params = self::smtp::resolve_smtp_params(
        self::smtp::SmtpOverrides {
            server: spec.smtp_server.clone(),
            port: spec.smtp_port,
            username: spec.smtp_username.clone(),
            password: spec.smtp_password.clone(),
            use_tls: spec.smtp_use_tls,
            use_mtls: spec.smtp_use_mtls,
        },
        self::smtp::EngineSmtp::from_env(),
    );

    let mailer = self::smtp::build_smtp_transport(smtp_params)?;

    match mailer.send(&message) {
        Ok(_) => {
            info!("Email sent successfully for step {}", step_id);
            complete_email_step(run_id, step_id, pool, nats_client).await
        }
        Err(e) => {
            let error_msg = format!("Failed to send email: {:?}", e);
            fail_email_step(run_id, step_id, error_msg, pool, nats_client).await
        }
    }
}

#[cfg(feature = "email")]
pub(crate) async fn complete_email_step(
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    crate::handler::integrations::utils::publish_step_completed_event(
        run_id,
        step_id,
        0,
        None,
        &pool,
        nats_client,
    )
    .await
}

#[cfg(feature = "email")]
pub(crate) async fn fail_email_step(
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    error_msg: String,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    error!("{}", error_msg);
    crate::handler::integrations::utils::publish_step_failed_event(
        run_id,
        step_id,
        0,
        error_msg,
        &pool,
        nats_client,
    )
    .await
}

#[cfg(not(feature = "email"))]
/// Handle email send.
pub async fn handle_email_send(
    _run_id: stormchaser_model::RunId,
    _step_id: stormchaser_model::StepInstanceId,
    _spec: Value,
    _pool: PgPool,
    _nats_client: async_nats::Client,
    __tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    anyhow::bail!("Email support is not enabled. Enable 'email' feature.")
}
