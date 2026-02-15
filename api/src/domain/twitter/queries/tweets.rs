//! Tweet domain - DB queries for tweets
//!
//! All functions use the generic Executor pattern, allowing them to work with
//! both `&PgPool` (for standalone queries) and `&mut PgConnection` (for transactions).

use sqlx::{Executor, Postgres};

use super::super::models::{Tweet, TweetForPosting};

/// Parsed status filter enum for type-safe query building
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusFilter {
    Pending,
    Posted,
    Dismissed,
    All,
}

impl StatusFilter {
    pub fn from_str(s: Option<&str>) -> Self {
        match s {
            Some("pending") => StatusFilter::Pending,
            Some("posted") => StatusFilter::Posted,
            Some("dismissed") => StatusFilter::Dismissed,
            _ => StatusFilter::All,
        }
    }

    /// Returns SQL WHERE clause fragment for filtering by post status
    fn where_clause(&self) -> &'static str {
        match self {
            StatusFilter::Pending => "AND posted_at IS NULL AND dismissed_at IS NULL",
            StatusFilter::Posted => "AND posted_at IS NOT NULL",
            StatusFilter::Dismissed => "AND dismissed_at IS NOT NULL",
            StatusFilter::All => "AND dismissed_at IS NULL",
        }
    }
}

/// List pending standalone tweets (not in a thread) for a user (no pagination)
#[allow(dead_code)]
pub async fn list_pending_tweets<'e, E>(
    executor: E,
    user_id: i64,
) -> Result<Vec<Tweet>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_as(
        r#"
        SELECT id, text,
               COALESCE(copy_options, '[]'::jsonb) as copy_options,
               video_clip, image_capture_ids,
               COALESCE(media_options, '[]'::jsonb) as media_options,
               rationale, created_at,
               publish_status, publish_attempts, publish_error, publish_error_at,
               thread_position, reply_to_tweet_id, posted_at, tweet_id
        FROM tweet_collateral
        WHERE user_id = $1 AND posted_at IS NULL AND dismissed_at IS NULL AND thread_id IS NULL
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(executor)
    .await
}

/// Count standalone tweets for pagination
pub async fn count_standalone_tweets<'e, E>(
    executor: E,
    user_id: i64,
    status_filter: Option<&str>,
) -> Result<i64, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let filter = StatusFilter::from_str(status_filter);
    let query = format!(
        "SELECT COUNT(*) FROM tweet_collateral WHERE user_id = $1 AND thread_id IS NULL {}",
        filter.where_clause()
    );

    let (count,): (i64,) = sqlx::query_as(&query)
        .bind(user_id)
        .fetch_one(executor)
        .await?;

    Ok(count)
}

/// List pending standalone tweets with pagination
pub async fn list_pending_tweets_paginated<'e, E>(
    executor: E,
    user_id: i64,
    status_filter: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Tweet>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let filter = StatusFilter::from_str(status_filter);
    let query = format!(
        r#"SELECT id, text,
                  COALESCE(copy_options, '[]'::jsonb) as copy_options,
                  video_clip, image_capture_ids,
                  COALESCE(media_options, '[]'::jsonb) as media_options,
                  rationale, created_at,
                  publish_status, publish_attempts, publish_error, publish_error_at,
                  thread_position, reply_to_tweet_id, posted_at, tweet_id
           FROM tweet_collateral
           WHERE user_id = $1 AND thread_id IS NULL {}
           ORDER BY created_at DESC
           LIMIT $2 OFFSET $3"#,
        filter.where_clause()
    );

    sqlx::query_as(&query)
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(executor)
        .await
}

/// List standalone tweets with pagination (not in a thread) for a user
#[allow(dead_code)]
pub async fn list_standalone_tweets_paginated<'e, E>(
    executor: E,
    user_id: i64,
    status_filter: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Tweet>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let filter = StatusFilter::from_str(status_filter);
    let query = format!(
        r#"SELECT id, text,
                  COALESCE(copy_options, '[]'::jsonb) as copy_options,
                  video_clip, image_capture_ids,
                  COALESCE(media_options, '[]'::jsonb) as media_options,
                  rationale, created_at,
                  publish_status, publish_attempts, publish_error, publish_error_at,
                  thread_position, reply_to_tweet_id, posted_at, tweet_id
           FROM tweet_collateral
           WHERE user_id = $1 AND thread_id IS NULL {}
           ORDER BY created_at DESC
           LIMIT $2 OFFSET $3"#,
        filter.where_clause()
    );

    sqlx::query_as(&query)
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(executor)
        .await
}

