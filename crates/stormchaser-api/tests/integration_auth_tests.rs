use axum::extract::connect_info::ConnectInfo;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Once;
use stormchaser_api::auth::jwks::OidcConfig;
use stormchaser_api::{app, AppState};
use stormchaser_model::auth::OpaClient;
use tower::ServiceExt;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

static INIT: Once = Once::new();

fn init_test() {
    INIT.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("Failed to install default crypto provider");
    });
}

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rsa::{pkcs8::EncodePrivateKey, traits::PublicKeyParts, RsaPrivateKey};
use std::sync::OnceLock;

static TEST_KEYS: OnceLock<(String, serde_json::Value)> = OnceLock::new();

fn get_test_keys() -> &'static (String, serde_json::Value) {
    TEST_KEYS.get_or_init(|| {
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate a key");
        let pem = priv_key
            .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
            .expect("failed to export pem")
            .to_string();

        let n_b64 = URL_SAFE_NO_PAD.encode(priv_key.n().to_bytes_be());
        let e_b64 = URL_SAFE_NO_PAD.encode(priv_key.e().to_bytes_be());

        let jwk = serde_json::json!({
            "kty": "RSA",
            "n": n_b64,
            "e": e_b64,
            "kid": "test-kid",
            "alg": "RS256",
            "use": "sig"
        });

        (pem, jwk)
    })
}

#[derive(Serialize)]
struct IdTokenClaims {
    sub: String,
    email: Option<String>,
    exp: usize,
    aud: String,
    iss: String,
}

fn create_id_token(issuer: &str, client_id: &str) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-kid".to_string());

    let claims = IdTokenClaims {
        sub: "test-user-id".to_string(),
        email: Some("test@paninfracon.net".to_string()),
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        aud: client_id.to_string(),
        iss: issuer.to_string(),
    };

    let (pem, _) = get_test_keys();
    let key = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
    encode(&header, &claims, &key).unwrap()
}

async fn setup_app(mock_server_url: String) -> Option<axum::Router> {
    init_test();
    std::env::set_var("CRON_ENGINE", "none");
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.ok()?;

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .ok()?;

    Some(app(AppState {
        pool,
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: Some(OidcConfig {
            issuer: mock_server_url.clone(),
            external_issuer: mock_server_url.clone(),
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
            jwks_url: format!("{}/.well-known/jwks.json", mock_server_url),
        }),
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
    }))
}

#[tokio::test]
async fn test_auth_login_redirect() {
    let mock_server = MockServer::start().await;
    let app = setup_app(mock_server.uri()).await.unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/login?callback_url=http%3A%2F%2Flocalhost%3A3000%2Fcallback")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.starts_with(&format!("{}/auth?", mock_server.uri())));
    assert!(location.contains("client_id=test-client"));
    assert!(location.contains("redirect_uri=http%3A%2F%2Flocalhost%3A3000%2Fcallback"));
}

#[tokio::test]
async fn test_auth_exchange_success() {
    let mock_server = MockServer::start().await;

    // Mock JWKS endpoint
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [get_test_keys().1.clone()]
        })))
        .mount(&mock_server)
        .await;

    // Mock Token endpoint
    let id_token = create_id_token(&mock_server.uri(), "test-client");
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token": id_token,
            "refresh_token": "dummy-refresh-token"
        })))
        .mount(&mock_server)
        .await;

    let app = setup_app(mock_server.uri()).await.unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/exchange")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "sso_token": "dummy-auth-code",
                        "callback_url": "http://localhost:3000/callback"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert!(body.get("access_token").is_some());
    assert_eq!(
        body.get("refresh_token").unwrap().as_str().unwrap(),
        "dummy-refresh-token"
    );
    assert_eq!(body.get("token_type").unwrap().as_str().unwrap(), "Bearer");
}

