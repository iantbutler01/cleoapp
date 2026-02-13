//! Tweet model definitions

use chrono::{DateTime, Utc};
use sqlx::types::Json;

/// A tweet (standalone or part of a thread)
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Tweet {
    pub id: i64,
    pub text: String,
    pub copy_options: Json<Vec<String>>,
    pub video_clip: Option<serde_json::Value>,
    pub image_capture_ids: Vec<i64>,
    pub media_options: Json<Vec<serde_json::Value>>,
    pub rationale: String,
    pub created_at: DateTime<Utc>,
    pub thread_position: Option<i32>,
    pub reply_to_tweet_id: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
    pub tweet_id: Option<String>,
    pub publish_status: String,
    pub publish_attempts: i32,
    pub publish_error: Option<String>,
    #[allow(dead_code)]
    pub publish_error_at: Option<DateTime<Utc>>,
}

/// Tweet data needed for posting (includes media info)
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TweetForPosting {
    pub id: i64,
    pub text: String,
    #[allow(dead_code)]
    pub copy_options: Json<Vec<String>>,
    pub image_capture_ids: Vec<i64>,
    pub video_clip: Option<serde_json::Value>,
    #[allow(dead_code)]
    pub media_options: Json<Vec<serde_json::Value>>,
    #[allow(dead_code)]
    pub rationale: String,
}
