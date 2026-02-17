mod dto;
pub mod media;
pub mod threads;
pub mod tweets;

// Re-export DTOs for parent content/mod.rs
pub use dto::{ThreadWithTweetsResponse, TweetResponse};

use axum::Router;
use std::sync::Arc;

use crate::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .merge(tweets::routes())
        .merge(threads::routes())
}
