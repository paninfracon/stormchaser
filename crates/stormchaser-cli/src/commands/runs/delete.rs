use crate::utils::{handle_response, require_token};
use anyhow::Result;
use reqwest::header::AUTHORIZATION;

pub async fn delete_run(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: stormchaser_model::RunId,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .delete(format!("{}/api/v1/runs/{}", url, id))
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_runs_delete() {
        let server = MockServer::start().await;
        let id = stormchaser_model::RunId::new_v4();
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/runs/{}", id)))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();

        let result = delete_run(&server.uri(), Some("test-token"), &client, id).await;
        result.unwrap();
    }
}
