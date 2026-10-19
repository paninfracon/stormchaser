use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;

#[cfg(feature = "email")]
#[cfg(feature = "email")]
use stormchaser_model::workflow;

/// Handle approval notification.
pub async fn handle_approval_notification(
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    spec: Value,
    pool: PgPool,
    _nats_client: async_nats::Client,
) -> Result<()> {
    #[cfg(feature = "email")]
    {
        use lettre::message::header::ContentType;
        use lettre::{Message, Transport};
        use tracing::info;

        let spec: stormchaser_model::dsl::EmailSpec = serde_json::from_value(spec)?;
        let base_url =
            std::env::var("SYSTEM_URL").unwrap_or_else(|_| "http://localhost:3000".to_string());

        // 1. Generate Tokens. There is no fallback key: without one the mail goes out
        // with no links, and approvers use the UI or CLI.
        let (approve_link, reject_link) = match crate::hitl::approval_link_key() {
            Some(key) => generate_approval_links(run_id, step_id, &spec.to, &key, &base_url)?,
            None => {
                tracing::warn!(
                    "{} is not set; the approval email for step {} carries no approve or reject links",
                    crate::hitl::APPROVAL_LINK_KEY_ENV,
                    step_id
                );
                (String::new(), String::new())
            }
        };

        // 2. Prepare Context
        let run_context: workflow::RunContext =
            crate::handler::fetch_run_context(run_id, &pool).await?;
        let outputs: Value = crate::handler::fetch_outputs(run_id, &pool).await?;

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
        let rendered_body = crate::templating::render_str(&spec.body, &template_ctx)
            .map_err(|e| anyhow::anyhow!("Failed to render approval email body: {:?}", e))?;

        // 4. Build Email
        let mut builder = Message::builder()
            .from(spec.from.parse()?)
            .subject(spec.subject.clone());

        for to in &spec.to {
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
        let mailer = build_approval_mailer(&spec);
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
fn generate_approval_links(
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    approvers: &[String],
    key: &str,
    base_url: &str,
) -> Result<(String, String)> {
    let approve_token =
        crate::hitl::generate_approval_token(run_id, step_id, "approve", approvers, key)?;
    let reject_token =
        crate::hitl::generate_approval_token(run_id, step_id, "reject", approvers, key)?;

    let approve_link = format!("{}/api/v1/approve-link/{}", base_url, approve_token);
    let reject_link = format!("{}/api/v1/approve-link/{}", base_url, reject_token);
    Ok((approve_link, reject_link))
}

#[cfg(feature = "email")]
fn build_approval_mailer(spec: &stormchaser_model::dsl::EmailSpec) -> lettre::SmtpTransport {
    use super::email::smtp::{resolve_smtp_params, EngineSmtp, SmtpOverrides};
    use lettre::SmtpTransport;
    let params = resolve_smtp_params(
        SmtpOverrides {
            server: spec.smtp_server.clone(),
            port: spec.smtp_port,
            username: spec.smtp_username.clone(),
            password: spec.smtp_password.clone(),
            ..Default::default()
        },
        EngineSmtp::from_env(),
    );

    let mut mailer_builder = SmtpTransport::builder_dangerous(&params.server).port(params.port);

    if let (Some(user), Some(pass)) = (params.username, params.password) {
        let credentials = lettre::transport::smtp::authentication::Credentials::new(user, pass);
        mailer_builder = mailer_builder.credentials(credentials);
    }

    mailer_builder.build()
}

#[cfg(all(test, feature = "email"))]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "email")]
    fn test_generate_approval_links() {
        let run_id = stormchaser_model::RunId::new_v4();
        let step_id = stormchaser_model::StepInstanceId::new_v4();
        let key = "test-only-approval-link-key-aaaaaa";
        let base_url = "https://paninfracon.net";
        let approvers = vec!["lead@paninfracon.net".to_string()];

        let (approve, reject) =
            generate_approval_links(run_id, step_id, &approvers, key, base_url).unwrap();

        assert!(approve.starts_with(base_url));
        assert!(reject.starts_with(base_url));
        assert!(approve.contains("/api/v1/approve-link/"));
        assert!(reject.contains("/api/v1/approve-link/"));
        assert_ne!(approve, reject);
    }
}
