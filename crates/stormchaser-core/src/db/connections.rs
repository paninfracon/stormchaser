use sqlx::postgres::PgRow;
use sqlx::{Executor, Postgres};

#[allow(clippy::too_many_arguments)]
/// Get storage backend by name.
pub async fn get_storage_backend_by_name<'a, E, O>(
    executor: E,
    name: &str,
) -> Result<Option<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, PgRow>,
{
    sqlx::query_as::<_, O>("SELECT * FROM connections WHERE name = $1")
        .bind(name)
        .fetch_optional(executor)
        .await
}

/// Retrieves a storage backend by ID.
/// Get storage backend.
pub async fn get_connection<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    executor: E,
    id: stormchaser_model::ConnectionId,
) -> Result<Option<stormchaser_model::Connection>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM connections WHERE id = $1")
        .bind(id)
        .fetch_optional(executor)
        .await
}

/// Retrieves a storage backend by its unique name.
///
/// Unlike [`get_connection`], which looks up by [`ConnectionId`], this helper
/// resolves a connection using the user-facing `name` field.
pub async fn get_connection_by_name<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    executor: E,
    name: &str,
) -> Result<Option<stormchaser_model::Connection>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM connections WHERE name = $1")
        .bind(name)
        .fetch_optional(executor)
        .await
}
