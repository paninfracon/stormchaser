use super::*;
use std::time::Duration;
use stormchaser_model::login_flow::{wait_for_code, LoginAttempt};

async fn handle_oauth_callback(
    listener: tokio::net::TcpListener,
    tx: tokio::sync::mpsc::Sender<AppEvent>,
    url: String,
    callback_url: String,
    attempt: LoginAttempt,
) {
    // Only the callback that carries this attempt's state is accepted; anything else
    // that reaches the port is answered and ignored. Five minutes for the user to log in.
    let code = match wait_for_code(&listener, &attempt, Duration::from_secs(300)).await {
        Ok(code) => code,
        Err(e) => {
            let _ = tx.send(AppEvent::LoginFailed(e.to_string())).await;
            return;
        }
    };
    drop(listener);

    let client = reqwest::Client::new();
    match client
        .post(format!("{}/api/v1/auth/exchange", url))
        .json(&serde_json::json!({
            "sso_token": code,
            "callback_url": callback_url,
            "code_verifier": attempt.code_verifier,
        }))
        .send()
        .await
    {
        Ok(res) => {
            if res.status().is_success() {
                if let Ok(auth_res) = res.json::<crate::app::AuthExchangeResponse>().await {
                    let _ = tx
                        .send(AppEvent::LoginSuccessful(
                            auth_res.access_token,
                            auth_res.refresh_token,
                        ))
                        .await;
                } else {
                    let _ = tx
                        .send(AppEvent::LoginFailed(
                            "Failed to parse token response".to_string(),
                        ))
                        .await;
                }
            } else {
                let _ = tx
                    .send(AppEvent::LoginFailed(format!(
                        "Token exchange failed: {}",
                        res.status()
                    )))
                    .await;
            }
        }
        Err(e) => {
            let _ = tx
                .send(AppEvent::LoginFailed(format!("Request failed: {}", e)))
                .await;
        }
    }
}

impl<'a> App<'a> {
    /// Initiates the OAuth login flow, opens a browser, and waits for the callback.
    pub async fn login(&mut self) -> Result<()> {
        self.state = AppState::LoggingIn;
        self.error = None;
        let port = 8080;
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
        let callback_url = format!("http://localhost:{}/callback", port);

        let attempt = LoginAttempt::new();
        let login_url = attempt.login_url(&self.url, &callback_url);

        if let Err(e) = open::that(&login_url) {
            self.error = Some(format!("Failed to open browser: {}", e));
            return Ok(());
        }

        let url = self.url.clone();
        let tx = self.status_tx.clone();

        tokio::spawn(async move {
            handle_oauth_callback(listener, tx, url, callback_url, attempt).await;
        });

        Ok(())
    }

    /// Automatically logs in using the provided email and password by simulating a browser flow.
    pub async fn auto_login(&mut self, email: &str, password: &str) -> Result<()> {
        self.state = AppState::LoggingIn;
        self.error = None;
        let port = 8080;
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
        let callback_url = format!("http://localhost:{}/callback", port);

        let attempt = LoginAttempt::new();
        let login_url = attempt.login_url(&self.url, &callback_url);

        let tx = self.status_tx.clone();
        let email_str = email.to_string();
        let password_str = password.to_string();
        let url = self.url.clone();

        tokio::spawn(async move {
            handle_oauth_callback(listener, tx, url, callback_url, attempt).await;
        });

        let tx2 = self.status_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = simulate_browser_login(&login_url, &email_str, &password_str).await {
                let _ = tx2
                    .send(AppEvent::LoginFailed(format!("Auto-login failed: {}", e)))
                    .await;
            }
        });

        Ok(())
    }

    /// Refreshes the access token using the stored refresh token.
    pub async fn refresh_session(&mut self) -> Result<bool> {
        if let Some(refresh_token) = self.refresh_token.clone() {
            let res = self
                .api_request(
                    reqwest::Method::POST,
                    "/api/v1/auth/refresh",
                    Some(serde_json::json!({ "refresh_token": refresh_token })),
                )
                .await?;

            if res.status().is_success() {
                let auth_res: crate::app::AuthExchangeResponse = res.json().await?;
                self.token = Some(auth_res.access_token);
                if auth_res.refresh_token.is_some() {
                    self.refresh_token = auth_res.refresh_token;
                }
                self.state = AppState::LoggedIn;
                return Ok(true);
            }
        }

        self.token = None;
        self.refresh_token = None;
        self.state = AppState::LoggedOut;
        Ok(false)
    }
}

