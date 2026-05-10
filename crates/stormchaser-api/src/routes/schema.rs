use crate::AppState;
use axum::{extract::State, response::IntoResponse, Json};
use serde_json::Value;
use stormchaser_model::schema_gen::generate_dsl_schema;
use tokio_stream::StreamExt;

/// Retrieves the base JSON schema for the Stormchaser DSL.
#[utoipa::path(
    get,
    path = "/api/v1/schema",
    tag = "stormchaser",
    responses(
        (status = 200, description = "JSON schema retrieved successfully", body = serde_json::Value)
    )
)]
pub async fn get_schema() -> impl IntoResponse {
    let schema = generate_dsl_schema();
    Json(schema)
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct HydrateSchemaRequest {
    #[schema(value_type = Object)]
    pub schema: Value,
    #[schema(value_type = Object, default = "{}")]
    #[serde(default)]
    pub inputs: Value,
    #[schema(default = "[]")]
    #[serde(default)]
    pub queries: Vec<Value>,
}

#[derive(serde::Serialize, Clone, Debug, PartialEq, utoipa::ToSchema)]
pub enum HydrationStatus {
    #[serde(rename = "Update pending")]
    UpdatePending,
    #[serde(rename = "Incomplete Input")]
    IncompleteInput,
    #[serde(rename = "Schema validation failed")]
    SchemaValidationFailed,
    #[serde(rename = "Completed")]
    Completed,
}

#[derive(serde::Serialize, Clone, Debug, utoipa::ToSchema)]
pub struct HydrationEvent {
    pub status: HydrationStatus,
    #[schema(value_type = Object)]
    pub hydrated_schema: Value,
    pub query_status: std::collections::HashMap<String, String>,
    pub validation_errors: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/api/v1/schema/hydrate",
    request_body = HydrateSchemaRequest,
    tag = "stormchaser",
    security(
        ("bearer_auth" = [])
    ),
    responses(
        (status = 200, description = "Hydrated schema event stream")
    )
)]
pub async fn hydrate_schema(
    State(state): State<AppState>,
    Json(payload): Json<HydrateSchemaRequest>,
) -> impl IntoResponse {
    let queries: Vec<stormchaser_model::dsl::Query> = payload
        .queries
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();

    let (tx, rx) = tokio::sync::mpsc::channel(100);

    tokio::spawn(async move {
        run_hydration_loop(
            payload.schema,
            payload.inputs,
            queries,
            Some(state.pool),
            tx,
        )
        .await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|event| {
        Ok::<_, std::convert::Infallible>(
            axum::response::sse::Event::default()
                .json_data(event)
                .unwrap(),
        )
    });

    axum::response::sse::Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

#[derive(Clone, Debug)]
struct QueryTask {
    field_name: String,
    query_type: String,
    params: std::collections::HashMap<String, String>,
    dependencies: Vec<String>,
    status: String,
}

use hcl::expr::*;

fn extract_vars(expr: &Expression, deps: &mut std::collections::HashSet<String>) {
    match expr {
        Expression::Traversal(t) => {
            if let Expression::Variable(var) = &t.expr {
                if var.as_str() == "inputs" {
                    if let Some(TraversalOperator::GetAttr(attr)) = t.operators.first() {
                        deps.insert(attr.as_str().to_string());
                    }
                }
            }
        }
        Expression::FuncCall(f) => {
            for arg in &f.args {
                extract_vars(arg, deps);
            }
        }
        Expression::Array(arr) => {
            for e in arr {
                extract_vars(e, deps);
            }
        }
        Expression::Object(obj) => {
            for (k, v) in obj {
                if let ObjectKey::Expression(e) = k {
                    extract_vars(e, deps);
                }
                extract_vars(v, deps);
            }
        }
        Expression::Parenthesis(p) => {
            extract_vars(p, deps);
        }
        Expression::Conditional(c) => {
            extract_vars(&c.cond_expr, deps);
            extract_vars(&c.true_expr, deps);
            extract_vars(&c.false_expr, deps);
        }
        Expression::Operation(op) => match &**op {
            Operation::Unary(u) => {
                extract_vars(&u.expr, deps);
            }
            Operation::Binary(b) => {
                extract_vars(&b.lhs_expr, deps);
                extract_vars(&b.rhs_expr, deps);
            }
        },
        Expression::ForExpr(f) => {
            extract_vars(&f.collection_expr, deps);
            extract_vars(&f.value_expr, deps);
            if let Some(k) = &f.key_expr {
                extract_vars(k, deps);
            }
            if let Some(c) = &f.cond_expr {
                extract_vars(c, deps);
            }
        }
        _ => {}
    }
}

fn extract_template_deps(query: &str) -> Vec<String> {
    let mut deps = std::collections::HashSet::new();
    if let Ok(template) = query.parse::<hcl::Template>() {
        for el in template.elements() {
            match el {
                hcl::template::Element::Interpolation(interp) => {
                    extract_vars(&interp.expr, &mut deps);
                }
                hcl::template::Element::Directive(dir) => match &**dir {
                    hcl::template::Directive::If(i) => {
                        extract_vars(&i.cond_expr, &mut deps);
                        for e in i.true_template.elements() {
                            if let hcl::template::Element::Interpolation(interp) = e {
                                extract_vars(&interp.expr, &mut deps);
                            }
                        }
                        if let Some(f) = &i.false_template {
                            for e in f.elements() {
                                if let hcl::template::Element::Interpolation(interp) = e {
                                    extract_vars(&interp.expr, &mut deps);
                                }
                            }
                        }
                    }
                    hcl::template::Directive::For(f) => {
                        extract_vars(&f.collection_expr, &mut deps);
                        for e in f.template.elements() {
                            if let hcl::template::Element::Interpolation(interp) = e {
                                extract_vars(&interp.expr, &mut deps);
                            }
                        }
                    }
                },
                _ => {}
            }
        }
    }
    let mut result: Vec<String> = deps.into_iter().collect();
    result.sort();
    result
}

fn extract_queries(queries: &[stormchaser_model::dsl::Query], tasks: &mut Vec<QueryTask>) {
    for q in queries {
        let mut deps = std::collections::HashSet::new();
        // Extract deps from params
        for val in q.params.values() {
            let extracted = extract_template_deps(val);
            for d in extracted {
                deps.insert(d);
            }
        }

        let mut deps_vec: Vec<String> = deps.into_iter().collect();
        deps_vec.sort();

        tasks.push(QueryTask {
            field_name: q.name.clone(),
            query_type: q.r#type.clone(),
            params: q.params.clone(),
            dependencies: deps_vec,
            status: "Pending".to_string(),
        });
    }
}

async fn emit_event(
    schema: &Value,
    inputs: &Value,
    tasks: &[QueryTask],
    tx: &tokio::sync::mpsc::Sender<HydrationEvent>,
) -> Result<(), ()> {
    let mut query_status = std::collections::HashMap::new();
    let mut any_running = false;
    for task in tasks {
        query_status.insert(task.field_name.clone(), task.status.clone());
        if task.status == "Running" {
            any_running = true;
        }
    }

    let mut validation_errors = vec![];
    let mut is_incomplete = false;
    let mut schema_invalid = false;

    match jsonschema::validator_for(schema) {
        Ok(validator) => {
            if let Err(e) = validator.validate(inputs) {
                let msg = e.to_string();
                if msg.contains("is a required property") {
                    is_incomplete = true;
                }
                validation_errors.push(msg);
            }
        }
        Err(e) => {
            schema_invalid = true;
            validation_errors.push(format!("Invalid JSON Schema: {}", e));
        }
    }

    let status = if any_running {
        HydrationStatus::UpdatePending
    } else if schema_invalid {
        HydrationStatus::SchemaValidationFailed
    } else if validation_errors.is_empty() {
        HydrationStatus::Completed
    } else if is_incomplete {
        HydrationStatus::IncompleteInput
    } else {
        HydrationStatus::SchemaValidationFailed
    };

    let event = HydrationEvent {
        status,
        hydrated_schema: schema.clone(),
        query_status,
        validation_errors,
    };

    tx.send(event).await.map_err(|_| ())
}

async fn run_hydration_loop(
    mut schema: Value,
    inputs: Value,
    queries: Vec<stormchaser_model::dsl::Query>,
    pool: Option<sqlx::PgPool>,
    tx: tokio::sync::mpsc::Sender<HydrationEvent>,
) {
    let mut tasks = Vec::new();
    extract_queries(&queries, &mut tasks);

    let mut join_set = tokio::task::JoinSet::new();

    let mut tasks_to_run = Vec::new();
    for (idx, task) in tasks.iter_mut().enumerate() {
        let missing_deps = task.dependencies.iter().any(|dep| match inputs.get(dep) {
            Some(Value::Null) | None => true,
            Some(Value::String(s)) if s.is_empty() => true,
            _ => false,
        });

        if missing_deps {
            task.status = "Pending Dependency".to_string();
        } else {
            task.status = "Running".to_string();
            tasks_to_run.push(idx);
        }
    }

    if emit_event(&schema, &inputs, &tasks, &tx).await.is_err() {
        return;
    }

    let mut hcl_ctx = hcl::eval::Context::new();
    hcl_ctx.declare_var(
        "inputs",
        stormchaser_model::hcl_eval::json_to_hcl(inputs.clone()),
    );

    for idx in tasks_to_run {
        let task = &mut tasks[idx];

        let mut resolved_params = task.params.clone();
        for val in resolved_params.values_mut() {
            if let Ok(Some(eval_res)) = stormchaser_model::hcl_eval::evaluate_string(val, &hcl_ctx)
            {
                if let Value::String(s) = eval_res {
                    *val = s;
                } else {
                    *val = eval_res.to_string();
                }
            }
        }

        let pool_ref = pool.clone();
        let query_type = task.query_type.clone();
        let params_clone = resolved_params.clone();

        join_set.spawn(async move {
            let res = execute_query(&query_type, &params_clone, pool_ref.as_ref()).await;
            (idx, res)
        });
    }

    let mut query_results = serde_json::Map::new();

    while let Some(res) = join_set.join_next().await {
        match res {
            Ok((idx, Ok(options))) => {
                tasks[idx].status = "Resolved".to_string();
                query_results.insert(tasks[idx].field_name.clone(), Value::Array(options));
            }
            Ok((idx, Err(e))) => {
                tasks[idx].status = format!("Failed: {}", e);
            }
            Err(_) => {}
        }

        if emit_event(&schema, &inputs, &tasks, &tx).await.is_err() {
            break;
        }
    }

    // Now resolve schema using the query_results
    let mut schema_ctx = hcl::eval::Context::new();
    schema_ctx.declare_var(
        "inputs",
        stormchaser_model::hcl_eval::json_to_hcl(inputs.clone()),
    );
    schema_ctx.declare_var(
        "queries",
        stormchaser_model::hcl_eval::json_to_hcl(Value::Object(query_results)),
    );

    let _ = stormchaser_model::hcl_eval::resolve_expressions(&mut schema, &schema_ctx);

    // Emit the final fully hydrated schema
    let _ = emit_event(&schema, &inputs, &tasks, &tx).await;
}

async fn execute_query(
    query_type: &str,
    params: &std::collections::HashMap<String, String>,
    pool: Option<&sqlx::PgPool>,
) -> Result<Vec<Value>, anyhow::Error> {
    if query_type == "sql" {
        if let Some(p) = pool {
            let sql_query = params
                .get("query")
                .ok_or_else(|| anyhow::anyhow!("Missing 'query' param"))?;
            use sqlx::Row;

            let rows = sqlx::query(sql_query).fetch_all(p).await?;
            let mut results = Vec::new();
            for row in rows {
                if row.is_empty() {
                    continue;
                }
                if let Ok(val) = row.try_get::<String, _>(0) {
                    results.push(Value::String(val));
                } else if let Ok(val) = row.try_get::<i64, _>(0) {
                    results.push(serde_json::json!(val));
                } else if let Ok(val) = row.try_get::<bool, _>(0) {
                    results.push(Value::Bool(val));
                }
            }
            Ok(results)
        } else {
            Err(anyhow::anyhow!("SQL pool not available"))
        }
    } else if query_type == "api" {
        let url_str = params
            .get("url")
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' param"))?;
        let url = format!("https://{}", url_str);
        let response = reqwest::get(&url).await?.json::<Vec<Value>>().await?;
        Ok(response)
    } else if query_type == "mock" {
        let items_str = params
            .get("items")
            .ok_or_else(|| anyhow::anyhow!("Missing 'items' param"))?;
        let options: Vec<Value> = items_str
            .split(',')
            .map(|s| Value::String(s.to_string()))
            .collect();
        Ok(options)
    } else {
        Err(anyhow::anyhow!("Unsupported query protocol"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_template_deps() {
        let q = "SELECT ${inputs.env} UNION ${inputs.prod}";
        let mut deps = extract_template_deps(q);
        deps.sort();
        assert_eq!(deps, vec!["env", "prod"]);

        let q2 = "%{ if inputs.active }yes%{ endif }";
        let deps2 = extract_template_deps(q2);
        assert_eq!(deps2, vec!["active"]);
    }

    #[tokio::test]
    async fn test_execute_query_mock() {
        let mut params = std::collections::HashMap::new();
        params.insert("items".to_string(), "a,b,c".to_string());
        let res = execute_query("mock", &params, None).await.unwrap();
        assert_eq!(res, vec![json!("a"), json!("b"), json!("c")]);
    }

    #[tokio::test]
    async fn test_run_hydration_loop_success() {
        let schema = json!({
            "type": "object",
            "properties": {
                "dep": {
                    "type": "string"
                },
                "target": {
                    "type": "string",
                    "enum": "${queries.target}"
                }
            },
            "required": ["dep", "target"]
        });

        let mut query = stormchaser_model::dsl::Query {
            name: "target".to_string(),
            r#type: "mock".to_string(),
            params: std::collections::HashMap::new(),
        };
        query.params.insert(
            "items".to_string(),
            "${inputs.dep}1,${inputs.dep}2".to_string(),
        );

        let queries = vec![query];

        // 1. Missing dependency
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        let inputs_missing = json!({});
        run_hydration_loop(schema.clone(), inputs_missing, queries.clone(), None, tx).await;

        let event1 = rx.recv().await.unwrap();
        assert_eq!(event1.status, HydrationStatus::SchemaValidationFailed);
        assert_eq!(
            event1.query_status.get("target").unwrap(),
            "Pending Dependency"
        );

        // 2. Dependency provided
        let (tx2, mut rx2) = tokio::sync::mpsc::channel(100);
        let inputs_provided = json!({"dep": "val"});
        run_hydration_loop(schema.clone(), inputs_provided, queries.clone(), None, tx2).await;

        let event_start = rx2.recv().await.unwrap();
        assert_eq!(event_start.status, HydrationStatus::UpdatePending);
        assert_eq!(event_start.query_status.get("target").unwrap(), "Running");

        // End event 1 (resolved query but schema before final hydration)
        let _ = rx2.recv().await.unwrap();

        // Final event (hydrated schema)
        let event_end = rx2.recv().await.unwrap();
        println!("event_end: {:?}", event_end);
        assert_eq!(event_end.status, HydrationStatus::IncompleteInput); // Still missing 'target' input for full validation
        assert_eq!(event_end.query_status.get("target").unwrap(), "Resolved");

        let target_enum = event_end.hydrated_schema["properties"]["target"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(target_enum[0], json!("val1"));
        assert_eq!(target_enum[1], json!("val2"));
    }
}
