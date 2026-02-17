//! Thread domain - DB queries for threads
//!
//! All functions use the generic Executor pattern, allowing them to work with
//! both `&PgPool` (for standalone queries) and `&mut PgConnection` (for transactions).

use chrono::{DateTime, Utc};
use sqlx::types::Json;
use sqlx::{Executor, Postgres, QueryBuilder};

use super::super::models::{Thread, ThreadStatus, ThreadWithTweets, Tweet, TweetForPosting};

/// Parsed status filter enum for type-safe query building
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStatusFilter {
    Pending,
    Posted,
    All,
}

impl ThreadStatusFilter {
    pub fn from_str(s: Option<&str>) -> Self {
        match s {
            Some("pending") => ThreadStatusFilter::Pending,
            Some("posted") => ThreadStatusFilter::Posted,
            _ => ThreadStatusFilter::All,
        }
    }

    /// Returns SQL WHERE clause fragment for filtering by thread status
    fn where_clause(&self) -> &'static str {
        match self {
            ThreadStatusFilter::Pending => "AND status IN ('draft', 'posting', 'partial_failed')",
            ThreadStatusFilter::Posted => "AND status = 'posted'",
            ThreadStatusFilter::All => "",
        }
    }
}

/// Count threads for pagination
pub async fn count_threads<'e, E>(
    executor: E,
    user_id: i64,
    status_filter: Option<&str>,
) -> Result<i64, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let filter = ThreadStatusFilter::from_str(status_filter);
    let query = format!(
        "SELECT COUNT(*) FROM tweet_threads WHERE user_id = $1 {}",
        filter.where_clause()
    );

    let (count,): (i64,) = sqlx::query_as(&query)
        .bind(user_id)
        .fetch_one(executor)
        .await?;

    Ok(count)
}

/// List all threads for a user (no pagination)
#[allow(dead_code)]
pub async fn list_threads<'e, E>(
    executor: E,
    user_id: i64,
    status_filter: Option<&str>,
) -> Result<Vec<Thread>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let filter = ThreadStatusFilter::from_str(status_filter);
    let query = format!(
        r#"SELECT id, user_id, title,
                  COALESCE(copy_options, '[]'::jsonb) as copy_options,
                  status, created_at, posted_at, first_tweet_id
           FROM tweet_threads
           WHERE user_id = $1 {}
           ORDER BY created_at DESC"#,
        filter.where_clause()
    );

    sqlx::query_as(&query)
        .bind(user_id)
        .fetch_all(executor)
        .await
}

/// List threads for a user with pagination
pub async fn list_threads_paginated<'e, E>(
    executor: E,
    user_id: i64,
    status_filter: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Thread>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let filter = ThreadStatusFilter::from_str(status_filter);
    let query = format!(
        r#"SELECT id, user_id, title,
                  COALESCE(copy_options, '[]'::jsonb) as copy_options,
                  status, created_at, posted_at, first_tweet_id
           FROM tweet_threads
           WHERE user_id = $1 {}
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

/// Get a thread with its tweets
pub async fn get_thread_with_tweets<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
) -> Result<Option<ThreadWithTweets>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let thread: Option<Thread> = sqlx::query_as(
        r#"
        SELECT id, user_id, title,
               COALESCE(copy_options, '[]'::jsonb) as copy_options,
               status, created_at, posted_at, first_tweet_id
        FROM tweet_threads
        WHERE id = $1 AND user_id = $2
        "#,
    )
    .bind(thread_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;

    let Some(thread) = thread else {
        return Ok(None);
    };

    // Note: This requires a second executor call. For transactional use,
    // caller should use get_thread_tweets separately or we need a pool reference.
    // For now, this function works best with a pool.
    Ok(Some(ThreadWithTweets {
        thread,
        tweets: vec![],
    }))
}

/// Get tweets in a thread
pub async fn get_thread_tweets<'e, E>(
    executor: E,
    thread_id: i64,
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
        WHERE thread_id = $1 AND user_id = $2
        ORDER BY thread_position ASC
        "#,
    )
    .bind(thread_id)
    .bind(user_id)
    .fetch_all(executor)
    .await
}

