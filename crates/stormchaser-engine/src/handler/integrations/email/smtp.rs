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
