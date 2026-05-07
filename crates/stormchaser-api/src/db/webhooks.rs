use sqlx::PgPool;
use stormchaser_model::event_rules::WebhookConfig;
use stormchaser_model::WebhookId;

/// Creates a new webhook configuration.
pub async fn create_webhook(
    pool: &PgPool,
    id: WebhookId,
    name: &str,
    description: &Option<String>,
    source_type: &str,
    secret_token: &Option<String>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO webhooks (id, name, description, source_type, secret_token) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(source_type)
    .bind(secret_token)
    .execute(pool)
    .await?;
    Ok(())
}

/// Retrieves all configured webhooks
pub async fn list_webhooks(pool: &PgPool) -> Result<Vec<WebhookConfig>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM webhooks ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
}

/// Retrieves a specific webhook by ID.
/// Get webhook.
pub async fn get_webhook(
    pool: &PgPool,
    id: WebhookId,
) -> Result<Option<WebhookConfig>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM webhooks WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Retrieves an active webhook by ID.
/// Get active webhook.
pub async fn get_active_webhook(
    pool: &PgPool,
    id: WebhookId,
) -> Result<Option<WebhookConfig>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM webhooks WHERE id = $1 AND is_active = TRUE")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Updates an existing webhook configuration.
pub async fn update_webhook(
    pool: &PgPool,
    id: WebhookId,
    name: Option<String>,
    description: Option<Option<String>>,
    source_type: Option<String>,
    secret_token: Option<Option<String>>,
    is_active: Option<bool>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    let mut webhook =
        match sqlx::query_as::<_, WebhookConfig>("SELECT * FROM webhooks WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
        {
            Some(w) => w,
            None => return Err(sqlx::Error::RowNotFound),
        };

    if let Some(n) = name {
        webhook.name = n;
    }
    if let Some(d) = description {
        webhook.description = d;
    }
    if let Some(st) = source_type {
        webhook.source_type = st;
    }
    if let Some(t) = secret_token {
        webhook.secret_token = t;
    }
    if let Some(a) = is_active {
        webhook.is_active = a;
    }

    sqlx::query(
        "UPDATE webhooks SET name = $1, description = $2, source_type = $3, secret_token = $4, is_active = $5, updated_at = NOW() WHERE id = $6",
    )
    .bind(webhook.name)
    .bind(webhook.description)
    .bind(webhook.source_type)
    .bind(webhook.secret_token)
    .bind(webhook.is_active)
    .bind(id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Deletes a webhook from the database.
/// Delete webhook.
pub async fn delete_webhook(pool: &PgPool, id: WebhookId) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM webhooks WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Inserts a new webhook.
pub async fn insert_webhook(
    pool: &PgPool,
    id: WebhookId,
    name: &str,
    description: &Option<String>,
    source_type: &str,
    secret_token: &Option<String>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO webhooks (id, name, description, source_type, secret_token) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(source_type)
    .bind(secret_token)
    .execute(pool)
    .await?;
    Ok(())
}
