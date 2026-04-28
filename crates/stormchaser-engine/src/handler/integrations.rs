#![allow(clippy::explicit_auto_deref)]
#![allow(unused_imports)]
#![allow(unused_variables)]
use super::{fetch_outputs, fetch_run_context, fetch_step_instance};
use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_tls::TlsReloader;
use tracing::{error, info};
use uuid::Uuid;

#[cfg(feature = "email")]
pub struct SmtpParams {
    pub server: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub use_tls: bool,
    pub use_mtls: bool,
}

#[cfg(feature = "email")]
fn build_smtp_transport(
    params: SmtpParams,
    _tls_reloader: &Arc<TlsReloader>,
) -> Result<lettre::SmtpTransport> {
    use lettre::transport::smtp::client::{Tls, TlsParameters};
    use lettre::SmtpTransport;

    let mut mailer_builder = SmtpTransport::relay(&params.server)?.port(params.port);

    if params.use_tls || params.use_mtls {
        if params.use_mtls {
            return Err(anyhow::anyhow!(
                "mTLS for SMTP is currently unsupported due to lettre limitations."
            ));
        }

        let tls_parameters = TlsParameters::builder(params.server).build_rustls()?;
        mailer_builder = mailer_builder.tls(Tls::Required(tls_parameters));
    }

    if let (Some(user), Some(pass)) = (params.username, params.password) {
        let credentials = lettre::transport::smtp::authentication::Credentials::new(user, pass);
        mailer_builder = mailer_builder.credentials(credentials);
    }

    Ok(mailer_builder.build())
}

#[cfg(feature = "aws-ses")]
async fn build_ses_client(
    region: Option<String>,
    role_arn: Option<String>,
    run_id: Uuid,
) -> Result<aws_sdk_ses::Client> {
    let mut config_loader = aws_config::defaults(aws_config::BehaviorVersion::v2026_01_12());
    if let Some(r) = region {
        config_loader = config_loader.region(aws_config::Region::new(r));
    }
    let config = config_loader.load().await;

    if let Some(role) = role_arn {
        let sts_client = aws_sdk_sts::Client::new(&config);
        let session_name = format!("stormchaser-ses-{}", run_id);

        let assume_role_res = sts_client
            .assume_role()
            .role_arn(role)
            .role_session_name(session_name)
            .send()
            .await?;

        let credentials = assume_role_res
            .credentials()
            .context("Missing credentials from assume_role")?;

        let assumed_credentials = aws_sdk_ses::config::Credentials::new(
            credentials.access_key_id(),
            credentials.secret_access_key(),
            Some(credentials.session_token().to_string()),
            None,
            "StsAssumedRole",
        );

        let provider = aws_sdk_ses::config::SharedCredentialsProvider::new(assumed_credentials);
        let assumed_config = aws_sdk_ses::config::Builder::from(&config)
            .credentials_provider(provider)
            .build();

        Ok(aws_sdk_ses::Client::from_conf(assumed_config))
    } else {
        Ok(aws_sdk_ses::Client::new(&config))
    }
}

#[cfg(feature = "aws-ses")]
#[allow(clippy::too_many_arguments)]
async fn send_email_ses(
    from: String,
    to: Vec<String>,
    cc: Option<Vec<String>>,
    bcc: Option<Vec<String>>,
    subject: String,
    body: String,
    html: bool,
    region: Option<String>,
    role_arn: Option<String>,
    configuration_set_name: Option<String>,
    run_id: Uuid,
) -> Result<()> {
    use aws_sdk_ses::types::{Body, Content, Destination, Message};

    let client = build_ses_client(region, role_arn, run_id).await?;

    let mut dest_builder = Destination::builder();
    for addr in to {
        dest_builder = dest_builder.to_addresses(addr);
    }
    if let Some(ccs) = cc {
        for addr in ccs {
            dest_builder = dest_builder.cc_addresses(addr);
        }
    }
    if let Some(bccs) = bcc {
        for addr in bccs {
            dest_builder = dest_builder.bcc_addresses(addr);
        }
    }
    let destination = dest_builder.build();

    let content = Content::builder().data(body).charset("UTF-8").build()?;
    let mut body_builder = Body::builder();
    if html {
        body_builder = body_builder.html(content);
    } else {
        body_builder = body_builder.text(content);
    }

    let message = Message::builder()
        .subject(Content::builder().data(subject).charset("UTF-8").build()?)
        .body(body_builder.build())
        .build();

    let mut request = client
        .send_email()
        .source(from)
        .destination(destination)
        .message(message);

    if let Some(cs) = configuration_set_name {
        request = request.configuration_set_name(cs);
    }

    request.send().await?;

    Ok(())
}

