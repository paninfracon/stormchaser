use crate::utils::{handle_response, require_token};
use anyhow::Result;

pub async fn list_reports(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: stormchaser_model::RunId,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/reports", url, id))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

pub async fn get_report(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: stormchaser_model::RunId,
    report_id: stormchaser_model::TestReportId,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/reports/{}", url, id, report_id))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_retry::policies::ExponentialBackoff;
    use reqwest_retry::RetryTransientMiddleware;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn build_client() -> reqwest_middleware::ClientWithMiddleware {
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(0);
        reqwest_middleware::ClientBuilder::new(reqwest::Client::new())
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build()
    }

    #[tokio::test]
    async fn test_list_reports() {
        let mock_server = MockServer::start().await;
        let id = stormchaser_model::RunId::new_v4();

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}/reports", id)))
            .and(header(reqwest::header::AUTHORIZATION, "Bearer test_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&mock_server)
            .await;

        let client = build_client();
        let res = list_reports(&mock_server.uri(), Some("test_token"), &client, id).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_list_reports_no_token() {
        let mock_server = MockServer::start().await;
        let id = stormchaser_model::RunId::new_v4();
        let client = build_client();
        let res = list_reports(&mock_server.uri(), None, &client, id).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_get_report() {
        let mock_server = MockServer::start().await;
        let id = stormchaser_model::RunId::new_v4();
        let report_id = stormchaser_model::TestReportId::new_v4();

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}/reports/{}", id, report_id)))
            .and(header(reqwest::header::AUTHORIZATION, "Bearer test_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&mock_server)
            .await;

        let client = build_client();
        let res = get_report(
            &mock_server.uri(),
            Some("test_token"),
            &client,
            id,
            report_id,
        )
        .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_get_report_no_token() {
        let mock_server = MockServer::start().await;
        let id = stormchaser_model::RunId::new_v4();
        let report_id = stormchaser_model::TestReportId::new_v4();
        let client = build_client();
        let res = get_report(&mock_server.uri(), None, &client, id, report_id).await;
        assert!(res.is_err());
    }
}
