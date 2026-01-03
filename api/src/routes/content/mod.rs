//! Content endpoints - unified view of content items by platform

pub mod twitter;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::domain::content;
use crate::AppState;
use super::auth::AuthUser;
use twitter::{TweetResponse, ThreadWithTweetsResponse};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/content", get(list_content))
        .merge(twitter::routes())
}

/// Discriminated union for content items
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentItem {
    Tweet(TweetResponse),
    Thread(ThreadWithTweetsResponse),
}

#[derive(Debug, Deserialize)]
pub struct ContentQuery {
    pub platform: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    pub status: Option<String>,
}

fn default_limit() -> i64 {
    500
}

#[derive(Debug, Serialize)]
pub struct ContentResponse {
    pub items: Vec<ContentItem>,
    pub total: i64,
    pub has_more: bool,
}

/// GET /content?platform=twitter - List all content for a platform
/// Uses DB-level UNION query for proper pagination (no in-memory sorting)
async fn list_content(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Query(query): Query<ContentQuery>,
) -> Result<Json<ContentResponse>, StatusCode> {
    match query.platform.as_str() {
        "twitter" => {
            let status_filter = query.status.as_deref();

            // Use domain function with DB-level pagination via UNION query
            let (domain_items, total) = content::list_content_paginated(
                &state.db,
                user_id,
                status_filter,
                query.limit,
                query.offset,
            )
            .await
            .map_err(|e| {
                eprintln!("Failed to fetch content: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

            // Convert domain ContentItem to route ContentItem (with DTOs)
            let items: Vec<ContentItem> = domain_items
                .into_iter()
                .map(|item| match item {
                    content::ContentItem::Tweet(t) => ContentItem::Tweet(TweetResponse::from(t)),
                    content::ContentItem::Thread(t) => ContentItem::Thread(ThreadWithTweetsResponse::from(t)),
                })
                .collect();

            let has_more = (query.offset + query.limit) < total;

            Ok(Json(ContentResponse {
                items,
                total,
                has_more,
            }))
        }
        _ => Err(StatusCode::BAD_REQUEST),
    }
}
