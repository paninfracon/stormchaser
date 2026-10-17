use anyhow::Result;
use stormchaser_dsl::ast::Workflow;

/// True for the hex SHA-256 the agent reports for a parked volume.
fn is_storage_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Records the hash of each storage volume a step parked.
///
/// The names and hashes come from the step's own container, so only volumes the workflow
/// declares are recorded, and only values shaped like a hash.
pub async fn persist_storage_hashes(
    tx: &mut sqlx::PgConnection,
    run_id: uuid::Uuid,
    storage_hashes: Option<&serde_json::Map<String, serde_json::Value>>,
    workflow: &Workflow,
) -> Result<()> {
    let Some(hashes) = storage_hashes else {
        return Ok(());
    };
    for (name, hash_val) in hashes {
        let declared = workflow.storage.iter().any(|s| s.name == *name);
        match hash_val.as_str() {
            Some(hash) if declared && is_storage_hash(hash) => {
                crate::db::upsert_run_storage_state(&mut *tx, run_id, name, hash).await?;
            }
            _ => tracing::warn!(
                "Ignoring a storage hash reported for run {}: not a declared volume or not a hash",
                run_id
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_storage_hash;

    #[test]
    fn recognises_a_sha256_hex_digest() {
        assert!(is_storage_hash(&"a1".repeat(32)));
        assert!(!is_storage_hash("abc123"));
        assert!(!is_storage_hash(&"zz".repeat(32)));
        assert!(!is_storage_hash(&format!("{}/", "a".repeat(63))));
    }
}