/// Internal struct for batch fetching tweets with their thread_id
#[allow(dead_code)]
#[derive(sqlx::FromRow)]
struct ThreadTweetWithThreadId {
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

/// Get thread status as enum
pub async fn get_thread_status<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
) -> Result<Option<ThreadStatus>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let row: Option<(String,)> =
        sqlx::query_as("SELECT status FROM tweet_threads WHERE id = $1 AND user_id = $2")
            .bind(thread_id)
            .bind(user_id)
            .fetch_optional(executor)
            .await?;

    Ok(row.map(|(s,)| ThreadStatus::from_str(&s)))
}

/// Create a new thread
/// Note: Caller manages transaction. Use with a transaction for atomicity.
pub async fn create_thread<'e, E>(
    executor: E,
    user_id: i64,
    title: Option<&str>,
) -> Result<i64, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let (thread_id,): (i64,) = sqlx::query_as(
        r#"
        INSERT INTO tweet_threads (user_id, title, status, created_at)
        VALUES ($1, $2, 'draft', NOW())
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(title)
    .fetch_one(executor)
    .await?;

    Ok(thread_id)
}

/// Assign tweets to thread with positions (batch update using unnest)
/// Note: Caller manages transaction. Use with a transaction for atomicity.
pub async fn assign_tweets_to_thread<'e, E>(
    executor: E,
    thread_id: i64,
    tweet_ids: &[i64],
    user_id: i64,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let positions: Vec<i32> = (0..tweet_ids.len() as i32).collect();
    sqlx::query(
        r#"
        UPDATE tweet_collateral tc
        SET thread_id = $1, thread_position = batch.position
        FROM (SELECT unnest($2::bigint[]) AS id, unnest($3::int[]) AS position) AS batch
        WHERE tc.id = batch.id AND tc.user_id = $4
        "#,
    )
    .bind(thread_id)
    .bind(tweet_ids)
    .bind(&positions)
    .bind(user_id)
    .execute(executor)
    .await?;

    Ok(())
}

/// Update thread title
pub async fn update_thread_title<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
    title: &str,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query("UPDATE tweet_threads SET title = $1 WHERE id = $2 AND user_id = $3")
        .bind(title)
        .bind(thread_id)
        .bind(user_id)
        .execute(executor)
        .await?;
    Ok(())
}

/// Update thread status
pub async fn update_thread_status<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
    status: &str,
    first_tweet_id: Option<&str>,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"
        UPDATE tweet_threads
        SET status = $1,
            posted_at = NOW(),
            first_tweet_id = COALESCE($2, first_tweet_id)
        WHERE id = $3 AND user_id = $4
        "#,
    )
    .bind(status)
    .bind(first_tweet_id)
    .bind(thread_id)
    .bind(user_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Set thread to posting status
pub async fn set_thread_posting<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        "UPDATE tweet_threads
         SET status = 'posting'
         WHERE id = $1 AND user_id = $2 AND status IN ('draft', 'partial_failed')",
    )
    .bind(thread_id)
    .bind(user_id)
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Check if thread exists and belongs to user
pub async fn thread_exists<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let exists: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM tweet_threads WHERE id = $1 AND user_id = $2")
            .bind(thread_id)
            .bind(user_id)
            .fetch_optional(executor)
            .await?;

    Ok(exists.is_some())
}

/// Unlink all tweets from a thread
pub async fn unlink_all_tweets_from_thread<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "UPDATE tweet_collateral SET thread_id = NULL, thread_position = NULL WHERE thread_id = $1 AND user_id = $2"
    )
    .bind(thread_id)
    .bind(user_id)
    .execute(executor)
    .await?;

    Ok(())
}

/// Delete thread record
pub async fn delete_thread_record<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query("DELETE FROM tweet_threads WHERE id = $1 AND user_id = $2")
        .bind(thread_id)
        .bind(user_id)
        .execute(executor)
        .await?;

    Ok(())
}

/// Verify tweets belong to user and are unposted
pub async fn verify_tweets_for_thread<'e, E>(
    executor: E,
    tweet_ids: &[i64],
    user_id: i64,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM tweet_collateral WHERE id = ANY($1) AND user_id = $2 AND posted_at IS NULL"
    )
    .bind(tweet_ids)
    .bind(user_id)
    .fetch_one(executor)
    .await?;

    Ok(count == tweet_ids.len() as i64)
}

