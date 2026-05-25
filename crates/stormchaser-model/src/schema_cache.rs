#![cfg(not(target_arch = "wasm32"))]
use oci_distribution::client::{Client, ClientConfig, ClientProtocol};
use oci_distribution::secrets::RegistryAuth;
use oci_distribution::Reference;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory cache for OCI-backed CloudEvent schemas.
#[derive(Clone)]
pub struct SchemaCache {
    schemas: Arc<RwLock<HashMap<String, Value>>>,
}

impl Default for SchemaCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SchemaCache {
    /// Create a new empty schema cache.
    pub fn new() -> Self {
        Self {
            schemas: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get a schema by its ID or Version.
    pub async fn get(&self, schema_id: &str) -> Option<Value> {
        let cache = self.schemas.read().await;
        cache.get(schema_id).cloned()
    }

    /// Insert or update a schema in the cache.
    pub async fn insert(&self, schema_id: String, schema: Value) {
        let mut cache = self.schemas.write().await;
        cache.insert(schema_id, schema);
    }

    /// Start a background sync process to fetch schemas from an OCI registry.
    /// This is a permissive sync; it runs asynchronously and updates the cache.
    pub fn start_background_sync(&self, oci_registry_url: String) {
        let schemas = self.schemas.clone();
        tokio::spawn(async move {
            loop {
                let fetched_schemas = Self::fetch_schemas_from_oci(&oci_registry_url).await;
                for (id, schema) in fetched_schemas {
                    schemas.write().await.insert(id, schema);
                }
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        });
    }

    /// Fetches schemas from the given OCI registry URL.
    /// We attempt to pull image layers and parse them as JSON schemas.
    async fn fetch_schemas_from_oci(url: &str) -> Vec<(String, Value)> {
        let protocol = if url.starts_with("http://") {
            ClientProtocol::Http
        } else {
            ClientProtocol::Https
        };
        let reference_str = url
            .trim_start_matches("http://")
            .trim_start_matches("https://");

        let reference: Reference = match reference_str.parse() {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(
                    "Failed to parse OCI registry URL '{}': {}",
                    reference_str,
                    e
                );
                return vec![];
            }
        };

        let config = ClientConfig {
            protocol,
            ..Default::default()
        };
        let client = Client::new(config);
        let auth = RegistryAuth::Anonymous;

        let image_data = match client
            .pull(
                &reference,
                &auth,
                vec![
                    "application/vnd.oci.image.layer.v1.tar+gzip",
                    "application/vnd.oci.image.layer.v1.tar",
                ],
            )
            .await
        {
            Ok(data) => data,
            Err(e) => {
                tracing::error!("Failed to pull OCI artifact from '{}': {}", url, e);
                return vec![];
            }
        };

        let mut results = Vec::new();
        for layer in image_data.layers {
            // Read layer data
            let bytes = layer.data;
            let media_type = layer.media_type;

            // For now, assume it's just raw JSON or try to parse directly.
            // If the layer is a tarball, we might need to extract it.
            // A simple approach is to see if we can parse it as JSON first.
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                if let Some(id) = value.get("$id").and_then(|v| v.as_str()) {
                    results.push((id.to_string(), value));
                    continue;
                }
            }

            // Extract JSON files from tar or tar+gzip layer
            use std::io::Read;
            use tar::Archive;

            // Detect if the layer is gzip-compressed. Other compression formats (e.g., zstd)
            // and plain uncompressed tar both fall through to the uncompressed tar path.
            let is_gzip = media_type == "application/vnd.oci.image.layer.v1.tar+gzip";
            let reader: Box<dyn Read> = if is_gzip {
                use flate2::read::GzDecoder;
                Box::new(GzDecoder::new(std::io::Cursor::new(bytes)))
            } else {
                Box::new(std::io::Cursor::new(bytes))
            };
            let mut archive = Archive::new(reader);

            if let Ok(entries) = archive.entries() {
                for file in entries.flatten() {
                    let path = file
                        .path()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if path.ends_with(".json") {
                        let mut content = Vec::new();
                        let mut file = file; // take ownership of mut entry
                        if file.read_to_end(&mut content).is_ok() {
                            if let Ok(value) = serde_json::from_slice::<Value>(&content) {
                                if let Some(id) = value.get("$id").and_then(|v| v.as_str()) {
                                    results.push((id.to_string(), value));
                                }
                            }
                        }
                    }
                }
            }
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_schema_cache_insert_and_get() {
        let cache = SchemaCache::default();
        let schema_id = "test_schema_id".to_string();
        let schema_val = serde_json::json!({"type": "object"});

        // Should be empty initially
        assert_eq!(cache.get(&schema_id).await, None);

        // Insert
        cache.insert(schema_id.clone(), schema_val.clone()).await;

        // Should return the inserted value
        assert_eq!(cache.get(&schema_id).await, Some(schema_val));
    }

    #[tokio::test]
    async fn test_fetch_schemas_from_oci_mock() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        // Mock /v2/ auth check
        Mock::given(method("GET"))
            .and(path("/v2/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        // Mock manifest
        let manifest = serde_json::json!({
           "schemaVersion": 2,
           "mediaType": "application/vnd.oci.image.manifest.v1+json",
           "config": {
              "mediaType": "application/vnd.oci.image.config.v1+json",
              "digest": "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
              "size": 2
           },
           "layers": [
              {
                 "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                 "digest": "sha256:416c9c6f24b11975d1f224ea33076dc692289fda308f8ed61ca49f097186e6a1",
                 "size": 40
              }
           ]
        });
        Mock::given(method("GET"))
            .and(path("/v2/mock-repo/manifests/latest"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&manifest)
                    .insert_header("Docker-Content-Digest", "sha256:some-digest")
                    .insert_header("Content-Type", "application/vnd.oci.image.manifest.v1+json"),
            )
            .mount(&mock_server)
            .await;

        // Mock config blob
        let config_json = "{}";
        Mock::given(method("GET"))
            .and(path("/v2/mock-repo/blobs/sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"))
            .respond_with(ResponseTemplate::new(200).set_body_string(config_json))
            .mount(&mock_server)
            .await;

        // Mock blob
        let schema_json = r#"{"$id": "mock_schema", "type": "object"}"#;
        Mock::given(method("GET"))
            .and(path("/v2/mock-repo/blobs/sha256:416c9c6f24b11975d1f224ea33076dc692289fda308f8ed61ca49f097186e6a1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(schema_json))
            .mount(&mock_server)
            .await;

        let url = format!("{}/mock-repo:latest", mock_server.uri());
        let results = SchemaCache::fetch_schemas_from_oci(&url).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "mock_schema");
        assert_eq!(
            results[0].1.get("type").unwrap().as_str().unwrap(),
            "object"
        );
    }
}
