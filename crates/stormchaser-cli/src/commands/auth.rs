use crate::utils::handle_response;
use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::json;
use serde_json::Value;

#[derive(Subcommand)]
pub enum AuthCommands {
    /// Exchange an SSO token for a Stormchaser JWT
    Exchange { sso_token: String },
    /// Manage multi-environment authentication profiles
    Profiles {
        #[command(subcommand)]
        command: crate::commands::auth_profiles::ProfilesCommands,
    },
}

pub async fn handle(
    url: &str,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    command: AuthCommands,
) -> Result<()> {
    match command {
        AuthCommands::Exchange { sso_token } => {
            let res = http_client
                .post(format!("{}/api/v1/auth/exchange", url))
                .json(&json!({ "sso_token": sso_token }))
                .send()
                .await?;
            handle_response(res).await?;
        }
        AuthCommands::Profiles { command } => {
            crate::commands::auth_profiles::handle(command).await?;
        }
    }
    Ok(())
}

/// How long `login` waits for the browser to come back.
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Logs in through the browser and stores the token in the profile for `cli_url`.
///
/// The CLI holds no client secret. It sends the browser to the API's login URL, waits
/// on a loopback port for the authorization code, and gives the code to the API, which
/// exchanges it. A random `state` ties the callback to this login and a PKCE verifier
/// ties the code to this process.
pub async fn handle_login(
    cli_url: &str,
    callback_port: u16,
    print_token: bool,
    http_client: &reqwest_middleware::ClientWithMiddleware,
) -> Result<()> {
    use stormchaser_model::login_flow::{wait_for_code, LoginAttempt};

    // Listen before the browser opens, so the port is ours when the callback arrives
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", callback_port))
        .await
        .with_context(|| {
            format!(
                "Failed to bind the callback port {}. Is another login in progress?",
                callback_port
            )
        })?;
    let callback_url = format!("http://localhost:{}/callback", callback_port);

    let attempt = LoginAttempt::new();
    let login_url = attempt.login_url(cli_url, &callback_url);

    println!("Opening browser for authentication...");
    println!("If the browser does not open automatically, please visit:");
    println!("{}", login_url);
    if let Err(e) = open::that(&login_url) {
        eprintln!("Failed to open browser: {}", e);
    }

    let code = wait_for_code(&listener, &attempt, LOGIN_TIMEOUT).await?;
    drop(listener);

    let res = http_client
        .post(format!(
            "{}/api/v1/auth/exchange",
            cli_url.trim_end_matches('/')
        ))
        .json(&json!({
            "sso_token": code,
            "callback_url": callback_url,
            "code_verifier": attempt.code_verifier,
        }))
        .send()
        .await?;

    let status = res.status();
    if !status.is_success() {
        anyhow::bail!("Login failed: the API answered {}", status);
    }
    let body: Value = res.json().await.context("Login response was not JSON")?;
    let access_token = body
        .get("access_token")
        .and_then(|t| t.as_str())
        .context("Login response had no access_token")?;

    if print_token {
        println!("{}", access_token);
        return Ok(());
    }
    let profile = crate::commands::auth_profiles::store_token(cli_url, access_token)?;
    println!(
        "Logged in. The token is stored in profile '{}', which is now active.",
        profile
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_auth_exchange() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/exchange"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "test"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = AuthCommands::Exchange {
            sso_token: "test_sso".to_string(),
        };

        let result = handle(&server.uri(), &client, cmd).await;
        result.unwrap();
    }
}
