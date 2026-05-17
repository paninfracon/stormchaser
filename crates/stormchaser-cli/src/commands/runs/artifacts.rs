use crate::utils::{handle_response, require_token};
use anyhow::Result;
use reqwest::header::AUTHORIZATION;

pub async fn list_artifacts(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: stormchaser_model::RunId,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/artifacts", url, id))
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_runs_artifacts() {
        let server = MockServer::start().await;
        let id = stormchaser_model::RunId::new_v4();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}/artifacts", id)))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();

        let result = list_artifacts(&server.uri(), Some("test-token"), &client, id).await;
        result.unwrap();
    }
}
