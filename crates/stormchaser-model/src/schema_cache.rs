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
    pub fn start_background_sync(&self, _oci_registry_url: String) {
        let _schemas = self.schemas.clone();
        tokio::spawn(async move {
            // Implementation for syncing from `oci_registry_url` goes here.
            // For now, this is a stub that represents the background sync loop.
            loop {
                // TODO: Implement OCI fetch
                // let fetched_schemas = fetch_schemas_from_oci(&oci_registry_url).await;
                // for (id, schema) in fetched_schemas {
                //     schemas.write().await.insert(id, schema);
                // }
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        });
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
}
