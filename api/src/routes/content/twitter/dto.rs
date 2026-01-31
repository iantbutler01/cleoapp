//! API response DTOs for Twitter content

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::twitter::{Thread, ThreadStatus, ThreadWithTweets, Tweet};

/// Tweet API response
#[derive(Debug, Clone, Serialize)]
pub struct TweetResponse {
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

impl From<Tweet> for TweetResponse {
    fn from(t: Tweet) -> Self {
        Self {
            id: t.id,
            text: t.text,
            video_clip: t.video_clip,
            image_capture_ids: t.image_capture_ids,
            rationale: t.rationale,
            created_at: t.created_at,
            thread_position: t.thread_position,
            reply_to_tweet_id: t.reply_to_tweet_id,
            posted_at: t.posted_at,
            tweet_id: t.tweet_id,
        }
    }
}

/// Thread API response
#[derive(Debug, Clone, Serialize)]
pub struct ThreadResponse {
    pub id: i64,
    pub title: Option<String>,
    pub status: ThreadStatus,
    pub created_at: DateTime<Utc>,
    pub posted_at: Option<DateTime<Utc>>,
    pub first_tweet_id: Option<String>,
}

impl From<Thread> for ThreadResponse {
    fn from(t: Thread) -> Self {
        Self {
            id: t.id,
            title: t.title,
            status: t.status,
            created_at: t.created_at,
            posted_at: t.posted_at,
            first_tweet_id: t.first_tweet_id,
        }
    }
}

/// Thread with tweets API response
#[derive(Debug, Clone, Serialize)]
pub struct ThreadWithTweetsResponse {
    pub thread: ThreadResponse,
    pub tweets: Vec<TweetResponse>,
}

impl From<ThreadWithTweets> for ThreadWithTweetsResponse {
    fn from(t: ThreadWithTweets) -> Self {
        Self {
            thread: t.thread.into(),
            tweets: t.tweets.into_iter().map(Into::into).collect(),
        }
    }
}
