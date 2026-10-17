//! The browser login that the CLI and the TUI share.
//!
//! Both open the API's login URL, wait on a loopback port for the identity provider to
//! send the browser back with an authorization code, and hand that code to the API, which
//! holds the client secret and exchanges it. Neither client holds a secret of its own.
//!
//! Each attempt carries a random `state`, so a callback that some other page or program
//! sends to the loopback port is not mistaken for the answer to this login, and a PKCE
//! verifier, so a code that is intercepted on its way back cannot be redeemed by anyone
//! else.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// The secrets of one login attempt.
#[derive(Debug, Clone)]
pub struct LoginAttempt {
    /// Echoed back by the identity provider; ties a callback to this attempt.
    pub state: String,
    /// Kept by the client and sent with the code; proves the code is ours to redeem.
    pub code_verifier: String,
}

impl Default for LoginAttempt {
    fn default() -> Self {
        Self::new()
    }
}

impl LoginAttempt {
    /// Starts an attempt with fresh random values.
    pub fn new() -> Self {
        let random = || uuid::Uuid::new_v4().simple().to_string();
        Self {
            state: random(),
            // 64 characters from the unreserved set, within PKCE's 43 to 128
            code_verifier: format!("{}{}", random(), random()),
        }
    }

    /// The S256 challenge for this attempt's verifier.
    pub fn code_challenge(&self) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(self.code_verifier.as_bytes()))
    }

    /// The URL to open in the browser.
    pub fn login_url(&self, api_url: &str, callback_url: &str) -> String {
        format!(
            "{}/api/v1/auth/login?callback_url={}&state={}&code_challenge={}",
            api_url.trim_end_matches('/'),
            urlencoding::encode(callback_url),
            self.state,
            self.code_challenge()
        )
    }
}

/// What a request to the loopback listener turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callback {
    /// The identity provider's answer to this attempt, with the authorization code.
    Code(String),
    /// The identity provider's answer to this attempt was an error.
    ProviderError(String),
    /// Not the callback for this attempt: another path, or a missing or wrong `state`.
    /// The listener should answer and keep waiting.
    Ignored,
}

/// Reads the first request line of an HTTP request sent to the loopback listener.
pub fn parse_callback(request: &str, expected_state: &str) -> Callback {
    let mut parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else {
        return Callback::Ignored;
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/callback" {
        return Callback::Ignored;
    }
    let Ok(params) = parse_query(query) else {
        return Callback::Ignored;
    };
    if params.get("state").map(String::as_str) != Some(expected_state) {
        return Callback::Ignored;
    }
    if let Some(error) = params.get("error") {
        let description = params.get("error_description").cloned().unwrap_or_default();
        return Callback::ProviderError(format!("{} {}", error, description).trim().to_string());
    }
    match params.get("code") {
        Some(code) if !code.is_empty() => Callback::Code(code.clone()),
        _ => Callback::Ignored,
    }
}

fn parse_query(query: &str) -> Result<HashMap<String, String>, ()> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let decode = |s: &str| {
                urlencoding::decode(&s.replace('+', " "))
                    .map(|decoded| decoded.into_owned())
                    .map_err(|_| ())
            };
            Ok((decode(key)?, decode(value)?))
        })
        .collect()
}

/// The page shown in the browser once the callback has been handled.
pub fn browser_response(callback: &Callback) -> String {
    let (status, body) = match callback {
        Callback::Code(_) => ("200 OK", "Login successful. You can close this window."),
        Callback::ProviderError(_) => ("400 Bad Request", "Login failed. Return to the terminal."),
        Callback::Ignored => ("400 Bad Request", "This is not the login that is waiting."),
    };
    format!(
        "HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        body.len(),
        body
    )
}

