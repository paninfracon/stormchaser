//! The interactive API documentation, served only with `STORMCHASER_API_DOCS=true`.
//!
//! The OpenAPI document is served from here. The page that shows it loads a pinned
//! Swagger UI release from cdnjs, checked against the release's published hashes
//! (subresource integrity), and starts it with a script also served from here, so the
//! page's content security policy allows no inline code.

use axum::{
    http::header,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use utoipa::OpenApi;

/// The Swagger UI release the page loads, and the hashes its files must have.
const SWAGGER_UI_BASE: &str = "https://cdnjs.cloudflare.com/ajax/libs/swagger-ui/5.29.1";
const SWAGGER_UI_CSS_SRI: &str =
    "sha512-QIJpSy6rqOKoEjIR+Pp7r5lTkNPJPJCRejWzG4jb12bCT1EeL/vcjZtk4NNKeEuVRXY+d34d/t9y1CHzZbgeJQ==";
const SWAGGER_UI_JS_SRI: &str =
    "sha512-yfAKqi7F3JSDZA7V/XTql3caj2o8A81yg2EWIBykt2BJjLjNYnsd4CVAKvrFp/nYl9HQX7R06Z6e5KBn5aoj6g==";

/// Where the OpenAPI document is served.
pub const OPENAPI_PATH: &str = "/api-docs/openapi.json";

/// The documentation's routes: the OpenAPI document, the page and its start-up script.
pub fn routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    let document = crate::ApiDoc::openapi();
    Router::new()
        .route(
            OPENAPI_PATH,
            get(move || {
                let document = document.clone();
                async move { Json(document) }
            }),
        )
        .route("/swagger-ui", get(page))
        .route("/swagger-ui/", get(page))
        .route("/swagger-ui/init.js", get(init_script))
}

/// The policy for the page: Swagger UI from cdnjs, its start-up script and the document
/// from here, and nothing else.
fn content_security_policy() -> String {
    format!(
        "default-src 'none'; script-src 'self' {base}/; style-src {base}/; \
         img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'",
        base = SWAGGER_UI_BASE
    )
}

async fn page() -> Response {
    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Stormchaser API</title>
<link rel="stylesheet" href="{base}/swagger-ui.min.css" integrity="{css_sri}" crossorigin="anonymous" referrerpolicy="no-referrer">
</head>
<body>
<div id="swagger-ui"></div>
<script src="{base}/swagger-ui-bundle.min.js" integrity="{js_sri}" crossorigin="anonymous" referrerpolicy="no-referrer"></script>
<script src="/swagger-ui/init.js"></script>
</body>
</html>
"#,
        base = SWAGGER_UI_BASE,
        css_sri = SWAGGER_UI_CSS_SRI,
        js_sri = SWAGGER_UI_JS_SRI,
    );
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
            (header::CONTENT_SECURITY_POLICY, content_security_policy()),
            (header::REFERRER_POLICY, "no-referrer".to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        ],
        html,
    )
        .into_response()
}

async fn init_script() -> Response {
    let script = format!(
        "window.ui = SwaggerUIBundle({{ url: \"{OPENAPI_PATH}\", dom_id: \"#swagger-ui\" }});\n"
    );
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        script,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn get(path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
        let response = routes::<()>()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, headers, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn the_openapi_document_is_served() {
        let (status, _, body) = get(OPENAPI_PATH).await;
        assert_eq!(status, StatusCode::OK);
        let document: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(document["openapi"].as_str().unwrap().starts_with('3'));
        assert!(document["paths"].as_object().is_some_and(|p| !p.is_empty()));
    }

    #[tokio::test]
    async fn the_page_loads_pinned_swagger_ui_and_no_inline_code() {
        for path in ["/swagger-ui", "/swagger-ui/"] {
            let (status, headers, body) = get(path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert!(body.contains(SWAGGER_UI_JS_SRI) && body.contains(SWAGGER_UI_CSS_SRI));
            // Every script is loaded from a file; none is written into the page
            assert_eq!(
                body.matches("<script").count(),
                body.matches("<script src=").count()
            );

            let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
            assert!(csp.contains("default-src 'none'"), "{csp}");
            assert!(!csp.contains("unsafe-inline"), "{csp}");
        }
        let (status, headers, body) = get("/swagger-ui/init.js").await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/javascript"));
        assert!(body.contains(OPENAPI_PATH));
    }
}
