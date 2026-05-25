use crate::models::{HydrationEvent, HydrationStatus, QueryTask};
use hcl::expr::*;
use serde_json::Value;
use stormchaser_api::AppState;

pub fn extract_vars(expr: &Expression, deps: &mut std::collections::HashSet<String>) {
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

pub fn extract_template_deps(query: &str) -> Vec<String> {
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

pub fn extract_queries(queries: &[stormchaser_model::dsl::Query], tasks: &mut Vec<QueryTask>) {
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

pub async fn emit_event(
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

pub async fn execute_query(
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

pub async fn run_hydration_loop(
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

    let _ = stormchaser_model::hcl_eval::resolve_expressions(&mut schema, &schema_ctx, false);
    let _ = emit_event(&schema, &inputs, &tasks, &tx).await;
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

        let mut final_event = None;
        while let Some(evt) = rx2.recv().await {
            final_event = Some(evt);
        }

        let event_end = final_event.unwrap();
        assert_eq!(event_end.status, HydrationStatus::IncompleteInput); // Still missing 'target' input for full validation
        assert_eq!(event_end.query_status.get("target").unwrap(), "Resolved");

        let target_enum = event_end.hydrated_schema["properties"]["target"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(target_enum[0], json!("val1"));
        assert_eq!(target_enum[1], json!("val2"));
    }
}
