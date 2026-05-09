use crate::ApiDoc;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp_openapi::Server;
use std::io::Error;
use tracing::error;
use url::Url;
use utoipa::OpenApi;

pub fn mcp_service(
    api_base_url: &str,
) -> Option<StreamableHttpService<Server, LocalSessionManager>> {
    let openapi_val = match serde_json::to_value(ApiDoc::openapi()) {
        Ok(value) => value,
        Err(error) => {
            error!(?error, "Failed to serialize OpenAPI spec for MCP service");
            return None;
        }
    };
    let base_url = match Url::parse(api_base_url) {
        Ok(value) => value,
        Err(error) => {
            error!(?error, "Invalid API_BASE_URL; MCP service disabled");
            return None;
        }
    };

    Some(StreamableHttpService::new(
        move || {
            let mut server = Server::new(
                openapi_val.clone(),
                base_url.clone(),
                None,
                None,
                false,
                false,
            );
            server.load_openapi_spec().map_err(|error| {
                error!(?error, "Failed to load MCP OpenAPI spec");
                Error::other(error.to_string())
            })?;
            Ok(server)
        },
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    ))
}
