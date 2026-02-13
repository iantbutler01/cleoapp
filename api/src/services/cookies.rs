//! Cookie building utilities for session management
//!
//! Centralizes cookie formatting to avoid duplication and ensure consistency
//! across auth endpoints (login, refresh, logout).

use axum::http::{HeaderValue, StatusCode};

/// Cookie configuration constants
pub mod config {
    /// Access token cookie name
    pub const ACCESS_TOKEN_NAME: &str = "access_token";
    /// Refresh token cookie name
    pub const REFRESH_TOKEN_NAME: &str = "refresh_token";
    /// Access token max-age in seconds (10 minutes)
    pub const ACCESS_TOKEN_MAX_AGE_SECS: u32 = 600;
    /// Refresh token max-age in seconds (30 days)
    pub const REFRESH_TOKEN_MAX_AGE_SECS: u32 = 30 * 24 * 60 * 60;
    /// Path for access token cookie (all routes)
    pub const ACCESS_COOKIE_PATH: &str = "/";
    /// Path for refresh token cookie
    /// Must be "/" because the frontend proxies /api/auth/* and the browser sees that path,
    /// not the rewritten /auth/* path that the backend sees.
    pub const REFRESH_COOKIE_PATH: &str = "/";
}

fn is_dev() -> bool {
    std::env::var("ENV").as_deref() != Ok("prod")
}

fn cookie_same_site() -> &'static str {
    match std::env::var("COOKIE_SAMESITE")
        .unwrap_or_else(|_| "Lax".to_string())
        .to_lowercase()
        .as_str()
    {
        "none" => "None",
        "strict" => "Strict",
        "lax" => "Lax",
        _ => "Lax",
    }
}

/// Build an access token Set-Cookie header value
pub fn build_access_cookie(token: &str) -> Result<HeaderValue, StatusCode> {
    let same_site = cookie_same_site();
    let secure = if is_dev() { "" } else { " Secure;" };
    let cookie = format!(
        "{}={}; HttpOnly;{} SameSite={}; Path={}; Max-Age={}",
        config::ACCESS_TOKEN_NAME,
        token,
        secure,
        same_site,
        config::ACCESS_COOKIE_PATH,
        config::ACCESS_TOKEN_MAX_AGE_SECS
    );
    cookie.parse().map_err(|_| {
        eprintln!("Failed to parse access cookie header");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

/// Build a refresh token Set-Cookie header value
pub fn build_refresh_cookie(token: &str) -> Result<HeaderValue, StatusCode> {
    let same_site = cookie_same_site();
    let secure = if is_dev() { "" } else { " Secure;" };
    let cookie = format!(
        "{}={}; HttpOnly;{} SameSite={}; Path={}; Max-Age={}",
        config::REFRESH_TOKEN_NAME,
        token,
        secure,
        same_site,
        config::REFRESH_COOKIE_PATH,
        config::REFRESH_TOKEN_MAX_AGE_SECS
    );
    cookie.parse().map_err(|_| {
        eprintln!("Failed to parse refresh cookie header");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

/// Build a Set-Cookie header to clear the access token
pub fn build_clear_access_cookie() -> HeaderValue {
    format!(
        "{}=; HttpOnly; Secure; SameSite=Lax; Path={}; Max-Age=0",
        config::ACCESS_TOKEN_NAME,
        config::ACCESS_COOKIE_PATH
    )
    .parse()
    .expect("static cookie string should always parse")
}

/// Build a Set-Cookie header to clear the refresh token
pub fn build_clear_refresh_cookie() -> HeaderValue {
    format!(
        "{}=; HttpOnly; Secure; SameSite=Lax; Path={}; Max-Age=0",
        config::REFRESH_TOKEN_NAME,
        config::REFRESH_COOKIE_PATH
    )
    .parse()
    .expect("static cookie string should always parse")
}
