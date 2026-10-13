use crate::hydration::run_hydration_loop;
use crate::models::HydrateSchemaRequest;
use axum::{extract::State, response::IntoResponse, Json};
use stormchaser_api::AppState;
use tokio_stream::StreamExt;

pub async fn hydrate_schema(
    // Require a valid token: the query service previously had NO authentication
    // (any caller could drive user-defined SQL/API/Git queries + HCL eval).
    // AuthClaims rejects with 401 before the body is read.
    stormchaser_api::AuthClaims(claims): stormchaser_api::AuthClaims,
    State(state): State<AppState>,
    Json(payload): Json<HydrateSchemaRequest>,
) -> impl IntoResponse {
    let queries: Vec<stormchaser_model::dsl::Query> = payload
        .queries
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();

    // Connection use is authorized against the caller, named the same way a run records
    // its initiating user.
    let user = claims.email.clone().unwrap_or(claims.sub.clone());

    let (tx, rx) = tokio::sync::mpsc::channel(100);

    let state_clone = state.clone();
    tokio::spawn(async move {
        run_hydration_loop(
            payload.schema,
            payload.inputs,
            queries,
            Some(state_clone),
            user,
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
