//! Content domain - unified content queries with DB-level pagination

use chrono::{DateTime, Utc};
use sqlx::types::Json;
use sqlx::{Executor, PgPool, Postgres};
use std::collections::HashMap;

use super::twitter::{Thread, ThreadWithTweets, Tweet};

/// Content item reference from UNION query
#[derive(Debug, Clone)]
struct ContentRef {
    id: i64,
    content_type: String, // "tweet" or "thread"
}

/// Internal struct for batch fetching tweets with their thread_id
#[derive(sqlx::FromRow)]
struct TweetWithThreadId {
    thread_id: i64,
    id: i64,
    text: String,
    copy_options: Json<Vec<String>>,
    video_clip: Option<serde_json::Value>,
    image_capture_ids: Vec<i64>,
    media_options: Json<Vec<serde_json::Value>>,
    rationale: String,
    created_at: DateTime<Utc>,
    publish_status: String,
    publish_attempts: i32,
    publish_error: Option<String>,
    publish_error_at: Option<DateTime<Utc>>,
    thread_position: Option<i32>,
    reply_to_tweet_id: Option<String>,
    posted_at: Option<DateTime<Utc>>,
    tweet_id: Option<String>,
}

/// Parsed content status filter enum for type-safe query building
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentStatusFilter {
    Pending,
    Posted,
    All,
}

impl ContentStatusFilter {
    pub fn from_str(s: Option<&str>) -> Self {
        match s {
            Some("pending") => ContentStatusFilter::Pending,
            Some("posted") => ContentStatusFilter::Posted,
            _ => ContentStatusFilter::All,
        }
    }

    /// Tweet WHERE clause fragment (for tweet_collateral table)
    fn tweet_where(&self) -> &'static str {
        match self {
            ContentStatusFilter::Pending => "AND posted_at IS NULL AND dismissed_at IS NULL",
            ContentStatusFilter::Posted => "AND posted_at IS NOT NULL",
            ContentStatusFilter::All => "AND dismissed_at IS NULL",
        }
    }

    /// Thread WHERE clause fragment (for tweet_threads table)
    fn thread_where(&self) -> &'static str {
        match self {
            ContentStatusFilter::Pending => "AND status IN ('draft', 'posting', 'partial_failed')",
            ContentStatusFilter::Posted => "AND status = 'posted'",
            ContentStatusFilter::All => "",
        }
    }
}

/// Count total content items (tweets + threads) with status filter
pub async fn count_content<'e, E>(
    executor: E,
    user_id: i64,
    status_filter: Option<&str>,
) -> Result<i64, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let filter = ContentStatusFilter::from_str(status_filter);

    let query = format!(
        r#"
        SELECT (
            SELECT COUNT(*) FROM tweet_collateral
            WHERE user_id = $1 AND thread_id IS NULL {}
        ) + (
            SELECT COUNT(*) FROM tweet_threads
            WHERE user_id = $1 {}
        )
        "#,
        filter.tweet_where(),
        filter.thread_where()
    );

    let (count,): (i64,) = sqlx::query_as(&query)
        .bind(user_id)
        .fetch_one(executor)
        .await?;
    Ok(count)
}