#[cfg(feature = "aws-lambda")]
use aws_sdk_lambda::primitives::Blob;
#[cfg(feature = "aws-lambda")]
use aws_sdk_lambda::types::InvocationType;

#[cfg(feature = "aws-lambda")]
pub async fn handle_lambda_invoke(
    run_id: Uuid,
    step_id: Uuid,
    spec: serde_json::Value,
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

    // 2. Prepare AWS Client
    let mut config_loader = aws_config::defaults(aws_config::BehaviorVersion::v2026_01_12());
    if let Some(region) = spec.region {
        config_loader = config_loader.region(aws_config::Region::new(region));
    }
    let config = config_loader.load().await;

    let client = if let Some(role_arn) = spec.assume_role_arn {
        let sts_client = aws_sdk_sts::Client::new(&config);
        let session_name = spec
            .role_session_name
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

        aws_sdk_lambda::Client::from_conf(assumed_config)
    } else {
        aws_sdk_lambda::Client::new(&config)
    };

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
    let status_code = response.status_code();
    let payload = if let Some(payload) = response.payload() {
        let s = String::from_utf8_lossy(payload.as_ref());
        serde_json::from_str::<serde_json::Value>(&s)
            .unwrap_or(serde_json::Value::String(s.to_string()))
    } else {
        serde_json::Value::Null
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

        let event = serde_json::json!({
            "run_id": run_id,
            "step_id": step_id,
            "event_type": "step_completed",
            "outputs": {
                "response": payload,
            },
            "timestamp": Utc::now(),
        });
        let js = async_nats::jetstream::new(nats_client);
        js.publish("stormchaser.step.completed", event.to_string().into())
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
        js.publish("stormchaser.step.failed", event.to_string().into())
            .await?;
    }

    Ok(())
}

