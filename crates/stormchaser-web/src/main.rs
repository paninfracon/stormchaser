#![recursion_limit = "512"]

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::{routing::get, routing::post, Router};
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use stormchaser_web::app::*;

    let conf = get_configuration(None).unwrap();
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;
    let routes = generate_route_list(App);

    let app = Router::new()
        .route("/api/v1/runs/stream", get(proxy_runs_stream_handler))
        .route(
            "/api/v1/runs/:id/status/stream",
            get(proxy_run_status_stream_handler),
        )
        .route(
            "/api/v1/runs/:run_id/steps/:step_id/logs/stream",
            get(proxy_step_logs_stream_handler),
        )
        .route("/api/*fn_name", post(leptos_axum::handle_server_fns))
        .route("/auth/login", get(auth_login_handler))
        .route("/auth/logout", get(auth_logout_handler))
        .route("/auth/callback", get(auth_callback_handler))
        .leptos_routes(&leptos_options, routes, {
            let leptos_options = leptos_options.clone();
            move || shell(leptos_options.clone())
        })
        .fallback(leptos_axum::file_and_error_handler(shell))
        .with_state(leptos_options);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("listening on http://{}", &addr);
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

#[cfg(feature = "ssr")]
fn http_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new).clone()
}

#[cfg(feature = "ssr")]
async fn auth_login_handler() -> impl axum::response::IntoResponse {
    use axum::http::header;
    use axum::response::IntoResponse;
    let external_api_url = std::env::var("EXTERNAL_API_URL")
        .or_else(|_| std::env::var("API_URL"))
        .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let web_url = std::env::var("WEB_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_string());
    let callback = urlencoding::encode(&format!("{}/auth/callback", web_url)).into_owned();

    let state = uuid::Uuid::new_v4().to_string();
    let state_cookie = format!(
        "oauth_state={}; Path=/; HttpOnly; SameSite=Lax; Secure; Max-Age=300",
        state
    );

    let redirect_url = format!(
        "{}/api/v1/auth/login?callback_url={}&state={}",
        external_api_url,
        callback,
        urlencoding::encode(&state)
    );

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        header::HeaderValue::from_str(&state_cookie).unwrap(),
    );
    headers.insert(
        header::LOCATION,
        header::HeaderValue::from_str(&redirect_url).unwrap(),
    );
    (axum::http::StatusCode::SEE_OTHER, headers).into_response()
}

#[cfg(feature = "ssr")]
async fn auth_logout_handler() -> impl axum::response::IntoResponse {
    use axum::http::header;
    use axum::response::IntoResponse;
    let cookie =
        "auth_token=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; SameSite=Lax";
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(header::SET_COOKIE, header::HeaderValue::from_static(cookie));
    headers.insert(header::LOCATION, header::HeaderValue::from_static("/"));
    (axum::http::StatusCode::SEE_OTHER, headers).into_response()
}

#[cfg(feature = "ssr")]
#[derive(serde::Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

