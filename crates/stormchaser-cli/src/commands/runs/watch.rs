use crate::utils::stream_run_status;
use anyhow::Result;
use stormchaser_client::Client;

pub async fn watch_run(client: &Client, id: stormchaser_model::RunId) -> Result<()> {
    stream_run_status(client, id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_watch_run_success() {
        let server = MockServer::start().await;
        let id = stormchaser_model::RunId::new_v4();

        let sse_body = "event: status\ndata: running\n\n\
                        event: error\ndata: stream ended\n\n";

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}/status/stream", id)))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("fake_token")
            .build()
            .unwrap();

        let result = watch_run(&client, id).await;

        result.unwrap();
    }
}
