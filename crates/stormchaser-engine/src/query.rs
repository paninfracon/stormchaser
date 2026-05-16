use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;

pub async fn execute_query(
    query_type: &str,
    params: &HashMap<String, String>,
    pool: Option<&PgPool>,
    db_url_override: Option<String>,
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

                if let Some(url) = conn.config.get("url").and_then(|u| u.as_str()) {
                    db_url = Some(url.to_string());
                }
            }
        }
    }

    if query_type == "sql" {
        let sql_query = params
            .get("query")
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' param"))?;

        use sqlx::Row;
        let mut results = Vec::new();

        if let Some(url) = db_url {
            let tmp_pool = sqlx::PgPool::connect(&url).await?;
            let rows = sqlx::query(sql_query).fetch_all(&tmp_pool).await?;
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
        } else if let Some(p) = pool {
            let rows = sqlx::query(sql_query).fetch_all(p).await?;
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
        } else {
            return Err(anyhow::anyhow!(
                "No connection and no system pool available"
            ));
        }
        Ok(results)
    } else if query_type == "api" {
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

        let client = reqwest::Client::new();
        let mut req = match method_str.as_str() {
            "POST" => client.post(&url),
            "PUT" => client.put(&url),
            "PATCH" => client.patch(&url),
            "DELETE" => client.delete(&url),
            _ => client.get(&url),
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

        let response = req
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;

        if let Some(jq_filter_str) = params.get("jq_filter") {
            use jaq_core::load::{Arena, File, Loader};
            use jaq_core::{Ctx, RcIter};
            use jaq_json::Val;

            let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));
            let arena = Arena::default();
            let program = File {
                code: jq_filter_str.as_str(),
                path: (),
            };

            let modules = loader
                .load(&arena, program)
                .map_err(|e| anyhow::anyhow!("Failed to parse jq filter: {:?}", e))?;

            let filter = jaq_core::Compiler::default()
                .with_funs(jaq_std::funs().chain(jaq_json::funs()))
                .compile(modules)
                .map_err(|e| anyhow::anyhow!("Failed to compile jq filter: {:?}", e))?;

            let input = Val::from(response);
            let inputs = RcIter::new(core::iter::empty());
            let out = filter.run((Ctx::new([], &inputs), input));

            let mut results = Vec::new();
            for res in out {
                match res {
                    Ok(v) => results.push(Value::from(v)),
                    Err(e) => return Err(anyhow::anyhow!("JQ execution error: {:?}", e)),
                }
            }
            Ok(results)
        } else {
            if let Value::Array(arr) = response {
                Ok(arr)
            } else {
                Err(anyhow::anyhow!(
                    "Expected API to return a JSON array, but got an object. Use 'jq_filter' to extract array."
                ))
            }
        }
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
        #[cfg(feature = "aws-cloudcontrol")]
        {
            let type_name = params
                .get("type_name")
                .ok_or_else(|| anyhow::anyhow!("Missing 'type_name' param"))?;

            let mut config_loader =
                aws_config::defaults(aws_config::BehaviorVersion::v2026_01_12());
            if let Some(region) = params.get("region") {
                config_loader = config_loader.region(aws_config::Region::new(region.clone()));
            }
            let config = config_loader.load().await;

            let client = if let Some(role_arn) = params.get("assume_role_arn") {
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

                let provider = aws_sdk_cloudcontrol::config::SharedCredentialsProvider::new(
                    assumed_credentials,
                );

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
                use jaq_core::load::{Arena, File, Loader};
                use jaq_core::{Ctx, RcIter};
                use jaq_json::Val;

                let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));
                let arena = Arena::default();
                let program = File {
                    code: jq_filter_str.as_str(),
                    path: (),
                };

                let modules = loader
                    .load(&arena, program)
                    .map_err(|e| anyhow::anyhow!("Failed to parse jq filter: {:?}", e))?;

                let filter = jaq_core::Compiler::default()
                    .with_funs(jaq_std::funs().chain(jaq_json::funs()))
                    .compile(modules)
                    .map_err(|e| anyhow::anyhow!("Failed to compile jq filter: {:?}", e))?;

                let input_val = Value::Array(all_resources);
                let input = Val::from(input_val);
                let inputs = RcIter::new(core::iter::empty());
                let out = filter.run((Ctx::new([], &inputs), input));

                let mut results = Vec::new();
                for res in out {
                    match res {
                        Ok(v) => results.push(Value::from(v)),
                        Err(e) => return Err(anyhow::anyhow!("JQ execution error: {:?}", e)),
                    }
                }
                Ok(results)
            } else {
                Ok(all_resources)
            }
        }
        #[cfg(not(feature = "aws-cloudcontrol"))]
        {
            Err(anyhow::anyhow!("Feature 'aws-cloudcontrol' is not enabled"))
        }
    } else {
        Err(anyhow::anyhow!(
            "Unsupported query protocol: {}",
            query_type
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_execute_query_mock() {
        let mut params = HashMap::new();
        params.insert("items".to_string(), "a,b,c".to_string());
        let res = execute_query("mock", &params, None, None).await.unwrap();
        assert_eq!(res.len(), 3);
        assert_eq!(res[0], serde_json::json!("a"));
    }

    #[tokio::test]
    async fn test_execute_query_api() {
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

        let res = execute_query("api", &params, None, None).await.unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0], serde_json::json!("x"));
    }

    #[tokio::test]
    async fn test_execute_query_api_with_jq_filter() {
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

        let res = execute_query("api", &params, None, None).await.unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0], serde_json::json!("Resource1"));
        assert_eq!(res[1], serde_json::json!("Resource2"));
    }

    #[tokio::test]
    async fn test_execute_query_api_post() {
        use wiremock::matchers::{body_json, header};
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

        let res = execute_query("api", &params, None, None).await.unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0], serde_json::json!("result1"));
        assert_eq!(res[1], serde_json::json!("result2"));
    }

    #[tokio::test]
    async fn test_execute_query_unsupported() {
        let res = execute_query("unknown", &HashMap::new(), None, None).await;
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("Unsupported query protocol"));
    }
}
