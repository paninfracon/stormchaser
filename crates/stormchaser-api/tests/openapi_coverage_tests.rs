//! The OpenAPI document describes every route the API serves, with a body schema
//! wherever a route takes or returns one, so that a client can be written from it.

use std::collections::BTreeSet;
use utoipa::OpenApi;

/// Routes left out of the document on purpose: the MCP transport, health checks and
/// the landing page are not part of the REST API.
const UNDOCUMENTED: &[&str] = &["/api/v1/mcp", "/", "/healthz", "/api/health"];

/// A path with its parameter names left out, so `{id}` and `{run_id}` compare equal.
fn shape(path: &str) -> String {
    let mut out = String::new();
    let mut in_param = false;
    for c in path.chars() {
        match c {
            '{' => {
                in_param = true;
                out.push_str("{}");
            }
            '}' => in_param = false,
            _ if in_param => {}
            _ => out.push(c),
        }
    }
    out
}

/// (method, path) for every `.route(...)` in the API's router, read from its source.
fn routed() -> BTreeSet<(String, String)> {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("the router's source");
    let mut routes = BTreeSet::new();
    let mut rest = source.as_str();
    while let Some(i) = rest.find(".route(") {
        rest = &rest[i + ".route(".len()..];
        // The call's text, up to its closing parenthesis
        let mut depth = 1;
        let end = rest
            .char_indices()
            .find(|&(_, c)| {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                depth == 0
            })
            .map(|(i, _)| i)
            .unwrap();
        let call = &rest[..end];
        let Some(path) = call.split('"').nth(1) else {
            continue;
        };
        let path = if path.starts_with("/api/") || ["/", "/healthz"].contains(&path) {
            path.to_string()
        } else {
            format!("/api/v1{path}")
        };
        for method in ["get", "post", "put", "patch", "delete"] {
            let calls_it = call.contains(&format!(" {method}(", method = method))
                || call.contains(&format!("\n{method}(", method = method))
                || call.contains(&format!(".{method}(", method = method))
                || call.contains(&format!("({method}(", method = method));
            if calls_it {
                routes.insert((method.to_string(), shape(&path)));
            }
        }
    }
    routes
}

fn documented() -> serde_json::Value {
    serde_json::to_value(stormchaser_api::ApiDoc::openapi()).unwrap()
}

#[test]
fn every_route_is_in_the_openapi_document() {
    let doc = documented();
    let paths = doc["paths"].as_object().unwrap();
    let documented: BTreeSet<(String, String)> = paths
        .iter()
        .flat_map(|(path, item)| {
            item.as_object()
                .unwrap()
                .keys()
                .map(move |method| (method.clone(), shape(path)))
        })
        .collect();

    let routes = routed();
    assert!(routes.len() > 30, "{routes:?}");
    let missing: Vec<_> = routes
        .iter()
        .filter(|(_, path)| !UNDOCUMENTED.contains(&path.as_str()))
        .filter(|route| !documented.contains(*route))
        .collect();
    assert!(
        missing.is_empty(),
        "not in the OpenAPI document: {missing:#?}"
    );

    let stale: Vec<_> = documented.difference(&routes).collect();
    assert!(stale.is_empty(), "documented but not routed: {stale:#?}");
}

#[test]
fn operations_that_take_or_return_json_say_what() {
    let doc = documented();
    let mut thin = Vec::new();
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            // A create or update sends a body
            if ["post", "put", "patch"].contains(&method.as_str())
                && op.get("requestBody").is_none()
                && !op["x-no-request-body"].as_bool().unwrap_or(false)
            {
                let takes_none = path.contains("/approve")
                    || path.contains("/reject")
                    || path.contains("/approve-link/")
                    || path.contains("/cron-trigger/")
                    || path.ends_with("/auth/login")
                    || path.contains("/webhooks/{id}") && method == "post";
                if !takes_none {
                    thin.push(format!("{method} {path}: no request body"));
                }
            }
            // A successful GET of JSON says what it returns
            if method == "get" {
                let ok = &op["responses"]["200"];
                let streams = path.ends_with("/stream");
                let html = path.contains("/approve-link/") || path.ends_with("/auth/login");
                if !streams && !html && ok.get("content").is_none() {
                    thin.push(format!("{method} {path}: no response body"));
                }
            }
        }
    }
    assert!(thin.is_empty(), "{thin:#?}");
}

#[test]
fn bearer_auth_is_defined() {
    let doc = documented();
    assert!(
        doc["components"]["securitySchemes"]["bearer_auth"].is_object(),
        "the bearer_auth scheme operations refer to is defined"
    );
}
