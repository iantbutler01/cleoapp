pub mod auth;
pub mod captures;
pub mod content;
pub mod twitter_oauth;
pub mod user;

use axum::Router;
use std::sync::Arc;

use crate::AppState;

/// Build all routes for the API
pub fn build_routes() -> Router<Arc<AppState>> {
    Router::new()
        .merge(auth::routes())
        .merge(captures::routes())
        .merge(content::routes())
        .merge(twitter_oauth::routes())
        .merge(user::routes())
}
