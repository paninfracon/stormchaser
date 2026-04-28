use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[cfg(feature = "email")]
use stormchaser_model::dsl::EmailSpec;
#[cfg(feature = "email")]
use stormchaser_model::workflow;

pub async fn handle_approval_notification(
    run_id: Uuid,
    step_id: Uuid,
    spec: Value,
    pool: PgPool,
    _nats_client: async_nats::Client,
) -> Result<()> {
    #[cfg(feature = "email")]
    {
        use lettre::message::header::ContentType;
        use lettre::{Message, Transport};
        use minijinja::Environment;
        use tracing::info;

        let spec: EmailSpec = serde_json::from_value(spec)?;
        let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "test-secret".to_string());
        let base_url =
            std::env::var("SYSTEM_URL").unwrap_or_else(|_| "http://localhost:3000".to_string());

        // 1. Generate Tokens
        let (approve_link, reject_link) =
            generate_approval_links(run_id, step_id, &secret, &base_url)?;

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
        let env = Environment::new();
        let rendered_body = env
            .render_str(&spec.body, template_ctx)
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
    run_id: Uuid,
    step_id: Uuid,
    secret: &str,
    base_url: &str,
) -> Result<(String, String)> {
    let approve_token = crate::hitl::generate_approval_token(run_id, step_id, "approve", secret)?;
    let reject_token = crate::hitl::generate_approval_token(run_id, step_id, "reject", secret)?;

    let approve_link = format!("{}/api/v1/approve-link/{}", base_url, approve_token);
    let reject_link = format!("{}/api/v1/approve-link/{}", base_url, reject_token);
    Ok((approve_link, reject_link))
}

#[cfg(feature = "email")]
fn build_approval_mailer(spec: &EmailSpec) -> lettre::SmtpTransport {
    use lettre::SmtpTransport;
    let smtp_server = spec.smtp_server.clone().unwrap_or_else(|| {
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
            .clone()
            .or_else(|| std::env::var("SMTP_USERNAME").ok()),
        spec.smtp_password
            .clone()
            .or_else(|| std::env::var("SMTP_PASSWORD").ok()),
    ) {
        let credentials = lettre::transport::smtp::authentication::Credentials::new(user, pass);
        mailer_builder = mailer_builder.credentials(credentials);
    }

    mailer_builder.build()
}

#[cfg(all(test, feature = "email"))]
mod tests {
    use super::*;
    use stormchaser_model::dsl::{EmailBackend, EmailSpec};
    use uuid::Uuid;

    #[test]
    #[cfg(feature = "email")]
    fn test_generate_approval_links() {
        let run_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        let secret = "test-secret";
        let base_url = "https://paninfracon.net";

        let (approve, reject) = generate_approval_links(run_id, step_id, secret, base_url).unwrap();

        assert!(approve.starts_with(base_url));
        assert!(reject.starts_with(base_url));
        assert!(approve.contains("/api/v1/approve-link/"));
        assert!(reject.contains("/api/v1/approve-link/"));
        assert_ne!(approve, reject);
    }

    #[test]
    #[cfg(feature = "email")]
    fn test_build_approval_mailer_explicit() {
        let spec = EmailSpec {
            from: "sender@paninfracon.net".to_string(),
            to: vec!["receiver@paninfracon.net".to_string()],
            cc: None,
            bcc: None,
            subject: "Test".to_string(),
            body: "Hello".to_string(),
            html: None,
            backend: Some(EmailBackend::Smtp),
            smtp_server: Some("smtp.paninfracon.net".to_string()),
            smtp_port: Some(587),
            smtp_username: Some("user".to_string()),
            smtp_password: Some("pass".to_string()),
            smtp_use_tls: Some(true),
            smtp_use_mtls: None,
            ses_region: None,
            ses_role_arn: None,
            ses_configuration_set_name: None,
        };

        let _mailer = build_approval_mailer(&spec);
    }

    #[test]
    #[cfg(feature = "email")]
    fn test_build_approval_mailer_env_vars() {
        std::env::set_var("SMTP_SERVER", "env-smtp.paninfracon.net");
        std::env::set_var("SMTP_PORT", "2525");
        std::env::set_var("SMTP_USERNAME", "env-user");
        std::env::set_var("SMTP_PASSWORD", "env-pass");

        let spec = EmailSpec {
            from: "sender@paninfracon.net".to_string(),
            to: vec!["receiver@paninfracon.net".to_string()],
            cc: None,
            bcc: None,
            subject: "Test".to_string(),
            body: "Hello".to_string(),
            html: None,
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

        let _mailer = build_approval_mailer(&spec);

        std::env::remove_var("SMTP_SERVER");
        std::env::remove_var("SMTP_PORT");
        std::env::remove_var("SMTP_USERNAME");
        std::env::remove_var("SMTP_PASSWORD");
    }
}
