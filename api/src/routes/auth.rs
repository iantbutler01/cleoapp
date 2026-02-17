//! Authentication and session management endpoints

use axum::{
    Json, Router,
    extract::{FromRequestParts, State},
    http::{StatusCode, header::SET_COOKIE, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use serde::Serialize;
use std::sync::Arc;
use tower_governor::{
    GovernorLayer, governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor,
};

use crate::AppState;
use crate::domain::users;
use crate::services::{cookies, session, twitter};

pub fn routes() -> Router<Arc<AppState>> {
    // Rate limit: 10 requests per minute for auth endpoints to prevent brute force
    let rate_limit_config = GovernorConfigBuilder::default()
        .per_second(6) // 6 tokens per second
        .burst_size(10) // Allow burst of 10 requests, then 1 per 10 seconds
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .expect("Failed to build rate limit config");

    let rate_limit_layer = GovernorLayer {
        config: rate_limit_config.into(),
    };

    Router::new()
        .route("/me/token", get(get_api_token).post(generate_api_token))
        .route("/auth/refresh", post(refresh_session))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(get_me))
        .layer(rate_limit_layer)
}

// ============================================================================
// Auth Extractor - validates JWT cookie and extracts user_id
// ============================================================================

/// Extractor that validates the access_token cookie and returns the user_id
pub struct AuthUser(pub i64);

impl FromRequestParts<Arc<AppState>> for AuthUser {
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        // Extract cookies from headers
        let jar = CookieJar::from_request_parts(parts, state)
            .await
            .map_err(|e| {
                eprintln!("Cookie extraction error: {:?}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        // Get access_token cookie
        let access_token = jar
            .get("access_token")
            .map(|c| c.value())
            .ok_or(StatusCode::UNAUTHORIZED)?;

        // Validate JWT
        let user_id =
            session::validate_access_token(access_token, &state.jwt_secret).map_err(|e| {
                eprintln!("JWT validation failed: {:?}", e);
                StatusCode::UNAUTHORIZED
            })?;

        Ok(AuthUser(user_id))
    }
}

// ============================================================================
// Session endpoints
// ============================================================================

/// POST /auth/refresh - Refresh the access token using the refresh token cookie
/// Implements refresh token rotation: old token is invalidated, new one is issued
async fn refresh_session(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Response, StatusCode> {
    // Get refresh_token cookie
    let old_refresh_token = jar
        .get("refresh_token")
        .map(|c| c.value().to_string())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Rotate refresh token: validate old, delete it, create new one
    // This is atomic - if two requests try to use the same token, only one succeeds
    // (silent - invalid/expired tokens are expected for expired sessions)
    let (user_id, new_refresh_token) = session::rotate_refresh_token(&old_refresh_token, &state.db)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Generate new access token
    let access_token = session::create_access_token(user_id, &state.jwt_secret).map_err(|e| {
        eprintln!("Failed to create access token: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Build response with cookies (204 No Content - only sets cookies)
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .append(SET_COOKIE, cookies::build_access_cookie(&access_token)?);
    response.headers_mut().append(
        SET_COOKIE,
        cookies::build_refresh_cookie(&new_refresh_token)?,
    );

    Ok(response)
}

/// POST /auth/logout - Clear session and revoke refresh token
async fn logout(State(state): State<Arc<AppState>>, jar: CookieJar) -> Response {
    // Try to revoke the refresh token if it exists
    if let Some(refresh_token) = jar.get("refresh_token") {
        if let Err(e) = session::revoke_refresh_token(refresh_token.value(), &state.db).await {
            // Log but don't fail logout - user is still logged out client-side
            eprintln!("Failed to revoke refresh token during logout: {}", e);
        }
    }

    // Clear cookies (204 No Content - session ended)
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .append(SET_COOKIE, cookies::build_clear_access_cookie());
    response
        .headers_mut()
        .append(SET_COOKIE, cookies::build_clear_refresh_cookie());

    response
}

#[derive(Serialize)]
struct MeResponse {
    id: i64,
    username: String,
}

/// GET /auth/me - Get current user info (validates session)
async fn get_me(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
) -> Result<Json<MeResponse>, StatusCode> {
    // Get user from database
    let user = users::get_user_by_id(&state.db, user_id)
        .await
        .map_err(|e| {
            eprintln!("Get user by ID error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Return 401 if user not found - a valid JWT for a deleted user is still unauthorized
    let user = user.ok_or(StatusCode::UNAUTHORIZED)?;

    Ok(Json(MeResponse {
        id: user_id,
        username: user.twitter_username,
    }))
}

// ============================================================================
// API Token endpoints (for daemon auth)
// ============================================================================

#[derive(Serialize)]
struct ApiTokenResponse {
    api_token: String,
}

/// POST /me/token - Generate a new API token for the daemon
async fn generate_api_token(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
) -> Result<(StatusCode, Json<ApiTokenResponse>), StatusCode> {
    let token = twitter::generate_api_token();
    twitter::set_user_api_token(&state.db, user_id, &token)
        .await
        .map_err(|e| {
            eprintln!("Set user API token error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok((
        StatusCode::CREATED,
        Json(ApiTokenResponse { api_token: token }),
    ))
}

/// GET /me/token - Get current API token (if exists)
async fn get_api_token(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
) -> Result<Json<Option<String>>, StatusCode> {
    let token = twitter::get_user_api_token(&state.db, user_id)
        .await
        .map_err(|e| {
            eprintln!("Get user API token error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(token))
}
