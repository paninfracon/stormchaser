use crate::utils::print_json;
use anyhow::{Context, Result};
use clap::Subcommand;
use stormchaser_client::Client;
use stormchaser_model::api::AuthExchangeRequest;

#[derive(Subcommand)]
pub enum AuthCommands {
    /// Exchange an authorization code for a Stormchaser JWT
    Exchange {
        /// The authorization code the identity provider returned
        sso_token: String,
        /// The redirect URI the code was issued for
        #[arg(long)]
        callback_url: String,
        /// The PKCE verifier whose challenge the login was started with
        #[arg(long)]
        code_verifier: String,
    },
    /// Manage multi-environment authentication profiles
    Profiles {
        #[command(subcommand)]
        command: crate::commands::auth_profiles::ProfilesCommands,
    },
}

pub async fn handle(client: &Client, command: AuthCommands) -> Result<()> {
    match command {
        AuthCommands::Exchange {
            sso_token,
            callback_url,
            code_verifier,
        } => {
            let response = client
                .exchange_token(&AuthExchangeRequest {
                    sso_token,
                    callback_url,
                    code_verifier: Some(code_verifier),
                })
                .await?;
            print_json(&response)?;
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
    client: &Client,
    cli_url: &str,
    callback_port: u16,
    print_token: bool,
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

    let response = client
        .exchange_token(&AuthExchangeRequest {
            sso_token: code,
            callback_url,
            code_verifier: Some(attempt.code_verifier),
        })
        .await;
    let access_token = match response {
        Ok(response) => response.access_token,
        Err(e) => match e.status() {
            Some(status) => anyhow::bail!("Login failed: the API answered {}", status),
            None => return Err(e).context("Login failed"),
        },
    };

    if print_token {
        println!("{}", access_token);
        return Ok(());
    }
    let profile = crate::commands::auth_profiles::store_token(cli_url, &access_token)?;
    println!(
        "Logged in. The token is stored in profile '{}', which is now active.",
        profile
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_auth_exchange() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/exchange"))
            .and(body_json(json!({
                "sso_token": "test_sso",
                "callback_url": "http://localhost:8080/callback",
                "code_verifier": "verifier",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "test",
                "refresh_token": null,
                "token_type": "Bearer",
                "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let client = Client::builder(server.uri()).build().unwrap();
        let cmd = AuthCommands::Exchange {
            sso_token: "test_sso".to_string(),
            callback_url: "http://localhost:8080/callback".to_string(),
            code_verifier: "verifier".to_string(),
        };

        let result = handle(&client, cmd).await;
        result.unwrap();
    }
}
