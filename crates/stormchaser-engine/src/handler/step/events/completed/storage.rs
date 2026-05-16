use anyhow::Result;

pub async fn persist_storage_hashes(
    tx: &mut sqlx::PgConnection,
    run_id: uuid::Uuid,
    storage_hashes: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<()> {
    let Some(hashes) = storage_hashes else {
        return Ok(());
    };
    for (name, hash_val) in hashes {
        if let Some(hash) = hash_val.as_str() {
            crate::db::upsert_run_storage_state(&mut *tx, run_id, name, hash).await?;
        }
    }
    Ok(())
}
