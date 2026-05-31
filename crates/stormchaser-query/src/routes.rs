use crate::hydration::run_hydration_loop;
use crate::models::HydrateSchemaRequest;
use axum::{extract::State, response::IntoResponse, Json};
use stormchaser_api::AppState;
use tokio_stream::StreamExt;

pub async fn hydrate_schema(
    // Require a valid token: the query service previously had NO authentication
    // (any caller could drive user-defined SQL/API/Git queries + HCL eval).
    // AuthClaims rejects with 401 before the body is read.
    _claims: stormchaser_api::AuthClaims,
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
