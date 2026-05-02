use super::*;
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn handle_oauth_callback(
    listener: tokio::net::TcpListener,
    tx: tokio::sync::mpsc::Sender<AppEvent>,
    url: String,
    callback_url: String,
) {
    let (mut stream, _) = match tokio::time::timeout(
        Duration::from_secs(300), // 5 minute timeout for user to login
        listener.accept(),
    )
    .await
    {
        Ok(Ok(s)) => s,
        _ => {
            let _ = tx
                .send(AppEvent::LoginFailed(
                    "Login timed out or failed to accept connection".to_string(),
                ))
                .await;
            return;
        }
    };

    let mut buf = [0; 4096];
    let mut request_str = String::new();
    if let Ok(n) = stream.read(&mut buf).await {
        request_str = String::from_utf8_lossy(&buf[0..n]).to_string();
    }

    let mut code = None;
    if let Some(path) = request_str.split_whitespace().nth(1) {
        if let Some(query) = path.split('?').nth(1) {
            if let Ok(params) = serde_urlencoded::from_str::<HashMap<String, String>>(query) {
                code = params.get("code").cloned();
            }
        }
    }

    // Respond to browser
    let response = "HTTP/1.1 200 OK
Content-Type: text/html

<html><body><h1>Login Successful</h1><p>You can close this window now.</p><script>window.close();</script></body></html>";
    let _ = stream.write_all(response.as_bytes()).await;

    if let Some(code) = code {
        let client = reqwest::Client::new();
        match client
            .post(format!("{}/api/v1/auth/exchange", url))
            .json(&serde_json::json!({
                "sso_token": code,
                "callback_url": callback_url
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
    } else {
        let _ = tx
            .send(AppEvent::LoginFailed(
                "No authorization code received from provider".to_string(),
            ))
            .await;
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

        let login_url = format!(
            "{}/api/v1/auth/login?callback_url={}",
            self.url,
            urlencoding::encode(&callback_url)
        );

        if let Err(e) = open::that(&login_url) {
            self.error = Some(format!("Failed to open browser: {}", e));
            return Ok(());
        }

        let url = self.url.clone();
        let tx = self.status_tx.clone();

        tokio::spawn(async move {
            handle_oauth_callback(listener, tx, url, callback_url).await;
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

        let login_url = format!(
            "{}/api/v1/auth/login?callback_url={}",
            self.url,
            urlencoding::encode(&callback_url)
        );

        let tx = self.status_tx.clone();
        let email_str = email.to_string();
        let password_str = password.to_string();
        let url = self.url.clone();

        tokio::spawn(async move {
            handle_oauth_callback(listener, tx, url, callback_url).await;
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
    let client = reqwest::Client::builder().cookie_store(true).build()?;

    let res1 = client.get(login_url).send().await?;
    let url1 = res1.url().clone();
    let html = res1.text().await?;

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
        let req_end = html2[req_start..].find('"').unwrap() + req_start;
        let req_val = &html2[req_start..req_end];

        let _res3 = client
            .post(url2)
            .form(&[("approval", "approve"), ("req", req_val)])
            .send()
            .await?;
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
        let mut app = App::new(server.uri(), None, tx);
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
        let mut app = App::new(server.uri(), None, tx);
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

        // Spawn the callback handler
        tokio::spawn(async move {
            handle_oauth_callback(listener, tx, url, callback_url).await;
        });

        // Simulate the browser hitting the callback endpoint
        let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .unwrap();
        let request = "GET /callback?code=mock_auth_code HTTP/1.1\r\nHost: localhost\r\n\r\n";
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
