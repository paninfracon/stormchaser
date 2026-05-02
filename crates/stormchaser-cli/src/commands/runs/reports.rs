use crate::utils::{handle_response, require_token};
use anyhow::Result;
use uuid::Uuid;

pub async fn list_reports(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: Uuid,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/reports", url, id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

pub async fn get_report(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: Uuid,
    report_id: Uuid,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/reports/{}", url, id, report_id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}
