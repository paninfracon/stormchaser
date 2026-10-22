use anyhow::Result;
use stormchaser_client::Client;

pub async fn delete_run(client: &Client, id: stormchaser_model::RunId) -> Result<()> {
    client.delete_run(id).await?;
    println!("Success");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_runs_delete() {
        let server = MockServer::start().await;
        let id = stormchaser_model::RunId::new_v4();
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/runs/{}", id)))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();

        let result = delete_run(&client, id).await;
        result.unwrap();
    }
}
