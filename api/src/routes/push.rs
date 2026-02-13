use axum::{
    extract::{Json, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::{domain::push as domain_push, AppState};
use crate::routes::auth::AuthUser;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/push/subscription", post(save_subscription).delete(remove_subscription))
        .route("/push/vapid-public-key", get(get_vapid_public_key))
}

#[derive(Deserialize)]
struct DeletePushSubscriptionRequest {
    endpoint: String,
}

async fn save_subscription(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    headers: HeaderMap,
    Json(subscription): Json<domain_push::PushSubscriptionData>,
) -> Result<StatusCode, StatusCode> {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok());

    if subscription.endpoint.trim().is_empty() || subscription.keys.p256dh.trim().is_empty() || subscription.keys.auth.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    domain_push::upsert_user_push_subscription(&state.db, user_id, &subscription, user_agent)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn remove_subscription(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Json(request): Json<DeletePushSubscriptionRequest>,
) -> StatusCode {
    let _ = domain_push::delete_user_push_subscription(&state.db, user_id, &request.endpoint).await;

    StatusCode::NO_CONTENT
}

#[derive(serde::Serialize)]
struct VapidPublicKeyResponse {
    vapid_public_key: String,
}

async fn get_vapid_public_key() -> Result<impl IntoResponse, StatusCode> {
    let vapid_public_key = std::env::var("VAPID_PUBLIC_KEY").map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    Ok(axum::Json(VapidPublicKeyResponse { vapid_public_key }))
}
