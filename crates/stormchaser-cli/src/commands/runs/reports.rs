use crate::utils::print_json;
use anyhow::Result;
use stormchaser_client::Client;

pub async fn list_reports(client: &Client, id: stormchaser_model::RunId) -> Result<()> {
    print_json(&client.run_test_reports(id).await?)
}

pub async fn get_report(
    client: &Client,
    id: stormchaser_model::RunId,
    report_id: stormchaser_model::TestReportId,
) -> Result<()> {
    print_json(&client.test_report(id, report_id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stormchaser_model::RunId;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn build_client(server: &MockServer) -> Client {
        Client::builder(server.uri())
            .token("test_token")
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn test_list_reports() {
        let mock_server = MockServer::start().await;
        let id = RunId::new_v4();

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}/reports", id)))
            .and(header("authorization", "Bearer test_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .expect(1)
            .mount(&mock_server)
            .await;

        let res = list_reports(&build_client(&mock_server), id).await;
        res.unwrap();
    }

    #[tokio::test]
    async fn test_get_report() {
        let mock_server = MockServer::start().await;
        let id = RunId::new_v4();
        let report_id = stormchaser_model::TestReportId::new_v4();

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}/reports/{}", id, report_id)))
            .and(header("authorization", "Bearer test_token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "content": "<testsuites/>" })),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let res = get_report(&build_client(&mock_server), id, report_id).await;
        res.unwrap();
    }

    #[tokio::test]
    async fn a_server_error_is_returned_with_its_status() {
        let mock_server = MockServer::start().await;
        let id = RunId::new_v4();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}/reports", id)))
            .respond_with(ResponseTemplate::new(404).set_body_string("Run not found"))
            .mount(&mock_server)
            .await;

        let err = list_reports(&build_client(&mock_server), id)
            .await
            .unwrap_err();
        match err.downcast_ref::<stormchaser_client::Error>() {
            Some(stormchaser_client::Error::Status { status, body }) => {
                assert_eq!(status.as_u16(), 404);
                assert_eq!(body, "Run not found");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
