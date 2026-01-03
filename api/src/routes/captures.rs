//! Capture and media endpoints (/captures/*, /media/*, /activity)

use axum::{
    Json, Router,
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;

use crate::constants::{BUCKET_NAME, SIGNED_URL_EXPIRY_SECS};
use crate::domain::{activities, captures as captures_domain};
use crate::services::{error::LogErr, rate_limit::DAEMON_RATE_LIMITER, twitter};
use crate::{get_extension, Activity, ActivityEvent, AppState, BatchCaptureResponse};
use super::auth::AuthUser;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/captures/batch", post(capture_batch))
        .route("/captures/browse", get(browse_captures))
        .route("/captures/{id}/url", get(get_capture_url))
        .route("/captures/{id}/thumbnail", get(get_capture_thumbnail))
        .route("/media/{*path}", get(serve_media))
        .route("/activity", post(activity))
}

/// Helper to extract user_id from Bearer token (for daemon auth)
pub async fn get_user_id_from_bearer(db: &PgPool, headers: &HeaderMap) -> Result<i64, StatusCode> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(StatusCode::UNAUTHORIZED)?;

    twitter::get_user_by_api_token(db, token)
        .await
        .log_500("Get user by API token error")?
        .ok_or(StatusCode::UNAUTHORIZED)
}

#[derive(Serialize)]
struct SignedUrlResponse {
    url: String,
    content_type: String,
}

/// GET /captures/:id/url - Get a signed URL for a capture
async fn get_capture_url(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path(capture_id): Path<i64>,
) -> Result<Json<SignedUrlResponse>, StatusCode> {

    // Get capture info and verify ownership
    let capture = captures_domain::get_capture_media(&state.db, capture_id, user_id)
        .await
        .log_500("Get capture media error")?
        .ok_or(StatusCode::NOT_FOUND)?;

    let gcs_path = capture.gcs_path;
    let content_type = capture.content_type;

    // If local storage is configured, return a local URL
    if state.local_storage_path.is_some() {
        // Return a URL that points to our /media endpoint
        let url = format!("/media/{}", gcs_path);
        return Ok(Json(SignedUrlResponse { url, content_type }));
    }

    // Generate signed URL (15 min expiry) using cloud-storage crate
    let client = cloud_storage::Client::default();
    let object = client
        .object()
        .read(BUCKET_NAME, &gcs_path)
        .await
        .log_500("Object read error")?;

    let signed_url = object
        .download_url(SIGNED_URL_EXPIRY_SECS)
        .log_500("Signed URL error")?;

    Ok(Json(SignedUrlResponse {
        url: signed_url,
        content_type,
    }))
}

#[derive(Serialize)]
struct ThumbnailUrlResponse {
    url: Option<String>,
    ready: bool,
}

/// GET /captures/:id/thumbnail - Get a thumbnail URL for a capture
async fn get_capture_thumbnail(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path(capture_id): Path<i64>,
) -> Result<Json<ThumbnailUrlResponse>, StatusCode> {

    // Get capture info and verify ownership
    let capture = captures_domain::get_capture_thumbnail(&state.db, capture_id, user_id)
        .await
        .log_500("Get capture thumbnail error")?
        .ok_or(StatusCode::NOT_FOUND)?;

    let thumbnail_path = capture.thumbnail_path;

    // If no thumbnail yet, return not ready
    let Some(thumb_path) = thumbnail_path else {
        return Ok(Json(ThumbnailUrlResponse {
            url: None,
            ready: false,
        }));
    };

    // If local storage is configured, return a local URL
    if state.local_storage_path.is_some() {
        let url = format!("/media/{}", thumb_path);
        return Ok(Json(ThumbnailUrlResponse {
            url: Some(url),
            ready: true,
        }));
    }

    // Generate signed URL for GCS
    let client = cloud_storage::Client::default();
    let object = client
        .object()
        .read(BUCKET_NAME, &thumb_path)
        .await
        .log_500("Thumbnail object read error")?;

    let signed_url = object
        .download_url(SIGNED_URL_EXPIRY_SECS)
        .log_500("Thumbnail signed URL error")?;

    Ok(Json(ThumbnailUrlResponse {
        url: Some(signed_url),
        ready: true,
    }))
}

#[derive(Deserialize)]
struct BrowseCapturesQuery {
    start: Option<String>,
    end: Option<String>,
    #[serde(rename = "type")]
    media_type: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
    /// Comma-separated list of capture IDs to always include in results
    include_ids: Option<String>,
}

