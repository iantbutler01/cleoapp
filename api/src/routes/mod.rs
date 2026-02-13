pub mod agent;
pub mod auth;
pub mod captures;
pub mod content;
pub mod media_studio;
pub mod push;
pub mod nudges;
pub mod twitter_oauth;
pub mod user;

use axum::Router;
use std::sync::Arc;

use crate::AppState;

/// Build all routes for the API
pub fn build_routes() -> Router<Arc<AppState>> {
    Router::new()
        .merge(agent::routes())
        .merge(auth::routes())
        .merge(captures::routes())
        .merge(content::routes())
        .merge(media_studio::routes())
        .merge(push::routes())
        .merge(nudges::routes())
        .merge(twitter_oauth::routes())
        .merge(user::routes())
}
