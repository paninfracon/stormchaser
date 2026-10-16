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

/// Records that a webhook accepted the delivery identified by `key`.
///
/// Returns `false` if the same delivery was already accepted within the last
/// `retention_hours`: it is a replay or a redelivery and must not be acted on again.
/// Recording and checking are one statement, so two replicas receiving the same delivery
/// at once cannot both act on it.
pub async fn record_delivery(
    pool: &PgPool,
    webhook_id: WebhookId,
    key: &str,
    retention_hours: i32,
) -> Result<bool, sqlx::Error> {
    let recorded = sqlx::query(
        r#"
        INSERT INTO webhook_deliveries (webhook_id, delivery_key)
        VALUES ($1, $2)
        ON CONFLICT (webhook_id, delivery_key) DO UPDATE SET received_at = NOW()
        WHERE webhook_deliveries.received_at < NOW() - make_interval(hours => $3)
        "#,
    )
    .bind(webhook_id)
    .bind(key)
    .bind(retention_hours)
    .execute(pool)
    .await?
    .rows_affected()
        > 0;

    // Old records are of no further use; clearing them here keeps the table small
    // without a separate job. A failure only delays the cleanup.
    if let Err(e) = sqlx::query(
        "DELETE FROM webhook_deliveries WHERE received_at < NOW() - make_interval(hours => $1)",
    )
    .bind(retention_hours)
    .execute(pool)
    .await
    {
        tracing::warn!("Failed to clear old webhook delivery records: {:?}", e);
    }

    Ok(recorded)
}

/// Forgets a delivery that was recorded but could not be processed, so the sender's
/// retry is not mistaken for a replay.
pub async fn forget_delivery(
    pool: &PgPool,
    webhook_id: WebhookId,
    key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM webhook_deliveries WHERE webhook_id = $1 AND delivery_key = $2")
        .bind(webhook_id)
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
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
    pool: impl sqlx::PgExecutor<'_>,
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