#[tokio::test]
async fn test_auth_exchange_failure() {
    let mock_server = MockServer::start().await;

    // Mock Token endpoint to return an error (invalid code)
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant"
        })))
        .mount(&mock_server)
        .await;

    let app = setup_app(mock_server.uri()).await.unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/exchange")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "sso_token": "invalid-auth-code",
                        "callback_url": "http://localhost:3000/callback"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_refresh_success() {
    let mock_server = MockServer::start().await;

    // Mock JWKS endpoint
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [get_test_keys().1.clone()]
        })))
        .mount(&mock_server)
        .await;

    // Mock Token endpoint
    let id_token = create_id_token(&mock_server.uri(), "test-client");
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token": id_token,
            "refresh_token": "new-dummy-refresh-token"
        })))
        .mount(&mock_server)
        .await;

    let app = setup_app(mock_server.uri()).await.unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "refresh_token": "old-dummy-refresh-token"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert!(body.get("access_token").is_some());
    assert_eq!(
        body.get("refresh_token").unwrap().as_str().unwrap(),
        "new-dummy-refresh-token"
    );
    assert_eq!(body.get("token_type").unwrap().as_str().unwrap(), "Bearer");
}

#[tokio::test]
async fn test_auth_exchange_network_error() {
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .unwrap();
    let app = stormchaser_api::app(AppState {
        pool,
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: Some(OidcConfig {
            issuer: "http://localhost:1".to_string(), // Invalid, connection refused
            external_issuer: "http://localhost:1".to_string(),
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
            jwks_url: "http://localhost:1/.well-known/jwks.json".to_string(),
        }),
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
    });

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/exchange")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "sso_token": "dummy-auth-code",
                        "callback_url": "http://localhost:3000/callback"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_exchange_invalid_json() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&mock_server)
        .await;
    let app = setup_app(mock_server.uri()).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/exchange")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "sso_token": "dummy-auth-code",
                        "callback_url": "http://localhost:3000/callback"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_exchange_invalid_id_token_header() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token": "not.a.jwt",
            "refresh_token": "dummy-refresh-token"
        })))
        .mount(&mock_server)
        .await;
    let app = setup_app(mock_server.uri()).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/exchange")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "sso_token": "dummy-auth-code",
                        "callback_url": "http://localhost:3000/callback"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_exchange_no_kid() {
    let mock_server = MockServer::start().await;
    let mut header = Header::new(Algorithm::RS256);
    header.kid = None; // No kid
    let claims = IdTokenClaims {
        sub: "test".to_string(),
        email: None,
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        aud: "test-client".to_string(),
        iss: mock_server.uri().to_string(),
    };
    let key = EncodingKey::from_rsa_pem(get_test_keys().0.as_bytes()).unwrap();
    let token = encode(&header, &claims, &key).unwrap();

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token": token,
            "refresh_token": "dummy"
        })))
        .mount(&mock_server)
        .await;

    let app = setup_app(mock_server.uri()).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/exchange")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "sso_token": "dummy-auth-code",
                        "callback_url": "http://localhost:3000/callback"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_exchange_kid_not_found() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [] // Empty keys list
        })))
        .mount(&mock_server)
        .await;

    let token = create_id_token(&mock_server.uri(), "test-client");
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token": token,
            "refresh_token": "dummy"
        })))
        .mount(&mock_server)
        .await;

    let app = setup_app(mock_server.uri()).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/exchange")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "sso_token": "dummy-auth-code",
                        "callback_url": "http://localhost:3000/callback"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_exchange_invalid_jwk() {
    let mock_server = MockServer::start().await;
    let mut invalid_jwk = get_test_keys().1.clone();
    // Invalidate the RSA parameters to trigger decoding key failure
    invalid_jwk["n"] = json!("invalid-base64");

    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [invalid_jwk]
        })))
        .mount(&mock_server)
        .await;

    let token = create_id_token(&mock_server.uri(), "test-client");
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token": token,
            "refresh_token": "dummy"
        })))
        .mount(&mock_server)
        .await;

    let app = setup_app(mock_server.uri()).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/exchange")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "sso_token": "dummy-auth-code",
                        "callback_url": "http://localhost:3000/callback"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_exchange_invalid_signature() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [get_test_keys().1.clone()]
        })))
        .mount(&mock_server)
        .await;

    // Use an expired token to fail validation
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-kid".to_string());
    let claims = IdTokenClaims {
        sub: "test".to_string(),
        email: None,
        exp: (chrono::Utc::now() - chrono::Duration::hours(1)).timestamp() as usize,
        aud: "test-client".to_string(),
        iss: mock_server.uri().to_string(),
    };
    let key = EncodingKey::from_rsa_pem(get_test_keys().0.as_bytes()).unwrap();
    let token = encode(&header, &claims, &key).unwrap();

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token": token,
            "refresh_token": "dummy"
        })))
        .mount(&mock_server)
        .await;

    let app = setup_app(mock_server.uri()).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/exchange")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "sso_token": "dummy-auth-code",
                        "callback_url": "http://localhost:3000/callback"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_refresh_network_error() {
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .unwrap();
    let app = stormchaser_api::app(AppState {
        pool,
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: Some(OidcConfig {
            issuer: "http://localhost:1".to_string(), // Invalid
            external_issuer: "http://localhost:1".to_string(),
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
            jwks_url: "http://localhost:1/.well-known/jwks.json".to_string(),
        }),
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
    });

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "refresh_token": "old-dummy-refresh-token"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_refresh_invalid_json() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&mock_server)
        .await;
    let app = setup_app(mock_server.uri()).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "refresh_token": "old-dummy-refresh-token"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_refresh_invalid_id_token_header() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token": "not.a.jwt",
            "refresh_token": "new-refresh-token"
        })))
        .mount(&mock_server)
        .await;
    let app = setup_app(mock_server.uri()).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "refresh_token": "old-dummy-refresh-token"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_refresh_no_kid() {
    let mock_server = MockServer::start().await;
    let mut header = Header::new(Algorithm::RS256);
    header.kid = None;
    let claims = IdTokenClaims {
        sub: "test".to_string(),
        email: None,
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        aud: "test-client".to_string(),
        iss: mock_server.uri().to_string(),
    };
    let key = EncodingKey::from_rsa_pem(get_test_keys().0.as_bytes()).unwrap();
    let token = encode(&header, &claims, &key).unwrap();

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token": token,
            "refresh_token": "dummy"
        })))
        .mount(&mock_server)
        .await;

    let app = setup_app(mock_server.uri()).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "refresh_token": "old-dummy-refresh-token"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_refresh_kid_not_found() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": []
        })))
        .mount(&mock_server)
        .await;

    let token = create_id_token(&mock_server.uri(), "test-client");
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token": token,
            "refresh_token": "dummy"
        })))
        .mount(&mock_server)
        .await;

    let app = setup_app(mock_server.uri()).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "refresh_token": "old-dummy-refresh-token"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_refresh_invalid_jwk() {
    let mock_server = MockServer::start().await;
    let mut invalid_jwk = get_test_keys().1.clone();
    invalid_jwk["n"] = json!("invalid-base64");

    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [invalid_jwk]
        })))
        .mount(&mock_server)
        .await;

    let token = create_id_token(&mock_server.uri(), "test-client");
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token": token,
            "refresh_token": "dummy"
        })))
        .mount(&mock_server)
        .await;

    let app = setup_app(mock_server.uri()).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "refresh_token": "old-dummy-refresh-token"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_refresh_invalid_signature() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [get_test_keys().1.clone()]
        })))
        .mount(&mock_server)
        .await;

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-kid".to_string());
    let claims = IdTokenClaims {
        sub: "test".to_string(),
        email: None,
        exp: (chrono::Utc::now() - chrono::Duration::hours(1)).timestamp() as usize, // Expired
        aud: "test-client".to_string(),
        iss: mock_server.uri().to_string(),
    };
    let key = EncodingKey::from_rsa_pem(get_test_keys().0.as_bytes()).unwrap();
    let token = encode(&header, &claims, &key).unwrap();

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token": token,
            "refresh_token": "dummy"
        })))
        .mount(&mock_server)
        .await;

    let app = setup_app(mock_server.uri()).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    json!({
                        "refresh_token": "old-dummy-refresh-token"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
