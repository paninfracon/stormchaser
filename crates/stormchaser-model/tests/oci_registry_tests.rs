use oci_distribution::client::{Client, ClientConfig, ClientProtocol, Config, ImageLayer};
use oci_distribution::secrets::RegistryAuth;
use oci_distribution::Reference;
use stormchaser_model::schema_cache::SchemaCache;
use testcontainers::{core::IntoContainerPort, core::WaitFor, runners::AsyncRunner, GenericImage};

#[tokio::test]
async fn test_oci_fetch_integration() {
    // Start a registry:2 container using testcontainers
    let image = GenericImage::new("registry", "2")
        .with_exposed_port(5000.tcp())
        .with_wait_for(WaitFor::message_on_stderr("listening on [::]:5000"));

    let container = image
        .start()
        .await
        .expect("Failed to start registry container");
    let port = container
        .get_host_port_ipv4(5000)
        .await
        .expect("Failed to get port");

    // Give it an extra moment to be fully ready to accept connections
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    // Build the registry URL
    let reference_str = format!("127.0.0.1:{}/integration-test-schema:latest", port);
    let url = format!("http://{}", reference_str);

    let reference: Reference = reference_str.parse().unwrap();
    let client_config = ClientConfig {
        protocol: ClientProtocol::Http,
        ..Default::default()
    };
    let client = Client::new(client_config);
    let auth = RegistryAuth::Anonymous;

    let schema_json = r#"{"$id": "real_integration_schema", "type": "object"}"#;
    let layer_data = schema_json.as_bytes().to_vec();
    let layer = ImageLayer::new(
        layer_data,
        "application/vnd.oci.image.layer.v1.tar+gzip".to_string(),
        None,
    );

    let config = Config {
        data: b"{}".to_vec(),
        media_type: "application/vnd.oci.image.config.v1+json".to_string(),
        annotations: None,
    };

    // Push the schema layer directly
    client
        .push(&reference, &[layer], config, &auth, None)
        .await
        .expect("Failed to push artifact");

    let cache = SchemaCache::default();
    cache.start_background_sync(url.clone());

    // Wait for sync to happen
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let schema = cache.get("real_integration_schema").await;
    assert!(schema.is_some(), "Schema was not fetched correctly");
}