pub async fn handle_webhook_invoke(
    run_id: Uuid,
    step_id: Uuid,
    spec: serde_json::Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    use minijinja::Environment;
    use stormchaser_model::dsl::WebhookInvokeSpec;

    let spec: WebhookInvokeSpec = serde_json::from_value(spec)?;

    info!("Invoking webhook {} for run {}", spec.url, run_id);

    // 1. Mark as Running
    let instance = fetch_step_instance(step_id, &pool).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
            instance,
        );
    let _ = machine
        .start("webhook".to_string(), &mut *pool.acquire().await?)
        .await?;

    // 2. Prepare Context for Template Rendering
    let run_context = fetch_run_context(run_id, &pool).await?;
    let outputs = fetch_outputs(run_id, &pool).await?;

    let template_ctx = serde_json::json!({
        "inputs": run_context.inputs,
        "steps": outputs,
        "run": {
            "id": run_id.to_string(),
        }
    });

    // 3. Render Body if present
    let rendered_body = if let Some(body_tmpl) = &spec.body {
        let env = Environment::new();
        Some(
            env.render_str(body_tmpl, template_ctx)
                .map_err(|e| anyhow::anyhow!("Failed to render webhook body: {:?}", e))?,
        )
    } else {
        None
    };

    // 4. Build and Execute Request
    let client = reqwest::Client::new();
    let method = match spec
        .method
        .as_deref()
        .unwrap_or("POST")
        .to_uppercase()
        .as_str()
    {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        _ => reqwest::Method::POST,
    };

    let mut builder = client.request(method, &spec.url);

    if let Some(headers) = spec.headers {
        for (k, v) in headers {
            builder = builder.header(k, v);
        }
    }

    if let Some(body) = rendered_body {
        builder = builder.body(body);
    }

    let timeout = spec
        .timeout
        .and_then(|t| humantime::parse_duration(&t).ok())
        .unwrap_or(std::time::Duration::from_secs(30));

    let res = builder.timeout(timeout).send().await?;
    let status = res.status();

    if status.is_success() {
        let body_bytes = res.bytes().await?;
        let body_val: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap_or_else(
            |_| serde_json::json!({ "text": String::from_utf8_lossy(&body_bytes) }),
        );

        let instance = fetch_step_instance(step_id, &pool).await?;
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
                instance,
            );
        let _ = machine.succeed(&mut *pool.acquire().await?).await?;

        let event = serde_json::json!({
            "run_id": run_id,
            "step_id": step_id,
            "event_type": "step_completed",
            "outputs": body_val,
            "timestamp": Utc::now(),
        });
        let js = async_nats::jetstream::new(nats_client);
        js.publish("stormchaser.step.completed", event.to_string().into())
            .await?;
        Ok(())
    } else {
        let error_body = res
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        anyhow::bail!("Webhook failed with status {}: {}", status, error_body);
    }
}

pub async fn handle_approval_notification(
    run_id: Uuid,
    step_id: Uuid,
    spec: serde_json::Value,
    pool: PgPool,
    _nats_client: async_nats::Client,
) -> Result<()> {
    #[cfg(feature = "email")]
    {
        use lettre::message::header::ContentType;
        use lettre::{Message, SmtpTransport, Transport};
        use minijinja::Environment;
        use stormchaser_model::dsl::EmailSpec;

        let spec: EmailSpec = serde_json::from_value(spec)?;
        let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "test-secret".to_string());
        let base_url =
            std::env::var("SYSTEM_URL").unwrap_or_else(|_| "http://localhost:3000".to_string());

        // 1. Generate Tokens
        let approve_token =
            crate::hitl::generate_approval_token(run_id, step_id, "approve", &secret)?;
        let reject_token =
            crate::hitl::generate_approval_token(run_id, step_id, "reject", &secret)?;

        let approve_link = format!("{}/api/v1/approve-link/{}", base_url, approve_token);
        let reject_link = format!("{}/api/v1/approve-link/{}", base_url, reject_token);

        // 2. Prepare Context
        let run_context = fetch_run_context(run_id, &pool).await?;
        let outputs = fetch_outputs(run_id, &pool).await?;

        let template_ctx = serde_json::json!({
            "inputs": run_context.inputs,
            "steps": outputs,
            "run": {
                "id": run_id.to_string(),
            },
            "step": {
                "id": step_id.to_string(),
            },
            "approve_link": approve_link,
            "reject_link": reject_link,
        });

        // 3. Render Body
        let env = Environment::new();
        let rendered_body = env
            .render_str(&spec.body, template_ctx)
            .map_err(|e| anyhow::anyhow!("Failed to render approval email body: {:?}", e))?;

        // 4. Build Email
        let mut builder = Message::builder()
            .from(spec.from.parse()?)
            .subject(spec.subject);

        for to in spec.to {
            builder = builder.to(to.parse()?);
        }

        let is_html = spec.html.unwrap_or(false);
        let message = if is_html {
            builder.header(ContentType::TEXT_HTML).body(rendered_body)?
        } else {
            builder
                .header(ContentType::TEXT_PLAIN)
                .body(rendered_body)?
        };

        // 5. Send Email
        let smtp_server = spec.smtp_server.unwrap_or_else(|| {
            std::env::var("SMTP_SERVER").unwrap_or_else(|_| "localhost".to_string())
        });
        let smtp_port = spec.smtp_port.unwrap_or_else(|| {
            std::env::var("SMTP_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(25)
        });

        let mut mailer_builder = SmtpTransport::builder_dangerous(&smtp_server).port(smtp_port);

        if let (Some(user), Some(pass)) = (
            spec.smtp_username
                .or_else(|| std::env::var("SMTP_USERNAME").ok()),
            spec.smtp_password
                .or_else(|| std::env::var("SMTP_PASSWORD").ok()),
        ) {
            let credentials = lettre::transport::smtp::authentication::Credentials::new(user, pass);
            mailer_builder = mailer_builder.credentials(credentials);
        }

        let mailer = mailer_builder.build();
        mailer.send(&message)?;
        info!("Approval notification email sent for step {}", step_id);
    }

    #[cfg(not(feature = "email"))]
    {
        let _ = (run_id, step_id, spec, pool, _nats_client);
        tracing::warn!("Approval notification requested but 'email' feature is not enabled.");
    }

    Ok(())
}

