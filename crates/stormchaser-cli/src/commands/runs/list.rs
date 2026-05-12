use crate::utils::{handle_response, require_token};
use anyhow::Result;
use reqwest::header::AUTHORIZATION;

pub struct ListRunsFilters {
    pub owner: Option<String>,
    pub name: Option<String>,
    pub repo_url: Option<String>,
    pub workflow_path: Option<String>,
    pub created_after: Option<String>,
    pub created_before: Option<String>,
    pub status: Option<String>,
}

pub async fn list_runs(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    filters: ListRunsFilters,
) -> Result<()> {
    let token = require_token(token)?;
    let url = build_list_runs_url(url, filters)?;

    let res = http_client
        .get(url)
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

fn build_list_runs_url(base_url: &str, filters: ListRunsFilters) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(&format!("{}/api/v1/runs", base_url))?;
    if let Some(o) = filters.owner {
        url.query_pairs_mut().append_pair("initiating_user", &o);
    }
    if let Some(n) = filters.name {
        url.query_pairs_mut().append_pair("workflow_name", &n);
    }
    if let Some(r) = filters.repo_url {
        url.query_pairs_mut().append_pair("repo_url", &r);
    }
    if let Some(w) = filters.workflow_path {
        url.query_pairs_mut().append_pair("workflow_path", &w);
    }
    if let Some(ca) = filters.created_after {
        url.query_pairs_mut().append_pair("created_after", &ca);
    }
    if let Some(cb) = filters.created_before {
        url.query_pairs_mut().append_pair("created_before", &cb);
    }
    if let Some(s) = filters.status {
        url.query_pairs_mut().append_pair("status", &s);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use serde_json::json;
    use std::collections::HashMap;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_build_list_runs_url_basic() {
        let url = build_list_runs_url(
            "http://localhost:8080",
            ListRunsFilters {
                owner: None,
                name: None,
                repo_url: None,
                workflow_path: None,
                created_after: None,
                created_before: None,
                status: None,
            },
        )
        .unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/api/v1/runs");
    }

    #[test]
    fn test_build_list_runs_url_with_params() {
        let url = build_list_runs_url(
            "http://localhost:8080",
            ListRunsFilters {
                owner: Some("alice".to_string()),
                name: Some("my-workflow".to_string()),
                repo_url: None,
                workflow_path: None,
                created_after: None,
                created_before: None,
                status: Some("Succeeded".to_string()),
            },
        )
        .unwrap();
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("initiating_user").unwrap(), "alice");
        assert_eq!(query.get("workflow_name").unwrap(), "my-workflow");
        assert_eq!(query.get("status").unwrap(), "Succeeded");
        assert!(!query.contains_key("repo_url"));
    }

    #[tokio::test]
    async fn test_runs_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/runs"))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let filters = ListRunsFilters {
            owner: None,
            name: None,
            repo_url: None,
            workflow_path: None,
            created_after: None,
            created_before: None,
            status: None,
        };

        let result = list_runs(&server.uri(), Some("test-token"), &client, filters).await;
        assert!(result.is_ok());
    }
}