#[derive(Serialize)]
struct CaptureItem {
    id: i64,
    media_type: String,
    content_type: String,
    captured_at: DateTime<Utc>,
    thumbnail_url: Option<String>,
    thumbnail_ready: bool,
}

#[derive(Serialize)]
struct BrowseCapturesResponse {
    captures: Vec<CaptureItem>,
    total: i64,
    has_more: bool,
}

/// GET /captures/browse - Browse captures with optional filters
async fn browse_captures(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Query(query): Query<BrowseCapturesQuery>,
) -> Result<Json<BrowseCapturesResponse>, StatusCode> {

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0);

    // Parse optional time filters
    let start_time: Option<DateTime<Utc>> = query
        .start
        .as_ref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let end_time: Option<DateTime<Utc>> = query
        .end
        .as_ref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    // Parse include_ids (comma-separated)
    let include_ids: Option<Vec<i64>> = query.include_ids.as_ref().map(|s| {
        s.split(',')
            .filter_map(|id| id.trim().parse::<i64>().ok())
            .collect()
    });

    // Get captures and total count in a single query to avoid race conditions
    let (captures, total) = captures_domain::browse_captures_with_count(
        &state.db,
        user_id,
        start_time,
        end_time,
        query.media_type.as_deref(),
        limit,
        offset,
        include_ids.as_deref(),
    )
    .await
    .log_500("Browse captures error")?;

    let use_local = state.local_storage_path.is_some();

    let items: Vec<CaptureItem> = captures
        .into_iter()
        .map(|row| {
            let (thumbnail_url, thumbnail_ready) = match row.thumbnail_path {
                Some(path) if use_local => (Some(format!("/media/{}", path)), true),
                Some(_) => (Some(format!("/captures/{}/thumbnail", row.id)), true),
                None => (None, false),
            };

            CaptureItem {
                id: row.id,
                media_type: row.media_type,
                content_type: row.content_type,
                captured_at: row.captured_at,
                thumbnail_url,
                thumbnail_ready,
            }
        })
        .collect();

    let has_more = (offset + limit) < total;

    Ok(Json(BrowseCapturesResponse {
        captures: items,
        total,
        has_more,
    }))
}

/// GET /media/*path - Serve local media files
async fn serve_media(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    // Security: reject paths with traversal attempts or null bytes upfront
    if path.contains("..") || path.contains('\0') {
        return Err(StatusCode::FORBIDDEN);
    }

    let local_path = state
        .local_storage_path
        .as_ref()
        .ok_or(StatusCode::NOT_FOUND)?;

    let full_path = local_path.join(&path);

    // Security: ensure the path doesn't escape the storage directory
    // canonicalize() resolves symlinks and normalizes the path
    let canonical = full_path
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;  // Silent - expected for missing files
    let storage_canonical = local_path
        .canonicalize()
        .log_500("Failed to canonicalize storage path")?;

    if !canonical.starts_with(&storage_canonical) {
        return Err(StatusCode::FORBIDDEN);
    }

    // Read file
    let bytes = tokio::fs::read(&canonical)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;  // Silent - expected for missing files

    // Determine content type from extension
    let content_type = match canonical.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        _ => "application/octet-stream",
    };

    // Media files are immutable (path includes timestamp), so we can cache aggressively
    // Cache for 1 year (max-age), mark as immutable to prevent revalidation
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    ))
}

