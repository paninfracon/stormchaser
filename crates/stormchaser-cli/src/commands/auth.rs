use crate::utils::handle_response;
use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::json;

#[derive(Subcommand)]
pub enum AuthCommands {
    /// Exchange an SSO token for a Stormchaser JWT
    Exchange { sso_token: String },
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
    }
    Ok(())
}

pub async fn handle_login(
    cli_url: &str,
    issuer: &str,
    client_id: &str,
    http_client: &reqwest_middleware::ClientWithMiddleware,
) -> Result<()> {
    let redirect_uri = "http://localhost:8080/callback";
    let auth_url = format!(
        "{}/auth?client_id={}&redirect_uri={}&response_type=code&scope=openid+profile+email",
        issuer.trim_end_matches('/'),
        client_id,
        redirect_uri
    );

    println!("Opening browser for authentication...");
    println!("If the browser does not open automatically, please visit:");
    println!("{}", auth_url);

    if let Err(e) = open::that(&auth_url) {
        eprintln!("Failed to open browser: {}", e);
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await
        .context(
            "Failed to bind callback server on port 8080. Is another login process running?",
        )?;

    let (mut stream, _) = listener
        .accept()
        .await
        .context("Failed to accept callback connection")?;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = [0; 4096];
    let mut request_str = String::new();
    if let Ok(n) = stream.read(&mut buf).await {
        request_str = String::from_utf8_lossy(&buf[0..n]).to_string();
    }

    let mut code = None;
    for line in request_str.lines() {
        if line.starts_with("GET /callback") {
            if let Some(query) = line
                .split_whitespace()
                .nth(1)
                .and_then(|p| p.split('?').nth(1))
            {
                for pair in query.split('&') {
                    if let Some((k, v)) = pair.split_once('=') {
                        if k == "code" {
                            code = Some(v.to_string());
                        }
                    }
                }
            }
            break;
        }
    }

    let response =
        "HTTP/1.1 200 OK\r\nContent-Length: 44\r\n\r\nLogin successful. You can close this window.";
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;

    let code = match code {
        Some(c) => c,
        None => {
            anyhow::bail!("Failed to parse authorization code from callback. Response may have been an error.");
        }
    };

    println!("Exchanging authorization code for token...");

    // The client uses the standard reqwest client, not the wrapped one, for this specific call because it's external
    let client = reqwest::Client::new();
    let token_res = client
        .post(format!("{}/token", issuer.trim_end_matches('/')))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("client_secret", "stormchaser-cli-secret"),
            ("redirect_uri", redirect_uri),
            ("code", &code),
        ])
        .send()
        .await?;

    let sso_token = if token_res.status().is_success() {
        let json: serde_json::Value = token_res.json().await.unwrap_or_default();
        if let Some(id_token) = json.get("id_token").and_then(|v| v.as_str()) {
            id_token.to_string()
        } else {
            anyhow::bail!("Dex response missing id_token");
        }
    } else {
        anyhow::bail!("Dex token exchange failed: {}", token_res.status());
    };

    println!("Exchanging SSO token for Stormchaser JWT...");
    let res = http_client
        .post(format!("{}/api/v1/auth/exchange", cli_url))
        .json(&json!({ "sso_token": sso_token }))
        .send()
        .await?;

    // To mirror typical CLI login experience, let's parse the response and print the token directly
    // so the user can easily export it
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if status.is_success() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body) {
            if let Some(access_token) = val.get("access_token").and_then(|t| t.as_str()) {
                println!("\nSuccessfully logged in! Export your token to use it:");
                println!("export STORMCHASER_TOKEN=\"{}\"", access_token);
            } else {
                println!("Success, but could not parse access_token from response.");
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or(body));
            }
        } else {
            println!("Success:\n{}", body);
        }
    } else {
        eprintln!("Error exchanging SSO token ({}): {}", status, body);
    }

    Ok(())
}
