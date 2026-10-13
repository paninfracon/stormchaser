#[cfg(feature = "email")]
use anyhow::Result;

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
/// The SMTP settings a step supplied in its own spec.
#[derive(Default)]
pub struct SmtpOverrides {
    pub server: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub use_tls: Option<bool>,
    pub use_mtls: Option<bool>,
}

#[cfg(feature = "email")]
/// The SMTP settings the operator gave the engine.
pub struct EngineSmtp {
    pub server: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub use_tls: bool,
    pub use_mtls: bool,
}

#[cfg(feature = "email")]
impl EngineSmtp {
    /// Reads the engine's SMTP settings from the `SMTP_*` environment variables.
    pub fn from_env() -> Self {
        use std::env::var;
        Self {
            server: var("SMTP_SERVER").unwrap_or_else(|_| "localhost".to_string()),
            port: var("SMTP_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(25),
            username: var("SMTP_USERNAME").ok(),
            password: var("SMTP_PASSWORD").ok(),
            use_tls: var("SMTP_USE_TLS").unwrap_or_default() == "true",
            use_mtls: var("SMTP_USE_MTLS").unwrap_or_default() == "true",
        }
    }
}

#[cfg(feature = "email")]
/// Combines a step's SMTP settings with the engine's.
///
/// The engine's username and password are only used with the engine's own server and port.
/// A step that names a different server must bring its own credentials, so a workflow
/// cannot have the engine's SMTP login sent to a server of the author's choosing. When the
/// engine's password is used, the step cannot turn off TLS that the operator turned on.
pub fn resolve_smtp_params(overrides: SmtpOverrides, engine: EngineSmtp) -> SmtpParams {
    let engine_server = overrides
        .server
        .as_deref()
        .is_none_or(|s| s.eq_ignore_ascii_case(&engine.server))
        && overrides.port.is_none_or(|p| p == engine.port);

    let (engine_username, engine_password) = if engine_server {
        (engine.username, engine.password)
    } else {
        (None, None)
    };
    let uses_engine_password = overrides.password.is_none() && engine_password.is_some();

    SmtpParams {
        server: overrides.server.unwrap_or(engine.server),
        port: overrides.port.unwrap_or(engine.port),
        username: overrides.username.or(engine_username),
        password: overrides.password.or(engine_password),
        use_tls: overrides.use_tls.unwrap_or(engine.use_tls)
            || (uses_engine_password && engine.use_tls),
        use_mtls: overrides.use_mtls.unwrap_or(engine.use_mtls),
    }
}

#[cfg(feature = "email")]
pub fn build_smtp_transport(params: SmtpParams) -> Result<lettre::SmtpTransport> {
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

#[cfg(all(test, feature = "email"))]
mod tests {
    use super::*;

    fn engine() -> EngineSmtp {
        EngineSmtp {
            server: "smtp.internal.example".to_string(),
            port: 587,
            username: Some("engine".to_string()),
            password: Some("engine-secret".to_string()),
            use_tls: true,
            use_mtls: false,
        }
    }

    #[test]
    fn engine_credentials_are_used_with_the_engine_server() {
        let params = resolve_smtp_params(SmtpOverrides::default(), engine());
        assert_eq!(params.server, "smtp.internal.example");
        assert_eq!(params.port, 587);
        assert_eq!(params.password.as_deref(), Some("engine-secret"));
        assert!(params.use_tls);

        let same = SmtpOverrides {
            server: Some("SMTP.internal.example".to_string()),
            port: Some(587),
            ..Default::default()
        };
        let params = resolve_smtp_params(same, engine());
        assert_eq!(params.username.as_deref(), Some("engine"));
        assert_eq!(params.password.as_deref(), Some("engine-secret"));
    }

    #[test]
    fn engine_credentials_are_not_sent_to_a_server_the_step_names() {
        let other_host = SmtpOverrides {
            server: Some("mail.attacker.example".to_string()),
            ..Default::default()
        };
        let params = resolve_smtp_params(other_host, engine());
        assert_eq!(params.server, "mail.attacker.example");
        assert!(params.username.is_none());
        assert!(params.password.is_none());

        let other_port = SmtpOverrides {
            port: Some(2525),
            ..Default::default()
        };
        let params = resolve_smtp_params(other_port, engine());
        assert!(params.username.is_none());
        assert!(params.password.is_none());
    }

    #[test]
    fn a_step_keeps_its_own_credentials_on_its_own_server() {
        let own = SmtpOverrides {
            server: Some("mail.team.example".to_string()),
            username: Some("team".to_string()),
            password: Some("team-secret".to_string()),
            use_tls: Some(false),
            ..Default::default()
        };
        let params = resolve_smtp_params(own, engine());
        assert_eq!(params.username.as_deref(), Some("team"));
        assert_eq!(params.password.as_deref(), Some("team-secret"));
        assert!(!params.use_tls);
    }

    #[test]
    fn a_step_cannot_turn_off_tls_for_the_engine_password() {
        let no_tls = SmtpOverrides {
            use_tls: Some(false),
            ..Default::default()
        };
        let params = resolve_smtp_params(no_tls, engine());
        assert_eq!(params.password.as_deref(), Some("engine-secret"));
        assert!(params.use_tls);
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
        let err = res.unwrap_err();
        assert_eq!(
            err.to_string(),
            "mTLS for SMTP is currently unsupported due to lettre limitations."
        );
    }
}
