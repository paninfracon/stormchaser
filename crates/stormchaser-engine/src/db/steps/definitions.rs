use serde_json::Value;
use sqlx::{Executor, Postgres};

/// Stepdefinitioninput.
pub struct StepDefinitionInput {
    /// The step type.
    pub step_type: String,
    /// The schema.
    pub schema: Value,
    /// The documentation.
    pub documentation: Option<String>,
}

#[allow(clippy::too_many_arguments)]
/// Upsert step definition.
pub async fn upsert_step_definition<'a, E>(
    executor: E,
    step_type: &str,
    schema: &Value,
    documentation: Option<&str>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                INSERT INTO step_definitions (step_type, schema, documentation, registered_at)
                VALUES ($1, $2, $3, NOW())
                ON CONFLICT (step_type) DO UPDATE SET
                    schema = EXCLUDED.schema,
                    documentation = EXCLUDED.documentation
                "#,
    )
    .bind(step_type)
    .bind(schema)
    .bind(documentation)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Upsert step definition with wasm.
pub async fn upsert_step_definition_with_wasm<'a, E>(
    executor: E,
    step_type: &str,
    schema: &Value,
    documentation: Option<&str>,
    wasm_module: &str,
    wasm_function: &str,
    wasm_config: &Value,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO step_definitions (step_type, schema, documentation, registered_at, wasm_module, wasm_function, wasm_config)
        VALUES ($1, $2, $3, NOW(), $4, $5, $6)
        ON CONFLICT (step_type) DO UPDATE SET
            schema = EXCLUDED.schema,
            documentation = EXCLUDED.documentation,
            wasm_module = EXCLUDED.wasm_module,
            wasm_function = EXCLUDED.wasm_function,
            wasm_config = EXCLUDED.wasm_config
        "#,
    )
    .bind(step_type)
    .bind(schema)
    .bind(documentation)
    .bind(wasm_module)
    .bind(wasm_function)
    .bind(wasm_config)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Get wasm step definition.
pub async fn get_wasm_step_definition<'a, E, O>(
    executor: E,
    step_type: &str,
) -> Result<Option<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as::<_, O>(
        "SELECT wasm_module, wasm_function, wasm_config FROM step_definitions WHERE step_type = $1 AND wasm_module IS NOT NULL"
    )
    .bind(step_type)
    .fetch_optional(executor)
    .await
}
