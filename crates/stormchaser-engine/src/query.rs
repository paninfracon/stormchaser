use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;

/// Longest a `sql` query may run.
const SQL_QUERY_TIMEOUT_MS: u64 = 30_000;

/// Most rows a `sql` query may return.
const MAX_SQL_QUERY_ROWS: usize = 10_000;

pub async fn execute_query(
    query_type: &str,
    params: &HashMap<String, String>,
    pool: Option<&PgPool>,
    db_url_override: Option<String>,
    acting_user: Option<&str>,
) -> Result<Vec<Value>, anyhow::Error> {
    let mut db_url = db_url_override;

    if db_url.is_none() {
        if let Some(conn_name) = params.get("connection") {
            if let Some(p) = pool {
                let conn = crate::db::get_storage_backend_by_name::<
                    _,
                    stormchaser_model::connections::Connection,
                >(p, conn_name)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Connection '{}' not found", conn_name))?;
                if let Some(user) = acting_user {
                    crate::authz::ensure_user_may_use(p, user, &conn).await?;
                }

                if let Some(url) = conn.config.get("url").and_then(|u| u.as_str()) {
                    db_url = Some(url.to_string());
                }
            }
        }
    }

    if query_type == "sql" {
        execute_sql_query(params, db_url).await
    } else if query_type == "api" {
        execute_api_query(params).await
    } else if query_type == "mock" {
        let items_str = params
            .get("items")
            .ok_or_else(|| anyhow::anyhow!("Missing 'items' param"))?;
        let options: Vec<Value> = items_str
            .split(',')
            .map(|s| Value::String(s.to_string()))
            .collect();
        Ok(options)
    } else if query_type == "aws_cloudcontrol" {
        execute_aws_cloudcontrol_query(params).await
    } else {
        Err(anyhow::anyhow!(
            "Unsupported query protocol: {}",
            query_type
        ))
    }
}

async fn execute_sql_query(
    params: &HashMap<String, String>,
    db_url: Option<String>,
) -> Result<Vec<Value>, anyhow::Error> {
    let sql_query = params
        .get("query")
        .ok_or_else(|| anyhow::anyhow!("Missing 'query' param"))?;

    // The query text is the workflow author's own source, executed as written, so it only
    // ever runs against a database named by a connection — never Stormchaser's own.
    let url = db_url.ok_or_else(|| {
        anyhow::anyhow!("SQL queries require a 'connection' param naming a connection with a 'url'")
    })?;

    use futures::TryStreamExt;
    use sqlx::{AssertSqlSafe, Connection, Row};
    let mut results = Vec::new();

    // A query fills in a form; it has no business changing data. It runs as one statement in
    // a read-only transaction that is always rolled back, with a time limit and a row cap.
    let mut conn = sqlx::PgConnection::connect(&url).await?;
    let mut tx = conn.begin().await?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await?;
    sqlx::query(AssertSqlSafe(format!(
        "SET LOCAL statement_timeout = {}",
        SQL_QUERY_TIMEOUT_MS
    )))
    .execute(&mut *tx)
    .await?;

    let mut rows = Vec::new();
    {
        let mut stream = sqlx::query(AssertSqlSafe(sql_query.as_str())).fetch(&mut *tx);
        while let Some(row) = stream.try_next().await? {
            if rows.len() >= MAX_SQL_QUERY_ROWS {
                anyhow::bail!("SQL query returned more than {} rows", MAX_SQL_QUERY_ROWS);
            }
            rows.push(row);
        }
    }
    tx.rollback().await?;
    conn.close().await.ok();

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
}

async fn execute_api_query(params: &HashMap<String, String>) -> Result<Vec<Value>, anyhow::Error> {
    let url_str = params
        .get("url")
        .ok_or_else(|| anyhow::anyhow!("Missing 'url' param"))?;
    let url = if url_str.starts_with("http://") || url_str.starts_with("https://") {
        url_str.to_string()
    } else {
        format!("https://{}", url_str)
    };

    let method_str = params
        .get("method")
        .map(|s| s.to_uppercase())
        .unwrap_or_else(|| "GET".to_string());

    let (client, url) = stormchaser_model::net_guard::guarded_client(&url).await?;
    let mut req = match method_str.as_str() {
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "PATCH" => client.patch(url),
        "DELETE" => client.delete(url),
        _ => client.get(url),
    };

    if let Some(body) = params.get("body") {
        req = req.header("Content-Type", "application/json");
        req = req.body(body.clone());
    }

    for (k, v) in params {
        if let Some(header_name) = k.strip_prefix("header_") {
            req = req.header(header_name, v);
        }
    }

    let res = req.send().await?.error_for_status()?;
    let body = stormchaser_model::net_guard::read_body_capped(
        res,
        stormchaser_model::net_guard::MAX_RESPONSE_BYTES,
    )
    .await?;
    let response: Value = serde_json::from_slice(&body)?;

    if let Some(jq_filter_str) = params.get("jq_filter") {
        apply_jq_filter(jq_filter_str, response)
    } else {
        if let Value::Array(arr) = response {
            Ok(arr)
        } else {
            Err(anyhow::anyhow!(
                "Expected API to return a JSON array, but got an object. Use 'jq_filter' to extract array."
            ))
        }
    }
}