/// Waits on the loopback listener for this attempt's callback and returns its code.
///
/// Requests that are not the callback (a browser asking for a favicon, a port scan, a
/// page guessing at the port) are answered and skipped, so they can neither complete the
/// login nor make it fail. Gives up after `timeout`.
pub async fn wait_for_code(
    listener: &tokio::net::TcpListener,
    attempt: &LoginAttempt,
    timeout: std::time::Duration,
) -> anyhow::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let wait = async {
        loop {
            let (mut stream, _) = listener.accept().await?;
            let mut buf = [0; 8192];
            // A client that connects and sends nothing must not hold the login up
            let read =
                tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut buf));
            let request = match read.await {
                Ok(Ok(n)) => String::from_utf8_lossy(&buf[..n]).into_owned(),
                _ => continue,
            };
            let callback = parse_callback(&request, &attempt.state);
            let _ = stream
                .write_all(browser_response(&callback).as_bytes())
                .await;
            let _ = stream.shutdown().await;
            match callback {
                Callback::Code(code) => return Ok(code),
                Callback::ProviderError(error) => {
                    anyhow::bail!("The identity provider refused the login: {}", error)
                }
                Callback::Ignored => continue,
            }
        }
    };
    match tokio::time::timeout(timeout, wait).await {
        Ok(result) => result,
        Err(_) => anyhow::bail!("Timed out waiting for the login to complete"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_attempt_has_its_own_state_and_a_valid_pkce_pair() {
        let a = LoginAttempt::new();
        let b = LoginAttempt::new();
        assert_ne!(a.state, b.state);
        assert_ne!(a.code_verifier, b.code_verifier);
        assert_eq!(a.code_verifier.len(), 64);
        // 32 bytes of SHA-256 in unpadded base64url
        assert_eq!(a.code_challenge().len(), 43);
        assert!(!a.code_challenge().contains(['=', '+', '/']));
    }

    #[test]
    fn the_challenge_matches_the_rfc_7636_example() {
        let attempt = LoginAttempt {
            state: String::new(),
            code_verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_string(),
        };
        assert_eq!(
            attempt.code_challenge(),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn only_the_callback_for_this_attempt_is_accepted() {
        let ok = "GET /callback?code=abc%2F123&state=s1 HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert_eq!(parse_callback(ok, "s1"), Callback::Code("abc/123".into()));

        for request in [
            // Another attempt's state, or none
            "GET /callback?code=abc&state=other HTTP/1.1\r\n\r\n",
            "GET /callback?code=abc HTTP/1.1\r\n\r\n",
            // Something else asking the port
            "GET /favicon.ico HTTP/1.1\r\n\r\n",
            "GET /callbackx?code=abc&state=s1 HTTP/1.1\r\n\r\n",
            "POST /callback?code=abc&state=s1 HTTP/1.1\r\n\r\n",
            "GET /callback?state=s1 HTTP/1.1\r\n\r\n",
            "",
        ] {
            assert_eq!(
                parse_callback(request, "s1"),
                Callback::Ignored,
                "{request}"
            );
        }
    }

    #[test]
    fn a_provider_error_for_this_attempt_ends_it() {
        let request =
            "GET /callback?error=access_denied&error_description=No+thanks&state=s1 HTTP/1.1\r\n";
        assert_eq!(
            parse_callback(request, "s1"),
            Callback::ProviderError("access_denied No thanks".into())
        );
    }

    #[test]
    fn the_login_url_carries_state_and_challenge() {
        let attempt = LoginAttempt::new();
        let url = attempt.login_url("http://api:3000/", "http://localhost:8080/callback");
        assert!(url.starts_with(
            "http://api:3000/api/v1/auth/login?callback_url=http%3A%2F%2Flocalhost%3A8080%2Fcallback&state="
        ));
        assert!(url.contains(&format!("&state={}", attempt.state)));
        assert!(url.ends_with(&format!("&code_challenge={}", attempt.code_challenge())));
        assert!(!url.contains(&attempt.code_verifier));
    }

    #[tokio::test]
    async fn the_listener_skips_strangers_and_returns_the_code() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let attempt = LoginAttempt::new();
        let state = attempt.state.clone();

        let client = tokio::spawn(async move {
            let mut answers = Vec::new();
            for target in [
                "/favicon.ico".to_string(),
                "/callback?code=stolen&state=guess".to_string(),
                format!("/callback?code=real&state={state}"),
            ] {
                let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
                stream
                    .write_all(
                        format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
                    )
                    .await
                    .unwrap();
                let mut answer = String::new();
                stream.read_to_string(&mut answer).await.unwrap();
                answers.push(answer);
            }
            answers
        });

        let code = wait_for_code(&listener, &attempt, std::time::Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(code, "real");
        let answers = client.await.unwrap();
        assert!(answers[0].starts_with("HTTP/1.1 400"));
        assert!(answers[1].starts_with("HTTP/1.1 400"));
        assert!(answers[2].starts_with("HTTP/1.1 200"));
    }

    #[tokio::test]
    async fn the_listener_gives_up_after_the_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let attempt = LoginAttempt::new();
        let result = wait_for_code(&listener, &attempt, std::time::Duration::from_millis(50)).await;
        assert!(result.is_err());
    }
}
