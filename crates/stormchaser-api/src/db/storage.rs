use crate::UpdateStorageBackendRequest;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use stormchaser_model::BackendId;

use stormchaser_model::storage;

/// Unsets the default Stormchaser File System.
/// Unset default sfs.
pub async fn unset_default_sfs(tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE storage_backends SET is_default_sfs = FALSE WHERE is_default_sfs = TRUE")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Creates a new storage backend.
/// Create storage backend.
#[allow(clippy::too_many_arguments)]
pub async fn create_storage_backend(
    tx: &mut Transaction<'_, Postgres>,
    id: BackendId,
    name: &str,
    description: &Option<String>,
    backend_type: &storage::BackendType,
    config: &Value,
    aws_assume_role_arn: &Option<String>,
    is_default_sfs: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO storage_backends (id, name, description, backend_type, config, aws_assume_role_arn, is_default_sfs)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(backend_type)
    .bind(config)
    .bind(aws_assume_role_arn)
    .bind(is_default_sfs)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Retrieves all storage backends.
/// List storage backends.
pub async fn list_storage_backends(
    pool: &PgPool,
) -> Result<Vec<storage::StorageBackend>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM storage_backends ORDER BY name ASC")
        .fetch_all(pool)
        .await
}

/// Retrieves a storage backend by ID.
/// Get storage backend.
pub async fn get_storage_backend(
    pool: &PgPool,
    id: BackendId,
) -> Result<Option<storage::StorageBackend>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM storage_backends WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Updates an existing storage backend.
/// Update storage backend.
pub async fn update_storage_backend(
    tx: &mut Transaction<'_, Postgres>,
    id: BackendId,
    payload: &UpdateStorageBackendRequest,
) -> Result<(), sqlx::Error> {
    let mut query = sqlx::QueryBuilder::new("UPDATE storage_backends SET ");
    let mut separated = query.separated(", ");

    if let Some(name) = &payload.name {
        separated.push("name = ").push_bind_unseparated(name);
    }
    if let Some(desc) = &payload.description {
        separated.push("description = ").push_bind_unseparated(desc);
    }
    if let Some(bt) = &payload.backend_type {
        separated.push("backend_type = ").push_bind_unseparated(bt);
    }
    if let Some(cfg) = &payload.config {
        separated.push("config = ").push_bind_unseparated(cfg);
    }
    if let Some(role) = &payload.aws_assume_role_arn {
        // An empty string is treated as a request to clear the ARN (set to NULL).
        let value: Option<&str> = if role.is_empty() {
            None
        } else {
            Some(role.as_str())
        };
        separated
            .push("aws_assume_role_arn = ")
            .push_bind_unseparated(value);
    }
    if let Some(is_default) = payload.is_default_sfs {
        separated
            .push("is_default_sfs = ")
            .push_bind_unseparated(is_default);
    }

    query.push(" WHERE id = ").push_bind(id);

    query.build().execute(&mut **tx).await?;
    Ok(())
}

/// Deletes a storage backend from the database.
/// Delete storage backend.
pub async fn delete_storage_backend(pool: &PgPool, id: BackendId) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM storage_backends WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
