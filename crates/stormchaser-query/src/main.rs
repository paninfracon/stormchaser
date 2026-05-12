use axum::{extract::State, middleware, response::IntoResponse, routing::post, Json, Router};
use serde_json::Value;
use std::env;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_stream::StreamExt;

// Import Config, AppState, and setup from stormchaser_api to maintain identical configuration
use stormchaser_api::auth::opa::opa_middleware;
use stormchaser_api::config::Config;
use stormchaser_api::setup::build_app_state;
use stormchaser_api::AppState;

#[derive(serde::Deserialize)]
pub struct HydrateSchemaRequest {
    pub schema: Value,
    #[serde(default)]
    pub inputs: Value,
    #[serde(default)]
    pub queries: Vec<Value>,
}

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
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

#[derive(serde::Serialize, Clone, Debug)]
pub struct HydrationEvent {
    pub status: HydrationStatus,
    pub hydrated_schema: Value,
    pub query_status: std::collections::HashMap<String, String>,
    pub validation_errors: Vec<String>,
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

    let hydrated_schema = stormchaser_model::schema_gen::flatten_schema_for_ui(schema, inputs);

    let event = HydrationEvent {
        status,
        hydrated_schema,
        query_status,
        validation_errors,
    };

    tx.send(event).await.map_err(|_| ())
}

async fn execute_query(
    query_type: &str,
    params: &std::collections::HashMap<String, String>,
    state: Option<&AppState>,
) -> Result<Vec<Value>, anyhow::Error> {
    let mut db_url = None;

    if let Some(conn_name) = params.get("connection") {
        if let Some(app_state) = state {
            let conn = stormchaser_engine::db::get_storage_backend_by_name::<
                _,
                stormchaser_model::connections::Connection,
            >(&app_state.pool, conn_name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Connection '{}' not found", conn_name))?;

            if app_state.opa.is_configured() {
                let opa_ctx = stormchaser_model::auth::ConnectionOpaContext {
                    connection_name: conn_name,
                    // In a real scenario we extract the username from JWT claims.
                    initiating_user: "schema_hydration_service",
                };
                if !app_state.opa.check_connection(opa_ctx).await? {
                    return Err(anyhow::anyhow!(
                        "Access to connection '{}' denied by OPA",
                        conn_name
                    ));
                }
            }

            if let Some(url) = conn.config.get("url").and_then(|u| u.as_str()) {
                db_url = Some(url.to_string());
            }
        }
    }

    let pool = state.map(|s| &s.pool);
    stormchaser_engine::query::execute_query(query_type, params, pool, db_url).await
}

async fn run_hydration_loop(
    mut schema: Value,
    inputs: Value,
    queries: Vec<stormchaser_model::dsl::Query>,
    state: Option<AppState>,
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

    let any_running_initial = tasks.iter().any(|t| t.status == "Running");
    if any_running_initial {
        // Skip validation on initial emit before queries are resolved since schemas with unrendered templates will fail
        let status = HydrationStatus::UpdatePending;
        let hydrated_schema =
            stormchaser_model::schema_gen::flatten_schema_for_ui(&schema, &inputs);
        let mut query_status = std::collections::HashMap::new();
        for task in &tasks {
            query_status.insert(task.field_name.clone(), task.status.clone());
        }
        let event = HydrationEvent {
            status,
            hydrated_schema,
            query_status,
            validation_errors: vec![],
        };
        if tx.send(event).await.is_err() {
            return;
        }
    } else {
        if emit_event(&schema, &inputs, &tasks, &tx).await.is_err() {
            return;
        }
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

        let state_for_query = state.clone();
        let query_type = task.query_type.clone();
        let params_clone = resolved_params.clone();

        join_set.spawn(async move {
            let res = execute_query(&query_type, &params_clone, state_for_query.as_ref()).await;
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

        let any_running = tasks.iter().any(|t| t.status == "Running");
        if any_running {
            let status = HydrationStatus::UpdatePending;
            let hydrated_schema =
                stormchaser_model::schema_gen::flatten_schema_for_ui(&schema, &inputs);
            let mut query_status = std::collections::HashMap::new();
            for task in &tasks {
                query_status.insert(task.field_name.clone(), task.status.clone());
            }
            let event = HydrationEvent {
                status,
                hydrated_schema,
                query_status,
                validation_errors: vec![],
            };
            if tx.send(event).await.is_err() {
                break;
            }
        }
    }

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
    let _ = emit_event(&schema, &inputs, &tasks, &tx).await;
}

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

    let state_clone = state.clone();
    tokio::spawn(async move {
        run_hydration_loop(
            payload.schema,
            payload.inputs,
            queries,
            Some(state_clone),
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    tracing::info!("Stormchaser Query Microservice starting");

    let config = Config::from_env(env::vars())?;

    // Uses the exact same state builder as stormchaser-api to guarantee TLS, DB, NATS,
    // OIDC, and OPA configuration parity.
    let state = build_app_state(config).await?;

    let app = Router::new()
        .route("/healthz", axum::routing::get(|| async { "OK" }))
        .route("/api/health", axum::routing::get(|| async { "OK" }))
        .route("/api/v1/schema/hydrate", post(hydrate_schema))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            opa_middleware, // Reusing identical authentication and OPA gates
        ))
        .with_state(state);

    let port = env::var("PORT").unwrap_or_else(|_| "3001".to_string());
    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;

    tracing::info!("Stormchaser Query Microservice listening on {}", addr);
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
