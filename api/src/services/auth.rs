//! Authentication helpers for token refresh

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use sqlx::PgPool;

use super::twitter::{self, TwitterClient, UserTokens};

/// Ensures the access token is valid, refreshing if expired.
/// Returns the valid access token or a StatusCode error.
pub async fn ensure_valid_access_token(
    db: &PgPool,
    twitter_client: &TwitterClient,
    user_id: i64,
    tokens: UserTokens,
) -> Result<String, StatusCode> {
    // Token still valid
    if tokens.token_expires_at >= Utc::now() {
        return Ok(tokens.access_token);
    }

    // Need to refresh
    let refresh_token = tokens.refresh_token.ok_or_else(|| {
        eprintln!("Token expired and no refresh token for user {}", user_id);
        StatusCode::UNAUTHORIZED
    })?;

    let new_tokens = twitter_client
        .refresh_token(&refresh_token)
        .await
        .map_err(|e| {
            eprintln!("Token refresh error: {}", e);
            StatusCode::UNAUTHORIZED
        })?;

    let expires_at = Utc::now() + Duration::seconds(new_tokens.expires_in);
    twitter::update_user_tokens(
        db,
        user_id,
        &new_tokens.access_token,
        new_tokens.refresh_token.as_deref(),
        expires_at,
    )
    .await
    .map_err(|e| {
        eprintln!("Update user tokens error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(new_tokens.access_token)
}

/// Variant that returns String errors (for WebSocket handlers).
pub async fn ensure_valid_access_token_str(
    db: &PgPool,
    twitter_client: &TwitterClient,
    user_id: i64,
    tokens: UserTokens,
) -> Result<String, String> {
    // Token still valid
    if tokens.token_expires_at >= Utc::now() {
        return Ok(tokens.access_token);
    }

    // Need to refresh
    let refresh_token = tokens
        .refresh_token
        .ok_or("Token expired and no refresh token")?;

    let new_tokens = twitter_client
        .refresh_token(&refresh_token)
        .await
        .map_err(|e| format!("Token refresh failed: {}", e))?;

    let expires_at = Utc::now() + Duration::seconds(new_tokens.expires_in);
    twitter::update_user_tokens(
        db,
        user_id,
        &new_tokens.access_token,
        new_tokens.refresh_token.as_deref(),
        expires_at,
    )
    .await
    .map_err(|e| format!("Failed to update tokens: {}", e))?;

    Ok(new_tokens.access_token)
}