/// List all standalone tweets (not in a thread) for a user
#[allow(dead_code)]
pub async fn list_standalone_tweets<'e, E>(
    executor: E,
    user_id: i64,
    status_filter: Option<&str>,
) -> Result<Vec<Tweet>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let filter = StatusFilter::from_str(status_filter);
    let query = format!(
        r#"SELECT id, text,
                  COALESCE(copy_options, '[]'::jsonb) as copy_options,
                  video_clip, image_capture_ids,
                  COALESCE(media_options, '[]'::jsonb) as media_options,
                  rationale, created_at,
                  publish_status, publish_attempts, publish_error, publish_error_at,
                  thread_position, reply_to_tweet_id, posted_at, tweet_id
           FROM tweet_collateral
           WHERE user_id = $1 AND thread_id IS NULL {}
           ORDER BY created_at DESC"#,
        filter.where_clause()
    );

    sqlx::query_as(&query)
        .bind(user_id)
        .fetch_all(executor)
        .await
}

/// Get a tweet by ID (with media info for posting)
pub async fn get_tweet_for_posting<'e, E>(
    executor: E,
    tweet_id: i64,
    user_id: i64,
) -> Result<Option<TweetForPosting>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_as(
        r#"
        SELECT id, text,
               COALESCE(copy_options, '[]'::jsonb) as copy_options,
               image_capture_ids, video_clip,
               COALESCE(media_options, '[]'::jsonb) as media_options,
               rationale
        FROM tweet_collateral
        WHERE id = $1 AND user_id = $2 AND posted_at IS NULL AND dismissed_at IS NULL
        "#,
    )
    .bind(tweet_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await
}

/// Mark a tweet as posted (atomic - only succeeds if not already posted)
/// Returns true if the update was applied, false if already posted
pub async fn mark_tweet_posted<'e, E>(
    executor: E,
    tweet_id: i64,
    twitter_id: &str,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        r#"
        UPDATE tweet_collateral
        SET posted_at = NOW(),
            tweet_id = $1,
            publish_status = 'posted',
            publish_error = NULL,
            publish_error_at = NULL
        WHERE id = $2 AND posted_at IS NULL
        "#,
    )
    .bind(twitter_id)
    .bind(tweet_id)
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Set tweet to posting status
#[allow(dead_code)]
pub async fn set_tweet_posting<'e, E>(
    executor: E,
    tweet_id: i64,
    user_id: i64,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        r#"
        UPDATE tweet_collateral
        SET publish_status = 'posting',
            publish_attempts = COALESCE(publish_attempts, 0) + 1,
            publish_error = NULL,
            publish_error_at = NULL
        WHERE id = $1
            AND user_id = $2
            AND posted_at IS NULL
            AND publish_status IN ('pending', 'failed')
        "#,
    )
    .bind(tweet_id)
    .bind(user_id)
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Mark tweet publish as failed
pub async fn mark_tweet_publish_failed<'e, E>(
    executor: E,
    tweet_id: i64,
    user_id: i64,
    error: &str,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        r#"
        UPDATE tweet_collateral
        SET publish_status = 'failed',
            publish_error = $3,
            publish_error_at = NOW()
        WHERE id = $1 AND user_id = $2 AND posted_at IS NULL
        "#,
    )
    .bind(tweet_id)
    .bind(user_id)
    .bind(error)
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Soft-delete a pending tweet (sets dismissed_at instead of removing the row)
pub async fn delete_tweet<'e, E>(
    executor: E,
    tweet_id: i64,
    user_id: i64,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        r#"
        UPDATE tweet_collateral
        SET dismissed_at = NOW(),
            publish_status = 'dismissed'
        WHERE id = $1 AND user_id = $2 AND posted_at IS NULL AND dismissed_at IS NULL
        "#,
    )
    .bind(tweet_id)
    .bind(user_id)
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}