async fn execute_aws_cloudcontrol_query(
    params: &HashMap<String, String>,
) -> Result<Vec<Value>, anyhow::Error> {
    #[cfg(feature = "aws-cloudcontrol")]
    {
        let type_name = params
            .get("type_name")
            .ok_or_else(|| anyhow::anyhow!("Missing 'type_name' param"))?;

        let mut config_loader = aws_config::defaults(aws_config::BehaviorVersion::v2026_01_12());
        if let Some(region) = params.get("region") {
            config_loader = config_loader.region(aws_config::Region::new(region.clone()));
        }
        let config = config_loader.load().await;

        let client = if let Some(role_arn) = params.get("assume_role_arn") {
            crate::aws_guard::ensure_role_allowed(role_arn)?;
            let sts_client = aws_sdk_sts::Client::new(&config);
            let session_name = params
                .get("role_session_name")
                .cloned()
                .unwrap_or_else(|| "stormchaser-query".to_string());

            let assume_role_res = sts_client
                .assume_role()
                .role_arn(role_arn)
                .role_session_name(session_name)
                .send()
                .await?;

            let credentials = assume_role_res
                .credentials()
                .ok_or_else(|| anyhow::anyhow!("Missing credentials from assume_role"))?;

            let assumed_credentials = aws_sdk_cloudcontrol::config::Credentials::new(
                credentials.access_key_id(),
                credentials.secret_access_key(),
                Some(credentials.session_token().to_string()),
                None,
                "StsAssumedRole",
            );

            let provider =
                aws_sdk_cloudcontrol::config::SharedCredentialsProvider::new(assumed_credentials);

            let assumed_config = aws_sdk_cloudcontrol::config::Builder::from(&config)
                .credentials_provider(provider)
                .build();

            aws_sdk_cloudcontrol::Client::from_conf(assumed_config)
        } else {
            aws_sdk_cloudcontrol::Client::new(&config)
        };

        let mut request = client.list_resources().type_name(type_name);
        if let Some(type_version_id) = params.get("type_version_id") {
            request = request.type_version_id(type_version_id);
        }
        if let Some(resource_model) = params.get("resource_model") {
            request = request.resource_model(resource_model);
        }

        let mut all_resources = Vec::new();
        let mut next_token = None;

        loop {
            let mut req = request.clone();
            if let Some(token) = &next_token {
                req = req.next_token(token);
            }

            let response = req.send().await?;

            for desc in response.resource_descriptions() {
                if let Some(props) = desc.properties() {
                    if let Ok(val) = serde_json::from_str::<Value>(props) {
                        all_resources.push(val);
                    }
                }
            }

            next_token = response.next_token().map(|s| s.to_string());
            if next_token.is_none() {
                break;
            }
        }

        if let Some(jq_filter_str) = params.get("jq_filter") {
            apply_jq_filter(jq_filter_str, Value::Array(all_resources))
        } else {
            Ok(all_resources)
        }
    }
    #[cfg(not(feature = "aws-cloudcontrol"))]
    {
        Err(anyhow::anyhow!("Feature 'aws-cloudcontrol' is not enabled"))
    }
}

/// Converts a JSON value into a jq value.
pub(crate) fn json_to_jq_val(value: Value) -> Result<jaq_json::Val, anyhow::Error> {
    serde_json::from_value(value)
        .map_err(|e| anyhow::anyhow!("Failed to convert JQ input from JSON: {}", e))
}

/// Converts a single jq output (or exception) back into a JSON value.
///
/// A `halt` raised by the program is reported as an error rather than being
/// allowed to terminate the process.
pub(crate) fn jq_output_to_json(
    output: jaq_core::ValX<jaq_json::Val>,
) -> Result<Value, anyhow::Error> {
    match output {
        Ok(v) => serde_json::from_str(&v.to_string())
            .map_err(|e| anyhow::anyhow!("JQ output is not valid JSON: {}", e)),
        Err(exn) => match exn.get_err() {
            Ok(e) => Err(anyhow::anyhow!("JQ execution error: {:?}", e)),
            Err(_) => Err(anyhow::anyhow!("JQ execution error: program halted")),
        },
    }
}

/// Native jq functions available to workflow-supplied programs.
///
/// `env` is left out: it would return the engine's whole process environment.
pub(crate) fn jq_funs(
) -> impl Iterator<Item = jaq_core::native::Fun<jaq_core::data::JustLut<jaq_json::Val>>> {
    jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs())
        .filter(|(name, _, _)| *name != "env")
}