/// Get tweets for posting (with media info)
pub async fn get_tweets_for_posting<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
) -> Result<Vec<TweetForPosting>, sqlx::Error>
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
        WHERE thread_id = $1 AND user_id = $2 AND posted_at IS NULL
        ORDER BY thread_position ASC
        "#,
    )
    .bind(thread_id)
    .bind(user_id)
    .fetch_all(executor)
    .await
}

/// Mark a tweet in a thread as posted
pub async fn mark_thread_tweet_posted<'e, E>(
    executor: E,
    collateral_id: i64,
    user_id: i64,
    twitter_id: &str,
    reply_to: Option<&str>,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        r#"
        UPDATE tweet_collateral
        SET posted_at = NOW(),
            tweet_id = $1,
            reply_to_tweet_id = $2,
            publish_status = 'posted',
            publish_error = NULL,
            publish_error_at = NULL
        WHERE id = $3 AND user_id = $4
        "#,
    )
    .bind(twitter_id)
    .bind(reply_to)
    .bind(collateral_id)
    .bind(user_id)
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Set a thread tweet publish status to posting
pub async fn set_thread_tweet_posting<'e, E>(
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
        WHERE id = $1 AND user_id = $2 AND posted_at IS NULL AND publish_status IN ('pending', 'failed')
        "#,
    )
    .bind(tweet_id)
    .bind(user_id)
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Mark a thread tweet publish attempt as failed
pub async fn mark_thread_tweet_publish_failed<'e, E>(
    executor: E,
    collateral_id: i64,
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
    .bind(collateral_id)
    .bind(user_id)
    .bind(error)
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Verify all tweets belong to a specific thread and user
pub async fn verify_tweets_in_thread<'e, E>(
    executor: E,
    tweet_ids: &[i64],
    thread_id: i64,
    user_id: i64,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM tweet_collateral WHERE id = ANY($1) AND thread_id = $2 AND user_id = $3",
    )
    .bind(tweet_ids)
    .bind(thread_id)
    .bind(user_id)
    .fetch_one(executor)
    .await?;

    Ok(count == tweet_ids.len() as i64)
}

/// Get latest posted tweet id for a thread, for continuing partial thread retries
pub async fn get_last_posted_tweet_id<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
) -> Result<Option<String>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let result: Option<(Option<String>,)> = sqlx::query_as(
        r#"
        SELECT tweet_id
        FROM tweet_collateral
        WHERE thread_id = $1
            AND user_id = $2
            AND posted_at IS NOT NULL
            AND tweet_id IS NOT NULL
        ORDER BY thread_position DESC
        LIMIT 1
        "#,
    )
    .bind(thread_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;

    Ok(result.and_then(|(tweet_id,)| tweet_id))
}

/// Reorder tweets in a thread (batch update)
pub async fn reorder_thread_tweets<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
    tweet_ids: &[i64],
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    // Batch UPDATE using unnest() - replaces N+1 loop with a single query
    let positions: Vec<i32> = (0..tweet_ids.len() as i32).collect();
    sqlx::query(
        r#"
        UPDATE tweet_collateral tc
        SET thread_position = batch.position
        FROM (SELECT unnest($1::bigint[]) AS id, unnest($2::int[]) AS position) AS batch
        WHERE tc.id = batch.id AND tc.thread_id = $3 AND tc.user_id = $4
        "#,
    )
    .bind(tweet_ids)
    .bind(&positions)
    .bind(thread_id)
    .bind(user_id)
    .execute(executor)
    .await?;

    Ok(())
}

/// Get tweet's current thread assignment (returns thread_id if any)
pub async fn get_tweet_thread_info<'e, E>(
    executor: E,
    tweet_id: i64,
    user_id: i64,
) -> Result<Option<Option<i64>>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let result: Option<(Option<i64>,)> = sqlx::query_as(
        "SELECT thread_id FROM tweet_collateral WHERE id = $1 AND user_id = $2 AND posted_at IS NULL",
    )
    .bind(tweet_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;

    Ok(result.map(|(thread_id,)| thread_id))
}

