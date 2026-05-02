use crate::utils::{handle_response, require_token};
use anyhow::Result;
use uuid::Uuid;

pub async fn watch_run(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: Uuid,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/status/stream", url, id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;

    if !res.status().is_success() {
        return handle_response(res).await;
    }

    use eventsource_stream::Eventsource;
    use futures::stream::StreamExt;
    let mut stream = res.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        match event {
            Ok(event) => {
                if event.event == "error" {
                    eprintln!("Error from stream: {}", event.data);
                    break;
                }
                println!("{}: {}", event.event, event.data);
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_watch_run_success() {
        let server = MockServer::start().await;
        let id = Uuid::new_v4();

        let sse_body = "event: status\ndata: running\n\n\
                        event: error\ndata: stream ended\n\n";

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}/status/stream", id)))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .mount(&server)
            .await;

        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(0);
        let client = ClientBuilder::new(reqwest::Client::new())
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        let result = watch_run(&server.uri(), Some("fake_token"), &client, id).await;

        assert!(result.is_ok());
    }
}