/// List content with DB-level pagination using UNION query
/// Returns (items, total_count) where items are properly paginated by the database
pub async fn list_content_paginated(
    db: &PgPool,
    user_id: i64,
    status_filter: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<ContentItem>, i64), sqlx::Error> {
    let filter = ContentStatusFilter::from_str(status_filter);

    // Use UNION to get content references ordered by created_at with LIMIT/OFFSET
    // This is the key optimization - we paginate in the database, not in memory
    let query = format!(
        r#"
        SELECT id, 'tweet' as content_type, created_at
        FROM tweet_collateral
        WHERE user_id = $1 AND thread_id IS NULL {}
        UNION ALL
        SELECT id, 'thread' as content_type, created_at
        FROM tweet_threads
        WHERE user_id = $1 {}
        ORDER BY created_at DESC
        LIMIT $2 OFFSET $3
        "#,
        filter.tweet_where(),
        filter.thread_where()
    );

    let refs: Vec<(i64, String, DateTime<Utc>)> = sqlx::query_as(&query)
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(db)
        .await?;

    let content_refs: Vec<ContentRef> = refs
        .into_iter()
        .map(|(id, content_type, _created_at)| ContentRef { id, content_type })
        .collect();

    // Separate tweet and thread IDs
    let tweet_ids: Vec<i64> = content_refs
        .iter()
        .filter(|r| r.content_type == "tweet")
        .map(|r| r.id)
        .collect();

    let thread_ids: Vec<i64> = content_refs
        .iter()
        .filter(|r| r.content_type == "thread")
        .map(|r| r.id)
        .collect();

    // Batch fetch tweets
    let tweets: HashMap<i64, Tweet> = if !tweet_ids.is_empty() {
        let rows: Vec<Tweet> = sqlx::query_as(
            r#"
            SELECT id, text,
                   COALESCE(copy_options, '[]'::jsonb) as copy_options,
                   video_clip, image_capture_ids,
                   COALESCE(media_options, '[]'::jsonb) as media_options,
                   rationale, created_at,
                   publish_status, publish_attempts, publish_error, publish_error_at,
                   thread_position, reply_to_tweet_id, posted_at, tweet_id
            FROM tweet_collateral
            WHERE id = ANY($1) AND user_id = $2
            "#,
        )
        .bind(&tweet_ids)
        .bind(user_id)
        .fetch_all(db)
        .await?;

        rows.into_iter().map(|t| (t.id, t)).collect()
    } else {
        HashMap::new()
    };

    // Batch fetch threads with their tweets
    let threads: HashMap<i64, ThreadWithTweets> = if !thread_ids.is_empty() {
        // First get the thread metadata
        let thread_structs: Vec<Thread> = sqlx::query_as(
            r#"
            SELECT id, user_id, title,
                   COALESCE(copy_options, '[]'::jsonb) as copy_options,
                   status, created_at, posted_at, first_tweet_id
            FROM tweet_threads
            WHERE id = ANY($1) AND user_id = $2
            "#,
        )
        .bind(&thread_ids)
        .bind(user_id)
        .fetch_all(db)
        .await?;

        // Then batch fetch all tweets for all threads
        let all_thread_tweets: Vec<TweetWithThreadId> = sqlx::query_as(
            r#"
            SELECT thread_id, id, text,
                   COALESCE(copy_options, '[]'::jsonb) as copy_options,
                   video_clip, image_capture_ids,
                   COALESCE(media_options, '[]'::jsonb) as media_options,
                   rationale, created_at,
                   publish_status, publish_attempts, publish_error, publish_error_at,
                   thread_position, reply_to_tweet_id, posted_at, tweet_id
            FROM tweet_collateral
            WHERE thread_id = ANY($1) AND user_id = $2
            ORDER BY thread_id, thread_position ASC
            "#,
        )
        .bind(&thread_ids)
        .bind(user_id)
        .fetch_all(db)
        .await?;

        // Group tweets by thread_id
        let mut tweets_by_thread: HashMap<i64, Vec<Tweet>> = HashMap::new();
        for tweet_row in all_thread_tweets {
            let tweet = Tweet {
                id: tweet_row.id,
                text: tweet_row.text,
                copy_options: tweet_row.copy_options,
                video_clip: tweet_row.video_clip,
                image_capture_ids: tweet_row.image_capture_ids,
                media_options: tweet_row.media_options,
                rationale: tweet_row.rationale,
                created_at: tweet_row.created_at,
                publish_status: tweet_row.publish_status,
                publish_attempts: tweet_row.publish_attempts,
                publish_error: tweet_row.publish_error,
                publish_error_at: tweet_row.publish_error_at,
                thread_position: tweet_row.thread_position,
                reply_to_tweet_id: tweet_row.reply_to_tweet_id,
                posted_at: tweet_row.posted_at,
                tweet_id: tweet_row.tweet_id,
            };
            tweets_by_thread
                .entry(tweet_row.thread_id)
                .or_default()
                .push(tweet);
        }

        // Build ThreadWithTweets
        thread_structs
            .into_iter()
            .map(|thread| {
                let tweets = tweets_by_thread.remove(&thread.id).unwrap_or_default();
                (thread.id, ThreadWithTweets { thread, tweets })
            })
            .collect()
    } else {
        HashMap::new()
    };

    // Build final items in the correct order (from content_refs)
    let items: Vec<ContentItem> = content_refs
        .into_iter()
        .filter_map(|r| {
            if r.content_type == "tweet" {
                tweets.get(&r.id).cloned().map(ContentItem::Tweet)
            } else {
                threads.get(&r.id).cloned().map(ContentItem::Thread)
            }
        })
        .collect();

    // Get total count
    let total = count_content(db, user_id, status_filter).await?;

    Ok((items, total))
}

/// Discriminated union for content items (matches route ContentItem)
#[derive(Debug, Clone)]
pub enum ContentItem {
    Tweet(Tweet),
    Thread(ThreadWithTweets),
}