async fn simulate_browser_login(login_url: &str, email: &str, password: &str) -> Result<()> {
    let client_no_redirect = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    let res_redirect = client_no_redirect.get(login_url).send().await?;

    let mut dex_auth_url = res_redirect
        .headers()
        .get(reqwest::header::LOCATION)
        .ok_or_else(|| anyhow::anyhow!("No location header in login response"))?
        .to_str()?
        .to_string();

    if let Ok(host_dex) = std::env::var("HOST_DEX") {
        if dex_auth_url.starts_with("http://localhost:") {
            dex_auth_url = dex_auth_url.replacen("localhost", &host_dex, 1);
        } else if dex_auth_url.starts_with("http://127.0.0.1:") {
            dex_auth_url = dex_auth_url.replacen("127.0.0.1", &host_dex, 1);
        }
    }

    let client = reqwest::Client::builder().cookie_store(true).build()?;

    let res1 = client.get(&dex_auth_url).send().await?;
    let url1 = res1.url().clone();
    let html = res1.text().await?;

    if html.contains("Login Successful") {
        // reqwest followed the redirect back to our local listener successfully
        // without needing to submit a login form (e.g. user was remembered).
        return Ok(());
    }

    let action_start = html
        .find("action=\"")
        .ok_or_else(|| anyhow::anyhow!("No action found in Dex response"))?
        + 8;
    let action_end = html[action_start..]
        .find('"')
        .ok_or_else(|| anyhow::anyhow!("No action end found"))?
        + action_start;
    let action = &html[action_start..action_end].replace("&amp;", "&");

    let base_url = format!(
        "{}://{}",
        url1.scheme(),
        url1.host_str().unwrap_or("127.0.0.1")
    );
    let port = url1.port().map(|p| format!(":{}", p)).unwrap_or_default();
    let dex_login_url = format!("{}{}{}", base_url, port, action);

    let res2 = client
        .post(&dex_login_url)
        .form(&[("login", email), ("password", password)])
        .send()
        .await?;

    let url2 = res2.url().clone();
    let html2 = res2.text().await?;

    if html2.contains("Grant Access") {
        let req_start = html2
            .find("name=\"req\" value=\"")
            .ok_or_else(|| anyhow::anyhow!("No req found"))?
            + 18;
        let req_end = html2[req_start..]
            .find('"')
            .ok_or_else(|| anyhow::anyhow!("No req value end found in Dex approval response"))?
            + req_start;
        let req_val = &html2[req_start..req_end];

        let _res3 = client
            .post(url2)
            .form(&[("approval", "approve"), ("req", req_val)])
            .send()
            .await?;
    } else if html2.contains("Login Successful") {
        // reqwest followed the redirect back to our local listener successfully
        return Ok(());
    } else if html2.contains("Invalid login or password") || !html2.contains("Log in to") {
        return Err(anyhow::anyhow!(
            "Invalid login credentials or unexpected Dex response"
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::mpsc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_refresh_session_success() {
        let server = MockServer::start().await;

        let response = crate::app::AuthExchangeResponse {
            access_token: "new_access_token".to_string(),
            refresh_token: Some("new_refresh_token".to_string()),
        };

        Mock::given(method("POST"))
            .and(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(server.uri(), "http://localhost:3001".to_string(), None, tx);
        app.refresh_token = Some("old_refresh_token".to_string());

        let result = app.refresh_session().await.unwrap();
        assert!(result);
        assert_eq!(app.token, Some("new_access_token".to_string()));
        assert_eq!(app.refresh_token, Some("new_refresh_token".to_string()));
        assert_eq!(app.state, AppState::LoggedIn);
    }

    #[tokio::test]
    async fn test_refresh_session_failure() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(server.uri(), "http://localhost:3001".to_string(), None, tx);
        app.refresh_token = Some("old_refresh_token".to_string());

        let result = app.refresh_session().await.unwrap();
        assert!(!result);
        assert_eq!(app.token, None);
        assert_eq!(app.refresh_token, None);
        assert_eq!(app.state, AppState::LoggedOut);
    }

    #[tokio::test]
    async fn test_handle_oauth_callback_success() {
        let server = MockServer::start().await;

        let response = crate::app::AuthExchangeResponse {
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
        };

        Mock::given(method("POST"))
            .and(path("/api/v1/auth/exchange"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let (tx, mut rx) = mpsc::channel(10);
        let url = server.uri();
        let callback_url = format!("http://localhost:{}/callback", port);
        let attempt = LoginAttempt::new();
        let state = attempt.state.clone();

        // Spawn the callback handler
        tokio::spawn(async move {
            handle_oauth_callback(listener, tx, url, callback_url, attempt).await;
        });

        // Simulate the browser hitting the callback endpoint
        let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .unwrap();
        let request = format!(
            "GET /callback?code=mock_auth_code&state={state} HTTP/1.1\r\nHost: localhost\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await.unwrap();

        // Wait for the event from the tx
        if let Some(event) = rx.recv().await {
            match event {
                AppEvent::LoginSuccessful(access, refresh) => {
                    assert_eq!(access, "access");
                    assert_eq!(refresh, Some("refresh".to_string()));
                }
                _ => panic!("Expected LoginSuccessful event"),
            }
        } else {
            panic!("Channel closed without sending event");
        }
    }
}
