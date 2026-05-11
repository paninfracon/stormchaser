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
    async fn test_execute_query_unsupported() {
        let res = execute_query("unknown", &HashMap::new(), None, None).await;
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("Unsupported query protocol"));
    }
}
