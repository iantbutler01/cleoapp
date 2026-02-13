//! Twitter OAuth endpoints (/auth/twitter/*)

use axum::{
    Json, Router,
    extract::State,
    http::{header::SET_COOKIE, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_governor::{
    GovernorLayer,
    governor::GovernorConfigBuilder,
    key_extractor::SmartIpKeyExtractor,
};

use crate::services::{cookies, session, twitter};
use crate::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    // Rate limit: Stricter for OAuth - 5 requests per minute to prevent abuse
    let rate_limit_config = GovernorConfigBuilder::default()
        .per_second(12)  // Refill rate
        .burst_size(5)   // Allow burst of 5 requests, then 1 per 12 seconds
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .expect("Failed to build rate limit config");

    let rate_limit_layer = GovernorLayer {
        config: rate_limit_config.into(),
    };

    Router::new()
        .route("/auth/twitter", get(auth_twitter))
        .route("/auth/twitter/token", post(auth_twitter_token))
        .layer(rate_limit_layer)
}

#[derive(Serialize)]
struct AuthUrlResponse {
    url: String,
}

/// GET /auth/twitter - Start OAuth flow, returns URL to redirect user to
async fn auth_twitter(State(state): State<Arc<AppState>>) -> Json<AuthUrlResponse> {
    let auth_request = state.twitter.get_authorize_url(&[
        "tweet.read",
        "tweet.write",
        "users.read",
        "media.write",
        "offline.access",
    ]);

    // Store state and code_verifier for callback
    if let Err(e) = twitter::save_oauth_state(&state.db, &auth_request.state, &auth_request.code_verifier).await {
        eprintln!("Failed to save OAuth state: {}", e);
        // Return the URL anyway - login will fail at token exchange if state isn't found
        // This is better than blocking the user completely
    }

    Json(AuthUrlResponse {
        url: auth_request.url,
    })
}

#[derive(Deserialize)]
struct TokenRequest {
    code: String,
    state: String,
}

#[derive(Serialize)]
struct LoginResponse {
    username: String,
}

/// POST /auth/twitter/token - Exchange OAuth code for session
/// Sets httpOnly cookies for access_token (JWT) and refresh_token
async fn auth_twitter_token(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TokenRequest>,
) -> Result<Response, StatusCode> {
    // Retrieve and validate state
    let code_verifier = twitter::get_oauth_state(&state.db, &req.state)
        .await
        .map_err(|e| {
            eprintln!("Get OAuth state error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Exchange code for tokens
    let token_response = state
        .twitter
        .exchange_code(&req.code, &code_verifier)
        .await
        .map_err(|e| {
            eprintln!("Token exchange error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Get user info
    let twitter_user = state
        .twitter
        .get_me(&token_response.access_token)
        .await
        .map_err(|e| {
            eprintln!("Get me error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Check if user is allowed to log in (if allowlist is configured)
    if let Some(ref allowed) = state.allowed_users {
        if !allowed.contains(&twitter_user.username.to_lowercase()) {
            eprintln!(
                "Login denied: @{} not in ALLOWED_USERS",
                twitter_user.username
            );
            return Err(StatusCode::FORBIDDEN);
        }
    }

    // Calculate token expiry
    let expires_at = Utc::now() + Duration::seconds(token_response.expires_in);

    // Upsert user
    let user_id = twitter::upsert_user(
        &state.db,
        &twitter_user.id,
        &twitter_user.username,
        Some(&twitter_user.name),
        &token_response.access_token,
        token_response.refresh_token.as_deref(),
        expires_at,
    )
    .await
    .map_err(|e| {
        eprintln!("Upsert user error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Create session tokens
    let access_token = session::create_access_token(user_id, &state.jwt_secret)
        .map_err(|e| {
            eprintln!("Failed to create access token: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let refresh_token = session::create_refresh_token(user_id, &state.db)
        .await
        .map_err(|e| {
            eprintln!("Failed to create refresh token: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Build response with cookies
    let body = Json(LoginResponse {
        username: twitter_user.username,
    });

    let mut response = body.into_response();
    response.headers_mut().append(SET_COOKIE, cookies::build_access_cookie(&access_token)?);
    response.headers_mut().append(SET_COOKIE, cookies::build_refresh_cookie(&refresh_token)?);

    Ok(response)
}
