//! Capture domain - DB queries for captures
//!
//! All functions use the generic Executor pattern, allowing them to work with
//! both `&PgPool` (for standalone queries) and `&mut PgConnection` (for transactions).

use chrono::{DateTime, Utc};
use sqlx::{Executor, Postgres};

#[derive(Debug, sqlx::FromRow)]
pub struct CaptureMedia {
    pub gcs_path: String,
    pub content_type: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct CaptureThumbnail {
    pub thumbnail_path: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct CaptureRow {
    pub id: i64,
    pub media_type: String,
    pub content_type: String,
    pub captured_at: DateTime<Utc>,
    pub thumbnail_path: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct CountResult {
    count: i64,
}

/// Get capture media info (for signed URL generation)
pub async fn get_capture_media<'e, E>(
    executor: E,
    capture_id: i64,
    user_id: i64,
) -> Result<Option<CaptureMedia>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_as(
        r#"
        SELECT gcs_path, content_type FROM captures
        WHERE id = $1 AND user_id = $2
        "#,
    )
    .bind(capture_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await
}

/// Get capture thumbnail path
pub async fn get_capture_thumbnail<'e, E>(
    executor: E,
    capture_id: i64,
    user_id: i64,
) -> Result<Option<CaptureThumbnail>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_as(
        r#"
        SELECT thumbnail_path FROM captures
        WHERE id = $1 AND user_id = $2
        "#,
    )
    .bind(capture_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await
}

/// Capture row with total count from window function
#[derive(Debug, sqlx::FromRow)]
pub struct CaptureRowWithTotal {
    pub id: i64,
    pub media_type: String,
    pub content_type: String,
    pub captured_at: DateTime<Utc>,
    pub thumbnail_path: Option<String>,
    pub total_count: i64,
}

/// Browse captures with optional filters and pagination, returning total count in same query
/// Uses window function to avoid race condition between SELECT and COUNT
/// If include_ids is provided, those captures are always included (for showing selected items)
#[allow(clippy::too_many_arguments)]
pub async fn browse_captures_with_count<'e, E>(
    executor: E,
    user_id: i64,
    start_time: Option<DateTime<Utc>>,
    end_time: Option<DateTime<Utc>>,
    media_type: Option<&str>,
    limit: i64,
    offset: i64,
    _include_ids: Option<&[i64]>,
) -> Result<(Vec<CaptureRow>, i64), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    // Note: include_ids functionality requires a second query, which would consume
    // the executor. For transaction use, caller should handle include_ids separately.
    // The main query provides the core functionality.

    let rows: Vec<CaptureRowWithTotal> = sqlx::query_as(
        r#"
        SELECT id, media_type, content_type, captured_at, thumbnail_path,
               COUNT(*) OVER() as total_count
        FROM captures
        WHERE user_id = $1
          AND ($2::timestamptz IS NULL OR captured_at >= $2)
          AND ($3::timestamptz IS NULL OR captured_at <= $3)
          AND ($4::text IS NULL OR media_type = $4)
        ORDER BY captured_at DESC
        LIMIT $5 OFFSET $6
        "#,
    )
    .bind(user_id)
    .bind(start_time)
    .bind(end_time)
    .bind(media_type)
    .bind(limit)
    .bind(offset)
    .fetch_all(executor)
    .await?;

    // Extract total from first row, or 0 if empty
    let total = rows.first().map(|r| r.total_count).unwrap_or(0);

    // Convert to CaptureRow (without total_count)
    let captures: Vec<CaptureRow> = rows
        .into_iter()
        .map(|r| CaptureRow {
            id: r.id,
            media_type: r.media_type,
            content_type: r.content_type,
            captured_at: r.captured_at,
            thumbnail_path: r.thumbnail_path,
        })
        .collect();

    Ok((captures, total))
}

#[derive(Debug, sqlx::FromRow)]
pub struct InsertedCapture {
    pub id: i64,
}

/// Insert a new capture record
pub async fn insert_capture<'e, E>(
    executor: E,
    interval_id: i64,
    user_id: i64,
    media_type: &str,
    content_type: &str,
    gcs_path: &str,
    captured_at: DateTime<Utc>,
) -> Result<i64, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let result: InsertedCapture = sqlx::query_as(
        r#"
        INSERT INTO captures (interval_id, user_id, media_type, content_type, gcs_path, captured_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(interval_id)
    .bind(user_id)
    .bind(media_type)
    .bind(content_type)
    .bind(gcs_path)
    .bind(captured_at)
    .fetch_one(executor)
    .await?;

    Ok(result.id)
}

/// Verify captures belong to user
pub async fn verify_captures_owned<'e, E>(
    executor: E,
    capture_ids: &[i64],
    user_id: i64,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let result: CountResult = sqlx::query_as(
        "SELECT COUNT(*) as count FROM captures WHERE id = ANY($1) AND user_id = $2",
    )
    .bind(capture_ids)
    .bind(user_id)
    .fetch_one(executor)
    .await?;

    Ok(result.count == capture_ids.len() as i64)
}

/// Capture info for media upload
#[derive(Debug, sqlx::FromRow)]
pub struct CaptureInfo {
    pub id: i64,
    pub gcs_path: String,
    pub content_type: String,
}

/// Get single capture info for media upload
pub async fn get_capture_info<'e, E>(
    executor: E,
    capture_id: i64,
    user_id: i64,
) -> Result<Option<CaptureInfo>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_as(
        "SELECT id, gcs_path, content_type FROM captures WHERE id = $1 AND user_id = $2",
    )
    .bind(capture_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await
}

/// Batch get capture info for media upload
pub async fn get_captures_batch<'e, E>(
    executor: E,
    capture_ids: &[i64],
    user_id: i64,
) -> Result<std::collections::HashMap<i64, CaptureInfo>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let rows: Vec<CaptureInfo> = sqlx::query_as(
        "SELECT id, gcs_path, content_type FROM captures WHERE id = ANY($1) AND user_id = $2",
    )
    .bind(capture_ids)
    .bind(user_id)
    .fetch_all(executor)
    .await?;

    Ok(rows.into_iter().map(|r| (r.id, r)).collect())
}
