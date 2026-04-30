use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_tls::TlsReloader;
use uuid::Uuid;

#[cfg(feature = "email")]
use crate::handler::{fetch_outputs, fetch_run_context, fetch_step_instance};
#[cfg(feature = "email")]
use anyhow::Context;
#[cfg(feature = "email")]
use chrono::Utc;
#[cfg(feature = "email")]
use stormchaser_model::dsl::{self, EmailBackend};
#[cfg(feature = "email")]
use stormchaser_model::workflow;
#[cfg(feature = "email")]
use tracing::{error, info};

#[cfg(feature = "email")]
/// Smtpparams.
pub struct SmtpParams {
    /// The server.
    pub server: String,
    /// The port.
    pub port: u16,
    pub username: Option<String>,
    /// The password.
    pub password: Option<String>,
    pub use_tls: bool,
    pub use_mtls: bool,
}

#[cfg(feature = "email")]
fn build_smtp_transport(params: SmtpParams) -> Result<lettre::SmtpTransport> {
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

#[cfg(feature = "email")]
/// Handle email send.
pub async fn handle_email_send(
    run_id: Uuid,
    step_id: Uuid,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
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
    let env = minijinja::Environment::new();
    let rendered_body = env
        .render_str(&spec.body, template_ctx)
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
    run_id: Uuid,
    step_id: Uuid,
    spec: &dsl::EmailSpec,
    rendered_body: String,
    is_html: bool,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    #[cfg(feature = "aws-ses")]
    {
        match send_email_ses(
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
    run_id: Uuid,
    step_id: Uuid,
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
    let smtp_params = SmtpParams {
        server: spec.smtp_server.clone().unwrap_or_else(|| {
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
            .clone()
            .or_else(|| std::env::var("SMTP_USERNAME").ok()),
        password: spec
            .smtp_password
            .clone()
            .or_else(|| std::env::var("SMTP_PASSWORD").ok()),
        use_tls: spec
            .smtp_use_tls
            .unwrap_or_else(|| std::env::var("SMTP_USE_TLS").unwrap_or_default() == "true"),
        use_mtls: spec
            .smtp_use_mtls
            .unwrap_or_else(|| std::env::var("SMTP_USE_MTLS").unwrap_or_default() == "true"),
    };

    let mailer = build_smtp_transport(smtp_params)?;

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
async fn complete_email_step(
    run_id: Uuid,
    step_id: Uuid,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
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
        "timestamp": Utc::now(),
    });
    let js = async_nats::jetstream::new(nats_client);
    js.publish("stormchaser.step.completed", event.to_string().into())
        .await?;
    Ok(())
}

#[cfg(feature = "email")]
async fn fail_email_step(
    run_id: Uuid,
    step_id: Uuid,
    error_msg: String,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    error!("{}", error_msg);
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
    Ok(())
}

#[cfg(feature = "email")]
/// Handle test report email.
pub async fn handle_test_report_email(
    run_id: Uuid,
    step_id: Uuid,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
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
    let reports = fetch_test_reports(run_id, &spec, &pool).await?;

    // 3. Prepare Context
    let run_context: workflow::RunContext = fetch_run_context(run_id, &pool).await?;
    let outputs: Value = fetch_outputs(run_id, &pool).await?;

    let template_ctx = serde_json::json!({
        "inputs": run_context.inputs,
        "steps": outputs,
        "run": {
            "id": run_id.to_string(),
        },
        "reports": reports,
    });

    // 4. Render Template
    let rendered_body = render_test_report_body(&spec, &template_ctx)?;

    let backend = spec.backend.clone().unwrap_or(EmailBackend::Smtp);

    match backend {
        EmailBackend::Ses => {
            send_test_report_via_ses(run_id, step_id, &spec, rendered_body, pool, nats_client).await
        }
        EmailBackend::Smtp => {
            send_test_report_via_smtp(
                run_id,
                step_id,
                &spec,
                rendered_body,
                pool,
                nats_client,
                tls_reloader,
            )
            .await
        }
    }
}

#[cfg(feature = "email")]
async fn fetch_test_reports(
    run_id: Uuid,
    spec: &dsl::TestReportEmailSpec,
    pool: &PgPool,
) -> Result<Vec<Value>> {
    let all_summaries = crate::db::get_test_summaries_for_run(pool, run_id).await?;
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
            crate::db::get_test_cases_for_report(pool, run_id, &summary.report_name).await?;
        reports.push(serde_json::json!({
            "summary": summary,
            "cases": cases
        }));
    }
    Ok(reports)
}

#[cfg(feature = "email")]
fn render_test_report_body(
    spec: &dsl::TestReportEmailSpec,
    template_ctx: &Value,
) -> Result<String> {
    use minijinja::Environment;
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
    let env = Environment::new();
    env.render_str(template_str, template_ctx)
        .map_err(|e| anyhow::anyhow!("Failed to render test report email body: {:?}", e))
}

#[cfg(feature = "email")]
async fn send_test_report_via_ses(
    run_id: Uuid,
    step_id: Uuid,
    spec: &dsl::TestReportEmailSpec,
    rendered_body: String,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    #[cfg(feature = "aws-ses")]
    {
        match send_email_ses(
            spec.from.clone(),
            spec.to.clone(),
            None,
            None,
            spec.subject.clone(),
            rendered_body,
            true, // HTML
            spec.ses_region.clone(),
            spec.ses_role_arn.clone(),
            spec.ses_configuration_set_name.clone(),
            run_id,
        )
        .await
        {
            Ok(_) => {
                info!(
                    "Test report email sent via SES successfully for step {}",
                    step_id
                );
                complete_email_step(run_id, step_id, pool, nats_client).await
            }
            Err(e) => {
                let error_msg = format!("Failed to send test report email via SES: {:?}", e);
                fail_email_step(run_id, step_id, error_msg, pool, nats_client).await
            }
        }
    }
    #[cfg(not(feature = "aws-ses"))]
    {
        let _ = (run_id, step_id, spec, rendered_body, pool, nats_client);
        anyhow::bail!("SES backend requested but 'aws-ses' feature is not enabled.");
    }
}

#[cfg(feature = "email")]
async fn send_test_report_via_smtp(
    run_id: Uuid,
    step_id: Uuid,
    spec: &dsl::TestReportEmailSpec,
    rendered_body: String,
    pool: PgPool,
    nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    use lettre::message::header::ContentType;
    use lettre::{Message, Transport};

    // 5. Build and Send Email
    let mut builder = Message::builder()
        .from(spec.from.parse()?)
        .subject(spec.subject.clone())
        .header(ContentType::TEXT_HTML);

    for to in &spec.to {
        builder = builder.to(to.parse()?);
    }

    let message = builder.body(rendered_body)?;

    let smtp_params = SmtpParams {
        server: spec.smtp_server.clone().unwrap_or_else(|| {
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
            .clone()
            .or_else(|| std::env::var("SMTP_USERNAME").ok()),
        password: spec
            .smtp_password
            .clone()
            .or_else(|| std::env::var("SMTP_PASSWORD").ok()),
        use_tls: spec
            .smtp_use_tls
            .unwrap_or_else(|| std::env::var("SMTP_USE_TLS").unwrap_or_default() == "true"),
        use_mtls: spec
            .smtp_use_mtls
            .unwrap_or_else(|| std::env::var("SMTP_USE_MTLS").unwrap_or_default() == "true"),
    };

    let mailer = build_smtp_transport(smtp_params)?;
    match mailer.send(&message) {
        Ok(_) => {
            info!("Test report email sent successfully for step {}", step_id);
            complete_email_step(run_id, step_id, pool, nats_client).await
        }
        Err(e) => {
            let error_msg = format!("Failed to send test report email: {:?}", e);
            fail_email_step(run_id, step_id, error_msg, pool, nats_client).await
        }
    }
}

#[cfg(not(feature = "email"))]
/// Handle email send.
pub async fn handle_email_send(
    _run_id: Uuid,
    _step_id: Uuid,
    _spec: Value,
    _pool: PgPool,
    _nats_client: async_nats::Client,
    __tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    anyhow::bail!("Email support is not enabled. Enable 'email' feature.")
}

#[cfg(not(feature = "email"))]
/// Handle test report email.
pub async fn handle_test_report_email(
    _run_id: Uuid,
    _step_id: Uuid,
    _spec: Value,
    _pool: PgPool,
    _nats_client: async_nats::Client,
    __tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    anyhow::bail!("Email support is not enabled. Enable 'email' feature.")
}

#[cfg(all(test, feature = "email"))]
mod tests {
    use super::*;
    use serde_json::json;
    use stormchaser_model::dsl::{EmailBackend, TestReportEmailSpec};

    #[test]
    #[cfg(feature = "email")]
    fn test_render_test_report_body_default() {
        let spec = TestReportEmailSpec {
            from: "sender@paninfracon.net".to_string(),
            to: vec!["receiver@paninfracon.net".to_string()],
            subject: "Test Report".to_string(),
            template: None,
            report_name: None,
            backend: Some(EmailBackend::Smtp),
            smtp_server: None,
            smtp_port: None,
            smtp_username: None,
            smtp_password: None,
            smtp_use_tls: None,
            smtp_use_mtls: None,
            ses_region: None,
            ses_role_arn: None,
            ses_configuration_set_name: None,
        };

        let template_ctx = json!({
            "run": { "id": "test-run-id" },
            "reports": [
                {
                    "summary": {
                        "report_name": "JUnit",
                        "total_tests": 10,
                        "passed": 8,
                        "failed": 1,
                        "errors": 1,
                        "skipped": 0,
                        "duration_ms": 1234
                    },
                    "cases": [
                        {
                            "test_suite": "suite1",
                            "test_case": "case1",
                            "status": "passed",
                            "message": null
                        },
                        {
                            "test_suite": "suite1",
                            "test_case": "case2",
                            "status": "failed",
                            "message": "Failure message"
                        },
                        {
                            "test_suite": "suite2",
                            "test_case": "case3",
                            "status": "error",
                            "message": "Error message"
                        }
                    ]
                }
            ]
        });

        let rendered = render_test_report_body(&spec, &template_ctx).unwrap();
        assert!(rendered.contains("Workflow Test Report: test-run-id"));
        assert!(rendered.contains("Report: JUnit"));
        assert!(rendered.contains("8")); // passed
        assert!(rendered.contains("1")); // failed
        assert!(rendered.contains("Failure message"));
        assert!(rendered.contains("Error message"));
    }

    #[test]
    #[cfg(feature = "email")]
    fn test_render_test_report_body_custom() {
        let spec = TestReportEmailSpec {
            from: "sender@paninfracon.net".to_string(),
            to: vec!["receiver@paninfracon.net".to_string()],
            subject: "Test Report".to_string(),
            template: Some("Custom: {{ run.id }}".to_string()),
            report_name: None,
            backend: Some(EmailBackend::Smtp),
            smtp_server: None,
            smtp_port: None,
            smtp_username: None,
            smtp_password: None,
            smtp_use_tls: None,
            smtp_use_mtls: None,
            ses_region: None,
            ses_role_arn: None,
            ses_configuration_set_name: None,
        };

        let template_ctx = json!({
            "run": { "id": "custom-run-id" },
            "reports": []
        });

        let rendered = render_test_report_body(&spec, &template_ctx).unwrap();
        assert_eq!(rendered.trim(), "Custom: custom-run-id");
    }

    #[test]
    #[cfg(feature = "email")]
    fn test_build_smtp_transport_basic() {
        let params = SmtpParams {
            server: "localhost".to_string(),
            port: 25,
            username: None,
            password: None,
            use_tls: false,
            use_mtls: false,
        };
        let _mailer = build_smtp_transport(params).unwrap();
    }

    #[test]
    #[cfg(feature = "email")]
    fn test_build_smtp_transport_mtls_error() {
        let params = SmtpParams {
            server: "localhost".to_string(),
            port: 25,
            username: None,
            password: None,
            use_tls: false,
            use_mtls: true,
        };
        let res = build_smtp_transport(params);
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "mTLS for SMTP is currently unsupported due to lettre limitations."
        );
    }
}