#[cfg(feature = "email")]
pub async fn handle_email_send(
    run_id: Uuid,
    step_id: Uuid,
    spec: serde_json::Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    use lettre::message::header::ContentType;
    use lettre::{Message, Transport};
    use stormchaser_model::dsl::EmailSpec;

    let spec: EmailSpec = serde_json::from_value(spec)?;

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
    let run_context = fetch_run_context(run_id, &pool).await?;
    let outputs = fetch_outputs(run_id, &pool).await?;

    let template_ctx = serde_json::json!({
        "inputs": run_context.inputs,
        "steps": outputs,
        "run": {
            "id": run_id.to_string(),
        }
    });

    // 3. Render Body
    let env = minijinja::Environment::new();
    let rendered_body = env
        .render_str(&spec.body, template_ctx)
        .map_err(|e| anyhow::anyhow!("Failed to render email body: {:?}", e))?;

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
    let is_html = spec.html.unwrap_or(false);

    let backend = spec
        .backend
        .clone()
        .unwrap_or(stormchaser_model::dsl::EmailBackend::Smtp);

    match backend {
        stormchaser_model::dsl::EmailBackend::Ses => {
            #[cfg(feature = "aws-ses")]
            {
                match send_email_ses(
                    spec.from,
                    spec.to,
                    spec.cc,
                    spec.bcc,
                    spec.subject,
                    rendered_body,
                    is_html,
                    spec.ses_region,
                    spec.ses_role_arn,
                    spec.ses_configuration_set_name,
                    run_id,
                )
                .await
                {
                    Ok(_) => {
                        info!("Email sent via SES successfully for step {}", step_id);
                        let instance = fetch_step_instance(step_id, &pool).await?;
                        let machine = crate::step_machine::StepMachine::<
                            crate::step_machine::state::Running,
                        >::from_instance(instance);
                        let _ = machine.succeed(&mut *pool.acquire().await?).await?;

                        let event = serde_json::json!({
                            "run_id": run_id,
                            "step_id": step_id,
                            "event_type": "step_completed",
                            "timestamp": Utc::now(),
                        });
                        let js = async_nats::jetstream::new(nats_client);
                        js.publish("stormchaser.step.completed", event.to_string().into())
                            .await?;
                    }
                    Err(e) => {
                        let error_msg = format!("Failed to send email via SES: {:?}", e);
                        error!("{}", error_msg);
                        let instance = fetch_step_instance(step_id, &pool).await?;
                        let machine = crate::step_machine::StepMachine::<
                            crate::step_machine::state::Running,
                        >::from_instance(instance);
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
                        js.publish("stormchaser.step.failed", event.to_string().into())
                            .await?;
                    }
                }
            }
            #[cfg(not(feature = "aws-ses"))]
            {
                anyhow::bail!("SES backend requested but 'aws-ses' feature is not enabled.");
            }
        }
        stormchaser_model::dsl::EmailBackend::Smtp => {
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
            let smtp_params = SmtpParams {
                server: spec.smtp_server.unwrap_or_else(|| {
                    std::env::var("SMTP_SERVER").unwrap_or_else(|_| "localhost".to_string())
                }),
                port: spec.smtp_port.unwrap_or_else(|| {
                    std::env::var("SMTP_PORT")
                        .ok()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(25)
                }),
                username: spec
                    .smtp_username
                    .or_else(|| std::env::var("SMTP_USERNAME").ok()),
                password: spec
                    .smtp_password
                    .or_else(|| std::env::var("SMTP_PASSWORD").ok()),
                use_tls: spec
                    .smtp_use_tls
                    .unwrap_or_else(|| std::env::var("SMTP_USE_TLS").unwrap_or_default() == "true"),
                use_mtls: spec.smtp_use_mtls.unwrap_or_else(|| {
                    std::env::var("SMTP_USE_MTLS").unwrap_or_default() == "true"
                }),
            };

            let mailer = build_smtp_transport(smtp_params, &tls_reloader)?;

            match mailer.send(&message) {
                Ok(_) => {
                    info!("Email sent successfully for step {}", step_id);
                    let instance = fetch_step_instance(step_id, &pool).await?;
                    let machine = crate::step_machine::StepMachine::<
                        crate::step_machine::state::Running,
                    >::from_instance(instance);
                    let _ = machine.succeed(&mut *pool.acquire().await?).await?;

                    let event = serde_json::json!({
                        "run_id": run_id,
                        "step_id": step_id,
                        "event_type": "step_completed",
                        "timestamp": Utc::now(),
                    });
                    let js = async_nats::jetstream::new(nats_client);
                    js.publish("stormchaser.step.completed", event.to_string().into())
                        .await?;
                }
                Err(e) => {
                    let error_msg = format!("Failed to send email: {:?}", e);
                    error!("{}", error_msg);
                    let instance = fetch_step_instance(step_id, &pool).await?;
                    let machine = crate::step_machine::StepMachine::<
                        crate::step_machine::state::Running,
                    >::from_instance(instance);
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
                    js.publish("stormchaser.step.failed", event.to_string().into())
                        .await?;
                }
            }
        }
    }

    Ok(())
}

