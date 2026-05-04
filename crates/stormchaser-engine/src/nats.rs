use anyhow::Result;
use async_nats::jetstream::{self, stream::Config};

/// Initializes the NATS JetStream context and creates required streams if they do not exist.
pub async fn init_jetstream(nats_client: &async_nats::Client) -> Result<jetstream::Context> {
    let js = jetstream::new(nats_client.clone());

    // Ensure the stormchaser stream exists
    js.get_or_create_stream(Config {
        name: "stormchaser".to_string(),
        subjects: vec!["stormchaser.v1.>".to_string()],
        ..Default::default()
    })
    .await?;

    Ok(js)
}
