use crate::ApiDoc;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp_openapi::Server;
use url::Url;
use utoipa::OpenApi;

pub fn mcp_service(api_base_url: &str) -> StreamableHttpService<Server, LocalSessionManager> {
    let openapi_val =
        serde_json::to_value(ApiDoc::openapi()).expect("Failed to serialize OpenAPI spec");
    let base_url = Url::parse(api_base_url).expect("Failed to parse API_BASE_URL");

    StreamableHttpService::new(
        move || {
            let mut server = Server::new(
                openapi_val.clone(),
                base_url.clone(),
                None,
                None,
                false,
                false,
            );
            server.load_openapi_spec().unwrap();
            Ok(server)
        },
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    )
}