pub async fn handle_jinja_render(
    run_id: Uuid,
    step_id: Uuid,
    spec: serde_json::Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    use minijinja::Environment;
    use stormchaser_model::dsl::JinjaRenderSpec;

    let spec: JinjaRenderSpec = serde_json::from_value(spec)?;

    info!("Rendering Jinja template for run {}", run_id);

    // 1. Mark as Running
    let instance = fetch_step_instance(step_id, &pool).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
            instance,
        );
    let _ = machine
        .start("jinja".to_string(), &mut *pool.acquire().await?)
        .await?;

    // 2. Prepare Context for Template Rendering
    let run_context = fetch_run_context(run_id, &pool).await?;
    let outputs = fetch_outputs(run_id, &pool).await?;

    let mut template_ctx = serde_json::json!({
        "inputs": run_context.inputs,
        "steps": outputs,
        "run": {
            "id": run_id.to_string(),
        }
    });

    // Merge custom context if provided
    if let Some(custom_ctx) = spec.context {
        if let Some(obj) = template_ctx.as_object_mut() {
            if let Some(custom_obj) = custom_ctx.as_object() {
                for (k, v) in custom_obj {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
    }

    // 3. Render Template
    let env = Environment::new();
    let rendered = env
        .render_str(&spec.template, template_ctx)
        .map_err(|e| anyhow::anyhow!("Failed to render Jinja template: {:?}", e))?;

    // 4. Mark as Succeeded and Save Output in one transaction
    let mut tx = pool.begin().await?;
    let instance = fetch_step_instance(step_id, &mut *tx).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
            instance,
        );
    let _ = machine.succeed(&mut *tx).await?;

    let output_key = spec.output_key.unwrap_or_else(|| "result".to_string());
    crate::db::upsert_step_output(
        &mut *tx,
        step_id,
        &output_key,
        &serde_json::Value::String(rendered.clone()),
    )
    .await?;

    tx.commit().await?;

    let event = serde_json::json!({
        "run_id": run_id,
        "step_id": step_id,
        "event_type": "step_completed",
        "outputs": {
            &output_key: rendered,
        },
        "timestamp": Utc::now(),
    });
    let js = async_nats::jetstream::new(nats_client);
    js.publish("stormchaser.step.completed", event.to_string().into())
        .await?;

    Ok(())
}

pub async fn handle_test_report_email(
    run_id: Uuid,
    step_id: Uuid,
    spec: serde_json::Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    #[cfg(feature = "email")]
    {
        use lettre::message::header::ContentType;
        use lettre::{Message, Transport};
        use minijinja::Environment;
        use stormchaser_model::dsl::TestReportEmailSpec;

        let spec: TestReportEmailSpec = serde_json::from_value(spec)?;

        info!("Sending test report email for run {}", run_id);

        // 1. Mark as Running
        let instance = fetch_step_instance(step_id, &pool).await?;
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
                instance,
            );
        let _ = machine
            .start("test-report-email".to_string(), &mut *pool.acquire().await?)
            .await?;

        // 2. Fetch Test Data
        let all_summaries = crate::db::get_test_summaries_for_run(&pool, run_id).await?;
        let filtered_summaries = if let Some(name) = &spec.report_name {
            all_summaries
                .into_iter()
                .filter(|s| &s.report_name == name)
                .collect::<Vec<_>>()
        } else {
            all_summaries
        };

        let mut reports = Vec::new();
        for summary in filtered_summaries {
            let cases =
                crate::db::get_test_cases_for_report(&pool, run_id, &summary.report_name).await?;
            reports.push(serde_json::json!({
                "summary": summary,
                "cases": cases
            }));
        }

        // 3. Prepare Context
        let run_context = fetch_run_context(run_id, &pool).await?;
        let outputs = fetch_outputs(run_id, &pool).await?;

        let template_ctx = serde_json::json!({
            "inputs": run_context.inputs,
            "steps": outputs,
            "run": {
                "id": run_id.to_string(),
            },
            "reports": reports,
        });

        // 4. Render Template
        let env = Environment::new();
        let default_template = r#"
        <html>
        <head>
            <style>
                body { font-family: sans-serif; color: #333; }
                .report { margin-bottom: 30px; border: 1px solid #ddd; padding: 15px; border-radius: 5px; }
                .summary { display: flex; gap: 20px; background: #f9f9f9; padding: 10px; margin-bottom: 10px; }
                .stat { text-align: center; }
                .stat-value { font-size: 20px; font-weight: bold; }
                .stat-label { font-size: 12px; color: #666; }
                .passed { color: #28a745; }
                .failed { color: #dc3545; }
                .skipped { color: #ffc107; }
                .error { color: #6f42c1; }
                table { width: 100%; border-collapse: collapse; }
                th, td { text-align: left; padding: 8px; border-bottom: 1px solid #eee; }
                tr.fail-row { background: #fff5f5; }
            </style>
        </head>
        <body>
            <h1>Workflow Test Report: {{ run.id }}</h1>
            {% for report in reports %}
                <div class="report">
                    <h2>Report: {{ report.summary.report_name }}</h2>
                    <div class="summary">
                        <div class="stat"><div class="stat-value">{{ report.summary.total_tests }}</div><div class="stat-label">Total</div></div>
                        <div class="stat"><div class="stat-value passed">{{ report.summary.passed }}</div><div class="stat-label">Passed</div></div>
                        <div class="stat"><div class="stat-value failed">{{ report.summary.failed }}</div><div class="stat-label">Failed</div></div>
                        <div class="stat"><div class="stat-value error">{{ report.summary.errors }}</div><div class="stat-label">Errors</div></div>
                        <div class="stat"><div class="stat-value skipped">{{ report.summary.skipped }}</div><div class="stat-label">Skipped</div></div>
                        <div class="stat"><div class="stat-value">{{ report.summary.duration_ms }}ms</div><div class="stat-label">Duration</div></div>
                    </div>
                    {% if report.summary.failed > 0 or report.summary.errors > 0 %}
                        <h3>Failures</h3>
                        <table>
                            <thead>
                                <tr><th>Suite</th><th>Case</th><th>Status</th><th>Message</th></tr>
                            </thead>
                            <tbody>
                                {% for case in report.cases %}
                                    {% if case.status == 'failed' or case.status == 'error' %}
                                        <tr class="fail-row">
                                            <td>{{ case.test_suite or "Default" }}</td>
                                            <td>{{ case.test_case }}</td>
                                            <td class="{{ case.status }}">{{ case.status }}</td>
                                            <td>{{ case.message or "" }}</td>
                                        </tr>
                                    {% endif %}
                                {% endfor %}
                            </tbody>
                        </table>
                    {% else %}
                        <p class="passed">All tests passed!</p>
                    {% endif %}
                </div>
            {% endfor %}
        </body>
        </html>
        "#;

        let template_str = spec.template.as_deref().unwrap_or(default_template);
        let rendered_body = env
            .render_str(template_str, template_ctx)
            .map_err(|e| anyhow::anyhow!("Failed to render test report email body: {:?}", e))?;

        let backend = spec
            .backend
            .clone()
            .unwrap_or(stormchaser_model::dsl::EmailBackend::Smtp);

        match backend {
            stormchaser_model::dsl::EmailBackend::Ses => {
                #[cfg(feature = "aws-ses")]
                {
                    match send_email_ses(
                        spec.from,
                        spec.to,
                        None,
                        None,
                        spec.subject,
                        rendered_body,
                        true, // HTML
                        spec.ses_region,
                        spec.ses_role_arn,
                        spec.ses_configuration_set_name,
                        run_id,
                    )
                    .await
                    {
                        Ok(_) => {
                            info!(
                                "Test report email sent via SES successfully for step {}",
                                step_id
                            );
                            let mut tx = pool.begin().await?;
                            let instance = fetch_step_instance(step_id, &mut *tx).await?;
                            let machine = crate::step_machine::StepMachine::<
                                crate::step_machine::state::Running,
                            >::from_instance(instance);
                            let _ = machine.succeed(&mut *tx).await?;
                            tx.commit().await?;

                            let event = serde_json::json!({
                                "run_id": run_id,
                                "step_id": step_id,
                                "event_type": "step_completed",
                                "timestamp": Utc::now(),
                            });
                            let js = async_nats::jetstream::new(nats_client);
                            js.publish("stormchaser.step.completed", event.to_string().into())
                                .await?;
                        }
                        Err(e) => {
                            let error_msg =
                                format!("Failed to send test report email via SES: {:?}", e);
                            error!("{}", error_msg);
                            let mut tx = pool.begin().await?;
                            let instance = fetch_step_instance(step_id, &mut *tx).await?;
                            let machine = crate::step_machine::StepMachine::<
                                crate::step_machine::state::Running,
                            >::from_instance(instance);
                            let _ = machine.fail(error_msg.clone(), None, &mut *tx).await?;
                            tx.commit().await?;

                            let event = serde_json::json!({
                                "run_id": run_id,
                                "step_id": step_id,
                                "event_type": "step_failed",
                                "error": error_msg,
                                "timestamp": Utc::now(),
                            });
                            let js = async_nats::jetstream::new(nats_client);
                            js.publish("stormchaser.step.failed", event.to_string().into())
                                .await?;
                        }
                    }
                }
                #[cfg(not(feature = "aws-ses"))]
                {
                    anyhow::bail!("SES backend requested but 'aws-ses' feature is not enabled.");
                }
            }
            stormchaser_model::dsl::EmailBackend::Smtp => {
                // 5. Build and Send Email
                let mut builder = Message::builder()
                    .from(spec.from.parse()?)
                    .subject(spec.subject)
                    .header(ContentType::TEXT_HTML);

                for to in spec.to {
                    builder = builder.to(to.parse()?);
                }

                let message = builder.body(rendered_body)?;

                let smtp_params = SmtpParams {
                    server: spec.smtp_server.unwrap_or_else(|| {
                        std::env::var("SMTP_SERVER").unwrap_or_else(|_| "localhost".to_string())
                    }),
                    port: spec.smtp_port.unwrap_or_else(|| {
                        std::env::var("SMTP_PORT")
                            .ok()
                            .and_then(|p| p.parse().ok())
                            .unwrap_or(25)
                    }),
                    username: spec
                        .smtp_username
                        .or_else(|| std::env::var("SMTP_USERNAME").ok()),
                    password: spec
                        .smtp_password
                        .or_else(|| std::env::var("SMTP_PASSWORD").ok()),
                    use_tls: spec.smtp_use_tls.unwrap_or_else(|| {
                        std::env::var("SMTP_USE_TLS").unwrap_or_default() == "true"
                    }),
                    use_mtls: spec.smtp_use_mtls.unwrap_or_else(|| {
                        std::env::var("SMTP_USE_MTLS").unwrap_or_default() == "true"
                    }),
                };

                let mailer = build_smtp_transport(smtp_params, &tls_reloader)?;
                match mailer.send(&message) {
                    Ok(_) => {
                        info!("Test report email sent successfully for step {}", step_id);
                        let mut tx = pool.begin().await?;
                        let instance = fetch_step_instance(step_id, &mut *tx).await?;
                        let machine = crate::step_machine::StepMachine::<
                            crate::step_machine::state::Running,
                        >::from_instance(instance);
                        let _ = machine.succeed(&mut *tx).await?;
                        tx.commit().await?;

                        let event = serde_json::json!({
                            "run_id": run_id,
                            "step_id": step_id,
                            "event_type": "step_completed",
                            "timestamp": Utc::now(),
                        });
                        let js = async_nats::jetstream::new(nats_client);
                        js.publish("stormchaser.step.completed", event.to_string().into())
                            .await?;
                    }
                    Err(e) => {
                        let error_msg = format!("Failed to send test report email: {:?}", e);
                        error!("{}", error_msg);
                        let mut tx = pool.begin().await?;
                        let instance = fetch_step_instance(step_id, &mut *tx).await?;
                        let machine = crate::step_machine::StepMachine::<
                            crate::step_machine::state::Running,
                        >::from_instance(instance);
                        let _ = machine.fail(error_msg.clone(), None, &mut *tx).await?;
                        tx.commit().await?;

                        let event = serde_json::json!({
                            "run_id": run_id,
                            "step_id": step_id,
                            "event_type": "step_failed",
                            "error": error_msg,
                            "timestamp": Utc::now(),
                        });
                        let js = async_nats::jetstream::new(nats_client);
                        js.publish("stormchaser.step.failed", event.to_string().into())
                            .await?;
                    }
                }
            }
        }
    }

    #[cfg(not(feature = "email"))]
    {
        let _ = (run_id, step_id, spec, pool, nats_client);
        anyhow::bail!("Test report email requested but 'email' feature is not enabled.");
    }

    #[cfg(feature = "email")]
    Ok(())
}
