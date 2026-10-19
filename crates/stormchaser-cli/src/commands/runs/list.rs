use crate::utils::print_json;
use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;
use stormchaser_client::Client;
use stormchaser_model::api::ListRunsQuery;

pub struct ListRunsFilters {
    pub owner: Option<String>,
    pub name: Option<String>,
    pub repo_url: Option<String>,
    pub workflow_path: Option<String>,
    pub created_after: Option<String>,
    pub created_before: Option<String>,
    pub status: Option<String>,
}

pub async fn list_runs(client: &Client, filters: ListRunsFilters) -> Result<()> {
    let query = build_list_runs_query(filters)?;
    print_json(&client.list_runs(&query).await?)
}

/// Reads a flag's text as the API's type for it, as the API would read it in a query.
fn parse_flag<T: DeserializeOwned>(flag: &str, text: Option<String>) -> Result<Option<T>> {
    text.map(|text| {
        serde_json::from_value(Value::String(text.clone()))
            .with_context(|| format!("--{} cannot be '{}'", flag, text))
    })
    .transpose()
}

/// A status as the API spells it (`succeeded`), accepting `Succeeded` too.
fn snake_case(text: String) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for (i, c) in text.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn build_list_runs_query(filters: ListRunsFilters) -> Result<ListRunsQuery> {
    Ok(ListRunsQuery {
        workflow_name: filters.name,
        status: parse_flag("status", filters.status.map(snake_case))?,
        initiating_user: filters.owner,
        repo_url: filters.repo_url,
        workflow_path: filters.workflow_path,
        created_after: parse_flag("created-after", filters.created_after)?,
        created_before: parse_flag("created-before", filters.created_before)?,
        limit: None,
        offset: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use stormchaser_model::RunStatus;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn no_filters() -> ListRunsFilters {
        ListRunsFilters {
            owner: None,
            name: None,
            repo_url: None,
            workflow_path: None,
            created_after: None,
            created_before: None,
            status: None,
        }
    }

    #[test]
    fn test_build_list_runs_query_basic() {
        let query = build_list_runs_query(no_filters()).unwrap();
        assert_eq!(serde_json::to_value(&query).unwrap()["status"], json!(null));
    }

    #[test]
    fn test_build_list_runs_query_with_params() {
        let query = build_list_runs_query(ListRunsFilters {
            owner: Some("alice".to_string()),
            name: Some("my-workflow".to_string()),
            created_after: Some("2026-01-02T03:04:05Z".to_string()),
            status: Some("Succeeded".to_string()),
            ..no_filters()
        })
        .unwrap();
        assert_eq!(query.initiating_user.as_deref(), Some("alice"));
        assert_eq!(query.workflow_name.as_deref(), Some("my-workflow"));
        assert_eq!(query.status, Some(RunStatus::Succeeded));
        assert!(query.created_after.is_some());
        assert!(query.repo_url.is_none());

        for status in ["start_pending", "StartPending"] {
            let query = build_list_runs_query(ListRunsFilters {
                status: Some(status.to_string()),
                ..no_filters()
            })
            .unwrap();
            assert_eq!(query.status, Some(RunStatus::StartPending));
        }
    }

    #[test]
    fn a_filter_the_api_cannot_read_is_refused() {
        let err = build_list_runs_query(ListRunsFilters {
            status: Some("sideways".to_string()),
            ..no_filters()
        })
        .unwrap_err();
        assert!(err.to_string().contains("--status"));
        build_list_runs_query(ListRunsFilters {
            created_before: Some("yesterday".to_string()),
            ..no_filters()
        })
        .unwrap_err();
    }

    #[tokio::test]
    async fn test_runs_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/runs"))
            .and(query_param("workflow_name", "wf"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(1)
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let filters = ListRunsFilters {
            name: Some("wf".to_string()),
            ..no_filters()
        };

        let result = list_runs(&client, filters).await;
        result.unwrap();
    }
}
