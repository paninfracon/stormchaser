use crate::utils::{handle_response, require_token};
use anyhow::Result;

pub async fn stream_logs(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: stormchaser_model::RunId,
    step_name: String,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!(
            "{}/api/v1/runs/{}/steps/{}/logs/stream",
            url,
            id,
            urlencoding::encode(&step_name)
        ))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
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
                println!("{}", event.data);
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
    async fn test_stream_logs_success() {
        let server = MockServer::start().await;
        let id = stormchaser_model::RunId::new_v4();
        let step_name = "test_step";

        let sse_body = "event: message\ndata: Hello from logs\n\n\
                        event: error\ndata: stream ended\n\n";

        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/runs/{}/steps/{}/logs/stream",
                id, step_name
            )))
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

        let result = stream_logs(
            &server.uri(),
            Some("fake_token"),
            &client,
            id,
            step_name.to_string(),
        )
        .await;

        result.unwrap();
    }
}
