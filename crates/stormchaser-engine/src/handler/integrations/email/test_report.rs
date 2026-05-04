use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_tls::TlsReloader;
use uuid::Uuid;

#[cfg(feature = "email")]
use crate::handler::{fetch_outputs, fetch_run_context, fetch_step_instance};
#[cfg(feature = "email")]
use stormchaser_model::dsl::{self, EmailBackend};
#[cfg(feature = "email")]
use stormchaser_model::workflow;
#[cfg(feature = "email")]
use tracing::info;

#[cfg(feature = "email")]
use super::{complete_email_step, fail_email_step, ses, smtp};

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
        match ses::send_email_ses(
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

    let smtp_params = smtp::SmtpParams {
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

    let mailer = smtp::build_smtp_transport(smtp_params)?;
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
}
