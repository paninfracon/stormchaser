use anyhow::Result;
use async_nats::jetstream::{self, stream::Config};
use stormchaser_model::nats::{stream_max_age, STREAM_NAME};

/// Initializes the NATS JetStream context and creates required streams if they do not exist.
pub async fn init_jetstream(nats_client: &async_nats::Client) -> Result<jetstream::Context> {
    let js = jetstream::new(nats_client.clone());
    let max_age = stream_max_age();

    // Ensure the stormchaser stream exists
    let mut stream = js
        .get_or_create_stream(Config {
            name: STREAM_NAME.to_string(),
            subjects: vec!["stormchaser.v1.>".to_string()],
            max_age,
            ..Default::default()
        })
        .await?;

    // A stream made by an older version, or by a runner that started first, keeps
    // messages forever. Give it the limit; one an operator already set is left alone.
    let mut config = stream.info().await?.config.clone();
    if config.max_age.is_zero() && !max_age.is_zero() {
        config.max_age = max_age;
        match js.update_stream(&config).await {
            Ok(_) => tracing::info!(
                "Set a maximum message age of {} hours on the '{}' stream",
                max_age.as_secs() / 3600,
                STREAM_NAME
            ),
            Err(e) => tracing::warn!(
                "Could not set a maximum message age on the '{}' stream; task messages are kept until erased: {:?}",
                STREAM_NAME,
                e
            ),
        }
    }

    Ok(js)
}