/// POST /captures/batch - Upload multiple captures in one request
/// Accepts multipart form data with:
/// - Multiple "file" fields containing the media bytes
/// - Each file should have proper content-type (image/* or video/*)
/// - X-Interval-ID header for all captures
async fn capture_batch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<BatchCaptureResponse>), StatusCode> {
    let user_id = get_user_id_from_bearer(&state.db, &headers).await?;

    // Per-user rate limiting
    if !DAEMON_RATE_LIMITER.check(user_id) {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    let interval_id: i64 = headers
        .get("x-interval-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let mut ids = Vec::new();
    let mut failed = 0usize;

    while let Some(field) = multipart
        .next_field()
        .await
        .log_status("Multipart field error", StatusCode::BAD_REQUEST)?
    {
        let content_type = field
            .content_type()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let media_type = if content_type.starts_with("image/") {
            "image"
        } else if content_type.starts_with("video/") {
            "video"
        } else {
            eprintln!(
                "[capture_batch] Skipping unsupported content type: {}",
                content_type
            );
            failed += 1;
            continue;
        };

        let body = match field.bytes().await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[capture_batch] Failed to read field bytes: {}", e);
                failed += 1;
                continue;
            }
        };

        let now = Utc::now();
        let day_bucket = now.format("%Y-%m-%d").to_string();
        let timestamp = now.timestamp_millis();
        let ext = get_extension(&content_type);

        let relative_path = format!(
            "{}/user_{}/{}/{}.{}",
            media_type, user_id, day_bucket, timestamp, ext
        );

        // Write to local storage or GCS
        let write_result = if let Some(local_path) = &state.local_storage_path {
            let full_path = local_path.join(&relative_path);
            if let Some(parent) = full_path.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    eprintln!(
                        "[capture_batch] Failed to create directory {:?}: {}",
                        parent, e
                    );
                    failed += 1;
                    continue;
                }
            }
            match tokio::fs::write(&full_path, &body).await {
                Ok(()) => {
                    println!(
                        "[capture_batch] LOCAL: Saved {} bytes to {:?}",
                        body.len(),
                        full_path
                    );
                    Ok(())
                }
                Err(e) => {
                    eprintln!(
                        "[capture_batch] Failed to write file {:?}: {}",
                        full_path, e
                    );
                    Err(e)
                }
            }
        } else {
            let bucket = format!("projects/_/buckets/{}", BUCKET_NAME);
            match state
                .gcs
                .write_object(&bucket, &relative_path, body.clone())
                .send_buffered()
                .await
            {
                Ok(_) => {
                    println!("[capture_batch] GCS: Uploaded to {}", relative_path);
                    Ok(())
                }
                Err(e) => {
                    eprintln!("[capture_batch] GCS upload failed: {}", e);
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e.to_string(),
                    ))
                }
            }
        };

        if write_result.is_err() {
            failed += 1;
            continue;
        }

        // Store reference in DB
        match captures_domain::insert_capture(
            &state.db,
            interval_id,
            user_id,
            media_type,
            &content_type,
            &relative_path,
            now,
        )
        .await
        {
            Ok(id) => {
                ids.push(id);
            }
            Err(e) => {
                eprintln!("[capture_batch] DB insert failed: {}", e);
                // Clean up orphaned file on DB failure
                if let Some(local_path) = &state.local_storage_path {
                    let full_path = local_path.join(&relative_path);
                    if let Err(cleanup_err) = tokio::fs::remove_file(&full_path).await {
                        eprintln!(
                            "[capture_batch] Failed to clean up orphaned file {:?}: {}",
                            full_path, cleanup_err
                        );
                    } else {
                        eprintln!("[capture_batch] Cleaned up orphaned file: {:?}", full_path);
                    }
                } else {
                    // For GCS, attempt to delete the orphaned object
                    let client = cloud_storage::Client::default();
                    if let Err(cleanup_err) = client.object().delete(BUCKET_NAME, &relative_path).await {
                        eprintln!(
                            "[capture_batch] Failed to clean up orphaned GCS object {}: {}",
                            relative_path, cleanup_err
                        );
                    } else {
                        eprintln!("[capture_batch] Cleaned up orphaned GCS object: {}", relative_path);
                    }
                }
                failed += 1;
            }
        }

        // Small delay between files to ensure unique timestamps
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
    }

    println!(
        "[capture_batch] Batch complete: {} uploaded, {} failed",
        ids.len(),
        failed
    );

    Ok((StatusCode::CREATED, Json(BatchCaptureResponse {
        ids: ids.clone(),
        uploaded: ids.len(),
        failed,
    })))
}

async fn activity(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(activity_list): Json<Vec<Activity>>,
) -> Result<StatusCode, StatusCode> {
    // Authenticate via bearer token
    let user_id = get_user_id_from_bearer(&state.db, &headers).await?;

    // Per-user rate limiting
    if !DAEMON_RATE_LIMITER.check(user_id) {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    for activity in activity_list {
        let (event_type, application, window) = match &activity.event {
            ActivityEvent::ForegroundSwitch {
                new_active,
                window_title,
            } => (
                "ForegroundSwitch",
                Some(new_active.as_str()),
                Some(window_title.as_str()),
            ),
            ActivityEvent::MouseClick => ("MouseClick", None, None),
        };

        activities::insert_activity(
            &state.db,
            user_id,
            activity.timestamp,
            activity.interval_id,
            event_type,
            application,
            window,
        )
        .await
        .log_500("Insert activity error")?;
    }

    Ok(StatusCode::CREATED)
}
