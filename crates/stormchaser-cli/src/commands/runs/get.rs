use crate::utils::print_json;
use anyhow::Result;
use stormchaser_client::Client;

pub async fn get_run(client: &Client, id: stormchaser_model::RunId) -> Result<()> {
    print_json(&client.get_run(id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_runs_get() {
        let server = MockServer::start().await;
        let id = stormchaser_model::RunId::new_v4();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}", id)))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(super::super::logs::tests::run_json(id, &[])),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();

        let result = get_run(&client, id).await;
        result.unwrap();
    }
}