fn apply_jq_filter(jq_filter_str: &str, input_val: Value) -> Result<Vec<Value>, anyhow::Error> {
    use jaq_core::load::{Arena, File, Loader};
    use jaq_core::{data, Ctx, Vars};
    use jaq_json::Val;

    let loader = Loader::new(
        jaq_core::defs()
            .chain(jaq_std::defs())
            .chain(jaq_json::defs()),
    );
    let arena = Arena::default();
    let program = File {
        code: jq_filter_str,
        path: (),
    };

    let modules = loader
        .load(&arena, program)
        .map_err(|e| anyhow::anyhow!("Failed to parse jq filter: {:?}", e))?;

    let filter = jaq_core::Compiler::default()
        .with_funs(jq_funs())
        .compile(modules)
        .map_err(|e| anyhow::anyhow!("Failed to compile jq filter: {:?}", e))?;

    let input = json_to_jq_val(input_val)?;
    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    filter.id.run((ctx, input)).map(jq_output_to_json).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The mock servers listen on loopback, which the SSRF guard blocks by default.
    fn allow_loopback() {
        std::env::set_var(stormchaser_model::net_guard::ALLOWED_HOSTS_ENV, "127.0.0.1");
    }

    #[tokio::test]
    async fn test_execute_query_mock() {
        let mut params = HashMap::new();
        params.insert("items".to_string(), "a,b,c".to_string());
        let res = execute_query("mock", &params, None, None, None)
            .await
            .unwrap();
        assert_eq!(res.len(), 3);
        assert_eq!(res[0], serde_json::json!("a"));
    }

    #[tokio::test]
    async fn test_execute_query_api() {
        allow_loopback();
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/options"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(["x", "y"])))
            .mount(&mock_server)
            .await;

        let mut params = HashMap::new();
        params.insert(
            "url".to_string(),
            format!("{}/api/options", mock_server.uri()),
        );

        let res = execute_query("api", &params, None, None, None)
            .await
            .unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0], serde_json::json!("x"));
    }

    #[tokio::test]
    async fn test_execute_query_api_with_jq_filter() {
        allow_loopback();
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/complex"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "NextToken": "string",
                "ResourceDescriptions": [
                    { "Identifier": "Resource1" },
                    { "Identifier": "Resource2" }
                ],
                "TypeName": "string"
            })))
            .mount(&mock_server)
            .await;

        let mut params = HashMap::new();
        params.insert(
            "url".to_string(),
            format!("{}/api/complex", mock_server.uri()),
        );
        params.insert(
            "jq_filter".to_string(),
            ".ResourceDescriptions[].Identifier".to_string(),
        );

        let res = execute_query("api", &params, None, None, None)
            .await
            .unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0], serde_json::json!("Resource1"));
        assert_eq!(res[1], serde_json::json!("Resource2"));
    }

    #[tokio::test]
    async fn test_execute_query_api_post() {
        use wiremock::matchers::{body_json, header};
        allow_loopback();
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/complex"))
            .and(header("X-Custom-Auth", "my-secret-token"))
            .and(body_json(serde_json::json!({ "filter": "active" })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!(["result1", "result2"])),
            )
            .mount(&mock_server)
            .await;

        let mut params = HashMap::new();
        params.insert(
            "url".to_string(),
            format!("{}/api/complex", mock_server.uri()),
        );
        params.insert("method".to_string(), "POST".to_string());
        params.insert("body".to_string(), r#"{"filter": "active"}"#.to_string());
        params.insert(
            "header_X-Custom-Auth".to_string(),
            "my-secret-token".to_string(),
        );

        let res = execute_query("api", &params, None, None, None)
            .await
            .unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0], serde_json::json!("result1"));
        assert_eq!(res[1], serde_json::json!("result2"));
    }

    #[tokio::test]
    async fn test_execute_query_unsupported() {
        let res = execute_query("unknown", &HashMap::new(), None, None, None).await;
        let err = res.unwrap_err();
        assert!(err.to_string().contains("Unsupported query protocol"));
    }

    #[tokio::test]
    async fn test_execute_query_sql_requires_connection() {
        let mut params = HashMap::new();
        params.insert("query".to_string(), "SELECT 1".to_string());

        let err = execute_query("sql", &params, None, None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("require a 'connection'"));
    }

    #[test]
    fn test_jq_filter_cannot_read_process_env() {
        let err = apply_jq_filter("env", serde_json::json!(null)).unwrap_err();
        assert!(err.to_string().contains("Failed to compile jq filter"));

        // The rest of the standard library is still available.
        let res = apply_jq_filter(
            "[.[] | ascii_upcase] | length",
            serde_json::json!(["a", "b"]),
        )
        .unwrap();
        assert_eq!(res, vec![serde_json::json!(2)]);
    }

    #[tokio::test]
    async fn test_execute_query_api_refuses_internal_addresses() {
        let mut params = HashMap::new();
        params.insert(
            "url".to_string(),
            "http://169.254.169.254/latest/meta-data/".to_string(),
        );

        let err = execute_query("api", &params, None, None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("restricted address"));
    }
}