#[cfg(feature = "ssr")]
async fn auth_callback_handler(
    axum::extract::Query(query): axum::extract::Query<CallbackQuery>,
    headers: axum::http::HeaderMap,
) -> impl axum::response::IntoResponse {
    use axum::http::header;
    use axum::response::IntoResponse;

    let cookie_str = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let mut oauth_state_cookie = None;
    for part in cookie_str.split(';') {
        let part = part.trim();
        if let Some(state_val) = part.strip_prefix("oauth_state=") {
            oauth_state_cookie = Some(state_val);
            break;
        }
    }

    if oauth_state_cookie.is_none()
        || query.state.is_none()
        || oauth_state_cookie != query.state.as_deref()
    {
        return axum::response::Redirect::to("/?error=auth_failed_state_mismatch").into_response();
    }

    if let Some(code) = query.code {
        let api_url =
            std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
        let web_url =
            std::env::var("WEB_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_string());

        let client = http_client();
        match client
            .post(format!("{}/api/v1/auth/exchange", api_url))
            .json(&serde_json::json!({
                "sso_token": code,
                "callback_url": format!("{}/auth/callback", web_url)
            }))
            .send()
            .await
        {
            Ok(res) if res.status().is_success() => {
                if let Ok(json) = res.json::<serde_json::Value>().await {
                    if let Some(access) = json.get("access_token").and_then(|v| v.as_str()) {
                        let cookie = format!(
                            "auth_token={}; Path=/; HttpOnly; SameSite=Lax; Secure",
                            access
                        );
                        let mut headers_res = axum::http::HeaderMap::new();
                        headers_res.insert(
                            header::SET_COOKIE,
                            header::HeaderValue::from_str(&cookie).unwrap(),
                        );
                        let clear_state_cookie = "oauth_state=; Path=/; HttpOnly; SameSite=Lax; Secure; Expires=Thu, 01 Jan 1970 00:00:00 GMT";
                        headers_res.append(
                            header::SET_COOKIE,
                            header::HeaderValue::from_static(clear_state_cookie),
                        );
                        headers_res.insert(header::LOCATION, header::HeaderValue::from_static("/"));
                        return (axum::http::StatusCode::SEE_OTHER, headers_res).into_response();
                    }
                }
            }
            _ => {}
        }
    }
    // Fallback error
    axum::response::Redirect::to("/?error=auth_failed").into_response()
}

#[cfg(feature = "ssr")]
async fn proxy_runs_stream_handler(
    headers: axum::http::HeaderMap,
) -> impl axum::response::IntoResponse {
    use axum::body::Body;
    use axum::http::{header, Response, StatusCode};
    use axum::response::IntoResponse;
    use futures::TryStreamExt;

    // Extract auth_token from Cookie
    let cookie_str = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    tracing::info!("SSE Proxy Hit: Extracted cookies: {}", cookie_str);

    let mut auth_token = None;
    for part in cookie_str.split(';') {
        let part = part.trim();
        if let Some(token) = part.strip_prefix("auth_token=") {
            auth_token = Some(token);
            break;
        }
    }

    let token = match auth_token {
        Some(t) => {
            tracing::info!("SSE Proxy: Auth token found");
            t
        }
        None => {
            tracing::error!("SSE Proxy: No active session found in cookies");
            return (StatusCode::UNAUTHORIZED, "No active session").into_response();
        }
    };

    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = http_client();
    match client
        .get(format!("{}/api/v1/runs/stream", api_url))
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
    {
        Ok(res) => {
            if !res.status().is_success() {
                return (res.status(), "Upstream error").into_response();
            }
            let stream = res.bytes_stream().map_err(std::io::Error::other);
            let body = Body::from_stream(stream);
            Response::builder()
                .header(header::CONTENT_TYPE, "text/event-stream")
                .header(header::CACHE_CONTROL, "no-cache")
                .body(body)
                .unwrap()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to connect").into_response(),
    }
}

#[cfg(feature = "ssr")]
async fn proxy_run_status_stream_handler(
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> impl axum::response::IntoResponse {
    use axum::body::Body;
    use axum::http::{header, Response, StatusCode};
    use axum::response::IntoResponse;
    use futures::TryStreamExt;

    let cookie_str = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let mut auth_token = None;
    for part in cookie_str.split(';') {
        let part = part.trim();
        if let Some(token) = part.strip_prefix("auth_token=") {
            auth_token = Some(token);
            break;
        }
    }
    let token = match auth_token {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "No active session").into_response(),
    };

    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = http_client();
    match client
        .get(format!("{}/api/v1/runs/{}/status/stream", api_url, id))
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
    {
        Ok(res) => {
            if !res.status().is_success() {
                return (res.status(), "Upstream error").into_response();
            }
            let stream = res.bytes_stream().map_err(std::io::Error::other);
            let body = Body::from_stream(stream);
            Response::builder()
                .header(header::CONTENT_TYPE, "text/event-stream")
                .header(header::CACHE_CONTROL, "no-cache")
                .body(body)
                .unwrap()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to connect").into_response(),
    }
}

#[cfg(feature = "ssr")]
async fn proxy_step_logs_stream_handler(
    axum::extract::Path((run_id, step_id)): axum::extract::Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> impl axum::response::IntoResponse {
    use axum::body::Body;
    use axum::http::{header, Response, StatusCode};
    use axum::response::IntoResponse;
    use futures::TryStreamExt;

    let cookie_str = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let mut auth_token = None;
    for part in cookie_str.split(';') {
        let part = part.trim();
        if let Some(token) = part.strip_prefix("auth_token=") {
            auth_token = Some(token);
            break;
        }
    }
    let token = match auth_token {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "No active session").into_response(),
    };

    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = http_client();
    match client
        .get(format!(
            "{}/api/v1/runs/{}/steps/{}/logs/stream",
            api_url, run_id, step_id
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
    {
        Ok(res) => {
            if !res.status().is_success() {
                return (res.status(), "Upstream error").into_response();
            }
            let stream = res.bytes_stream().map_err(std::io::Error::other);
            let body = Body::from_stream(stream);
            Response::builder()
                .header(header::CONTENT_TYPE, "text/event-stream")
                .header(header::CACHE_CONTROL, "no-cache")
                .body(body)
                .unwrap()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to connect").into_response(),
    }
}

#[cfg(not(feature = "ssr"))]
pub fn main() {}

#[cfg(test)]
#[cfg(feature = "ssr")]
mod tests {
    use super::*;
    use axum::http::header;
    use axum::response::IntoResponse;
    use futures::StreamExt;
    use wiremock::matchers::{header as wm_header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_proxy_runs_stream_handler() {
        let mock_server = MockServer::start().await;

        // Mock the SSE stream response from the API
        let sse_body = "event: workflow_run\ndata: {}\n\n";
        Mock::given(method("GET"))
            .and(path("/api/v1/runs/stream"))
            .and(wm_header("Authorization", "Bearer fake_token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .mount(&mock_server)
            .await;

        // Set the mock server as the API_URL
        std::env::set_var("API_URL", mock_server.uri());

        // Prepare the request with the auth_token cookie
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            header::COOKIE,
            header::HeaderValue::from_static("auth_token=fake_token"),
        );

        // Call the proxy handler
        let response = proxy_runs_stream_handler(headers).await.into_response();

        // Verify the response is an SSE stream
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );

        // Read the body
        let mut body_stream = response.into_body().into_data_stream();
        if let Some(Ok(bytes)) = body_stream.next().await {
            let body_str = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(body_str.contains("event: workflow_run"));
        } else {
            panic!("Expected body data");
        }
    }

    #[tokio::test]
    async fn test_auth_login_handler_sets_state() {
        let response = auth_login_handler().await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

        let headers = response.headers();
        let cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert!(cookie.contains("oauth_state="));
        assert!(cookie.contains("Secure"));

        let location = headers.get(header::LOCATION).unwrap().to_str().unwrap();
        assert!(location.contains("state="));
    }

    #[tokio::test]
    async fn test_auth_callback_rejects_missing_state() {
        let headers = axum::http::HeaderMap::new();
        // No cookie header

        let query = axum::extract::Query(CallbackQuery {
            code: Some("some_code".to_string()),
            state: Some("some_state".to_string()),
        });

        let response = auth_callback_handler(query, headers).await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
        let location = response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(location, "/?error=auth_failed_state_mismatch");
    }

    #[tokio::test]
    async fn test_auth_callback_rejects_mismatched_state() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            header::COOKIE,
            header::HeaderValue::from_static("oauth_state=cookie_state"),
        );

        let query = axum::extract::Query(CallbackQuery {
            code: Some("some_code".to_string()),
            state: Some("query_state".to_string()),
        });

        let response = auth_callback_handler(query, headers).await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
        let location = response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(location, "/?error=auth_failed_state_mismatch");
    }
}
