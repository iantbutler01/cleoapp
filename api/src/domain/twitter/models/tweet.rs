//! Tweet model definitions

use chrono::{DateTime, Utc};

/// A tweet (standalone or part of a thread)
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Tweet {
    pub id: i64,
    pub text: String,
    pub video_clip: Option<serde_json::Value>,
    pub image_capture_ids: Vec<i64>,
    pub rationale: String,
    pub created_at: DateTime<Utc>,
    pub thread_position: Option<i32>,
    pub reply_to_tweet_id: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
    pub tweet_id: Option<String>,
}

/// Tweet data needed for posting (includes media info)
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TweetForPosting {
    pub id: i64,
    pub text: String,
    pub image_capture_ids: Vec<i64>,
    pub video_clip: Option<serde_json::Value>,
}