/// Shift thread positions up (for inserting at a position)
pub async fn shift_positions_up<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
    from_position: i32,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "UPDATE tweet_collateral SET thread_position = thread_position + 1 WHERE thread_id = $1 AND user_id = $2 AND thread_position >= $3",
    )
    .bind(thread_id)
    .bind(user_id)
    .bind(from_position)
    .execute(executor)
    .await?;
    Ok(())
}

/// Get the maximum thread position (for appending)
pub async fn get_max_thread_position<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
) -> Result<Option<i32>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let (max_pos,): (Option<i32>,) = sqlx::query_as(
        "SELECT MAX(thread_position) FROM tweet_collateral WHERE thread_id = $1 AND user_id = $2",
    )
    .bind(thread_id)
    .bind(user_id)
    .fetch_one(executor)
    .await?;

    Ok(max_pos)
}

/// Assign a tweet to a thread at a specific position
pub async fn assign_tweet_to_thread<'e, E>(
    executor: E,
    tweet_id: i64,
    thread_id: i64,
    user_id: i64,
    position: i32,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query("UPDATE tweet_collateral SET thread_id = $1, thread_position = $2 WHERE id = $3 AND user_id = $4")
        .bind(thread_id)
        .bind(position)
        .bind(tweet_id)
        .bind(user_id)
        .execute(executor)
        .await?;
    Ok(())
}

/// Get tweet's position in thread
pub async fn get_tweet_position_in_thread<'e, E>(
    executor: E,
    tweet_id: i64,
    thread_id: i64,
    user_id: i64,
) -> Result<Option<Option<i32>>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let result: Option<(Option<i32>,)> = sqlx::query_as(
        "SELECT thread_position FROM tweet_collateral WHERE id = $1 AND thread_id = $2 AND user_id = $3",
    )
    .bind(tweet_id)
    .bind(thread_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;

    Ok(result.map(|(pos,)| pos))
}

/// Unlink tweet from thread
pub async fn unlink_tweet_from_thread<'e, E>(
    executor: E,
    tweet_id: i64,
    user_id: i64,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "UPDATE tweet_collateral SET thread_id = NULL, thread_position = NULL WHERE id = $1 AND user_id = $2",
    )
    .bind(tweet_id)
    .bind(user_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Shift thread positions down (for removing a tweet)
pub async fn shift_positions_down<'e, E>(
    executor: E,
    thread_id: i64,
    user_id: i64,
    after_position: i32,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "UPDATE tweet_collateral SET thread_position = thread_position - 1 WHERE thread_id = $1 AND user_id = $2 AND thread_position > $3",
    )
    .bind(thread_id)
    .bind(user_id)
    .bind(after_position)
    .execute(executor)
    .await?;
    Ok(())
}

/// Verify tweet exists and is unposted
pub async fn verify_tweet_exists_unposted<'e, E>(
    executor: E,
    tweet_id: i64,
    user_id: i64,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let exists: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM tweet_collateral WHERE id = $1 AND user_id = $2 AND posted_at IS NULL",
    )
    .bind(tweet_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;

    Ok(exists.is_some())
}

/// Update tweet collateral (media attachments)
pub async fn update_tweet_collateral<'e, E>(
    executor: E,
    tweet_id: i64,
    user_id: i64,
    text: Option<&str>,
    image_capture_ids: Option<&Vec<i64>>,
    video_clip: Option<Option<serde_json::Value>>,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    if text.is_none() && image_capture_ids.is_none() && video_clip.is_none() {
        return Ok(true);
    }

    let mut builder = QueryBuilder::<Postgres>::new("UPDATE tweet_collateral SET ");
    let mut separated = builder.separated(", ");

    if let Some(text) = text {
        separated.push("text = ").push_bind_unseparated(text);
    }

    if let Some(image_capture_ids) = image_capture_ids {
        separated
            .push("image_capture_ids = ")
            .push_bind_unseparated(image_capture_ids);
    }

    if let Some(video_clip) = video_clip {
        separated
            .push("video_clip = ")
            .push_bind_unseparated(video_clip);
    }

    builder.push(" WHERE id = ");
    builder.push_bind(tweet_id);
    builder.push(" AND user_id = ");
    builder.push_bind(user_id);

    let result = builder.build().execute(executor).await?;

    Ok(result.rows_affected() > 0)
}
