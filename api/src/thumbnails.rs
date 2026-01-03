//! Thumbnail generation background job using apalis
//!
//! Runs as a scheduled cron job that batch-processes captures without thumbnails.

use apalis::prelude::*;
use apalis_cron::{CronStream, Schedule};
use apalis_sql::postgres::PostgresStorage;
use std::str::FromStr;
use bytes::Bytes;
use image::ImageReader;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

use crate::models::CaptureForThumbnail;

const THUMBNAIL_WIDTH: u32 = 300;
const BATCH_SIZE: i64 = 50;
const THUMBNAIL_QUALITY: u8 = 80;
const MAX_ATTEMPTS: i32 = 5;

/// Job input - marker for batch processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThumbnailJob {
    pub scheduled_at: chrono::DateTime<chrono::Utc>,
}

impl From<chrono::DateTime<chrono::Utc>> for ThumbnailJob {
    fn from(dt: chrono::DateTime<chrono::Utc>) -> Self {
        ThumbnailJob { scheduled_at: dt }
    }
}

/// Shared context for thumbnail processing
#[derive(Clone)]
pub struct ThumbnailContext {
    pub pool: PgPool,
    pub gcs: google_cloud_storage::client::Storage,
    pub local_storage_path: Option<PathBuf>,
    pub bucket_name: String,
}

/// Job handler - processes a batch of thumbnails
/// Always returns Ok - individual capture failures are logged but don't fail the job
async fn process_thumbnail_job(
    _job: ThumbnailJob,
    ctx: Data<ThumbnailContext>,
) -> Result<(), Error> {
    match process_thumbnail_batch(&ctx).await {
        Ok((processed, failed)) => {
            if processed > 0 || failed > 0 {
                println!("[thumbnails] Batch complete: {} processed, {} failed", processed, failed);
            }
        }
        Err(e) => {
            // Only log - don't fail the job for batch-level errors
            eprintln!("[thumbnails] Batch error (will retry): {}", e);
        }
    }
    Ok(())
}

/// Start the thumbnail worker
pub async fn run_thumbnail_worker(
    pool: PgPool,
    gcs: google_cloud_storage::client::Storage,
    local_storage_path: Option<PathBuf>,
    bucket_name: String,
) {
    let ctx = ThumbnailContext {
        pool: pool.clone(),
        gcs,
        local_storage_path,
        bucket_name,
    };

    // Run apalis migrations
    PostgresStorage::setup(&pool)
        .await
        .expect("Failed to set up apalis storage");

    // Set up postgres storage for job queue
    let storage: PostgresStorage<ThumbnailJob> = PostgresStorage::new(pool.clone());

    // Create cron schedule - every 30 seconds
    let schedule = Schedule::from_str("*/30 * * * * *").unwrap();
    let cron = CronStream::new(schedule);

    // Pipe cron to storage so jobs persist and distribute
    let backend = cron.pipe_to_storage(storage);

    println!("[thumbnails] Apalis worker starting (every 30s)");

    // Build worker
    let worker = WorkerBuilder::new("thumbnail-worker")
        .data(ctx)
        .backend(backend)
        .build_fn(process_thumbnail_job);

    // Use Monitor to keep worker running forever
    Monitor::new()
        .register(worker)
        .run()
        .await
        .expect("Thumbnail worker monitor failed");
}

/// Process a batch of captures that need thumbnails
/// Returns (processed_count, failed_count)
async fn process_thumbnail_batch(
    ctx: &ThumbnailContext,
) -> Result<(usize, usize), Box<dyn std::error::Error + Send + Sync>> {
    // Fetch captures without thumbnails (excluding those that have failed too many times)
    let captures: Vec<CaptureForThumbnail> = sqlx::query_as(
        r#"
        SELECT id, media_type, gcs_path, captured_at
        FROM captures
        WHERE thumbnail_path IS NULL AND thumbnail_attempts < $1
        ORDER BY captured_at DESC
        LIMIT $2
        "#,
    )
    .bind(MAX_ATTEMPTS)
    .bind(BATCH_SIZE)
    .fetch_all(&ctx.pool)
    .await?;

    if captures.is_empty() {
        return Ok((0, 0));
    }

    println!(
        "[thumbnails] Processing batch of {} captures",
        captures.len()
    );

    let mut processed = 0;
    let mut failed = 0;

    // Process in parallel using JoinSet
    let mut tasks = tokio::task::JoinSet::new();

    for capture in captures {
        let pool = ctx.pool.clone();
        let ctx_gcs = ctx.gcs.clone();
        let local_path = ctx.local_storage_path.clone();
        let bucket = ctx.bucket_name.clone();

        tasks.spawn(async move {
            let result = process_single_capture(&pool, &ctx_gcs, local_path.as_ref(), &bucket, &capture).await;

            if let Err(ref e) = result {
                let full_path = local_path.as_ref().map(|p| p.join(&capture.gcs_path));
                eprintln!(
                    "[thumbnails] Failed capture {} type={} gcs_path={} full_path={:?} (will increment attempts): {}",
                    capture.id, capture.media_type, capture.gcs_path, full_path, e
                );
                // Increment attempt counter so we eventually stop retrying
                // Log errors to avoid infinite retry loops if counter update fails
                if let Err(db_err) = sqlx::query(
                    "UPDATE captures SET thumbnail_attempts = thumbnail_attempts + 1 WHERE id = $1 AND captured_at = $2"
                )
                .bind(capture.id)
                .bind(capture.captured_at)
                .execute(&pool)
                .await
                {
                    eprintln!(
                        "[thumbnails] CRITICAL: Failed to increment retry counter for capture {}: {} (may cause infinite retries)",
                        capture.id, db_err
                    );
                }
            }

            result.map(|_| capture.id)
        });
    }

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(id)) => {
                println!("[thumbnails] Generated thumbnail for capture {}", id);
                processed += 1;
            }
            Ok(Err(_)) => {
                failed += 1;
            }
            Err(e) => {
                eprintln!("[thumbnails] Task panicked: {}", e);
                failed += 1;
            }
        }
    }

    Ok((processed, failed))
}

async fn process_single_capture(
    pool: &PgPool,
    gcs: &google_cloud_storage::client::Storage,
    local_storage_path: Option<&PathBuf>,
    bucket_name: &str,
    capture: &CaptureForThumbnail,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Download the original file
    let data = download_capture(gcs, local_storage_path, bucket_name, &capture.gcs_path).await?;

    // Generate thumbnail based on media type
    let thumbnail_data = if capture.media_type == "video" {
        generate_video_thumbnail(&data).await?
    } else {
        generate_image_thumbnail(&data)?
    };

    // Upload thumbnail
    let thumbnail_path = get_thumbnail_path(&capture.gcs_path);
    upload_thumbnail(
        gcs,
        local_storage_path,
        bucket_name,
        &thumbnail_path,
        &thumbnail_data,
    )
    .await?;

    // Update database (use both parts of composite primary key for TimescaleDB hypertable)
    // If DB update fails, clean up the orphaned thumbnail
    let db_result = sqlx::query("UPDATE captures SET thumbnail_path = $1 WHERE id = $2 AND captured_at = $3")
        .bind(&thumbnail_path)
        .bind(capture.id)
        .bind(capture.captured_at)
        .execute(pool)
        .await;

    if let Err(e) = db_result {
        // Clean up orphaned thumbnail on DB failure
        if let Err(cleanup_err) = delete_thumbnail(gcs, local_storage_path, bucket_name, &thumbnail_path).await {
            eprintln!(
                "[thumbnails] Failed to clean up orphaned thumbnail {}: {}",
                thumbnail_path, cleanup_err
            );
        } else {
            eprintln!("[thumbnails] Cleaned up orphaned thumbnail: {}", thumbnail_path);
        }
        return Err(Box::new(e));
    }

    Ok(())
}

async fn download_capture(
    gcs: &google_cloud_storage::client::Storage,
    local_storage_path: Option<&PathBuf>,
    bucket_name: &str,
    gcs_path: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(local_path) = local_storage_path {
        // Read from local filesystem
        let full_path = local_path.join(gcs_path);
        Ok(tokio::fs::read(&full_path).await?)
    } else {
        // Download from GCS (streaming response)
        let bucket = format!("projects/_/buckets/{}", bucket_name);
        let mut resp = gcs.read_object(&bucket, gcs_path).send().await?;
        let mut data = Vec::new();
        while let Some(chunk) = resp.next().await {
            data.extend_from_slice(&chunk?);
        }
        Ok(data)
    }
}

async fn upload_thumbnail(
    gcs: &google_cloud_storage::client::Storage,
    local_storage_path: Option<&PathBuf>,
    bucket_name: &str,
    thumbnail_path: &str,
    data: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(local_path) = local_storage_path {
        // Write to local filesystem
        let full_path = local_path.join(thumbnail_path);
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&full_path, data).await?;
        println!("[thumbnails] LOCAL: Saved thumbnail to {:?}", full_path);
    } else {
        // Upload to GCS (convert to Bytes for the API)
        let bucket = format!("projects/_/buckets/{}", bucket_name);
        let bytes = Bytes::copy_from_slice(data);
        gcs.write_object(&bucket, thumbnail_path, bytes)
            .send_buffered()
            .await?;
        println!("[thumbnails] GCS: Uploaded thumbnail to {}", thumbnail_path);
    }
    Ok(())
}

/// Delete a thumbnail from storage (for cleanup on DB failure)
async fn delete_thumbnail(
    _gcs: &google_cloud_storage::client::Storage,
    local_storage_path: Option<&PathBuf>,
    bucket_name: &str,
    thumbnail_path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(local_path) = local_storage_path {
        // Delete from local filesystem
        let full_path = local_path.join(thumbnail_path);
        tokio::fs::remove_file(&full_path).await?;
    } else {
        // Delete from GCS using cloud_storage crate (consistent with captures.rs)
        let client = cloud_storage::Client::default();
        client.object().delete(bucket_name, thumbnail_path).await?;
    }
    Ok(())
}

fn get_thumbnail_path(original_path: &str) -> String {
    // Convert: image/user_1/2025-01-01/123.png -> thumbnails/user_1/2025-01-01/123.jpg
    // Convert: video/user_1/2025-01-01/123.mp4 -> thumbnails/user_1/2025-01-01/123.jpg
    let path = std::path::Path::new(original_path);

    // Get parts after media_type (image/ or video/)
    let components: Vec<_> = path.components().collect();
    if components.len() < 2 {
        return format!("thumbnails/{}.jpg", original_path);
    }

    // Skip first component (image/video), rebuild with thumbnails prefix
    let rest: PathBuf = components[1..].iter().collect();
    let stem = rest.file_stem().unwrap_or_default().to_string_lossy();
    let parent = rest.parent().unwrap_or(std::path::Path::new(""));

    format!("thumbnails/{}/{}.jpg", parent.display(), stem)
}

fn generate_image_thumbnail(
    data: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let img = ImageReader::new(Cursor::new(data))
        .with_guessed_format()?
        .decode()?;

    // Resize maintaining aspect ratio
    let thumbnail = img.thumbnail(THUMBNAIL_WIDTH, THUMBNAIL_WIDTH * 2);

    // Encode as JPEG
    let mut output = Cursor::new(Vec::new());
    thumbnail.write_to(&mut output, image::ImageFormat::Jpeg)?;

    Ok(output.into_inner())
}

async fn generate_video_thumbnail(
    data: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    // Write video to temp file (ffmpeg needs file input for seeking)
    let temp_dir = std::env::temp_dir();
    let input_path = temp_dir.join(format!("cleo_thumb_input_{}.tmp", rand::random::<u64>()));
    let output_path = temp_dir.join(format!("cleo_thumb_output_{}.jpg", rand::random::<u64>()));

    tokio::fs::write(&input_path, data).await.map_err(|e| {
        format!("Failed to write temp input file {:?}: {}", input_path, e)
    })?;

    // Extract frame at 1 second (or first frame if video is shorter)
    // Note: -update 1 is required for newer ffmpeg versions when writing a single image
    let output = Command::new("ffmpeg")
        .args([
            "-i",
            input_path.to_str().unwrap(),
            "-ss",
            "00:00:01",
            "-vframes",
            "1",
            "-vf",
            &format!("scale={}:-1", THUMBNAIL_WIDTH),
            "-q:v",
            &THUMBNAIL_QUALITY.to_string(),
            "-update",
            "1",
            "-y",
            output_path.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to spawn ffmpeg: {}", e))?;

    // If seeking to 1s failed (video too short), try first frame
    if !output.status.success() || !output_path.exists() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("[thumbnails] ffmpeg first attempt failed (trying without seek): {}", stderr);

        let retry_output = Command::new("ffmpeg")
            .args([
                "-i",
                input_path.to_str().unwrap(),
                "-vframes",
                "1",
                "-vf",
                &format!("scale={}:-1", THUMBNAIL_WIDTH),
                "-q:v",
                &THUMBNAIL_QUALITY.to_string(),
                "-update",
                "1",
                "-y",
                output_path.to_str().unwrap(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("Failed to spawn ffmpeg (retry): {}", e))?;

        if !retry_output.status.success() {
            let stderr = String::from_utf8_lossy(&retry_output.stderr);
            // Cleanup input before returning error
            if let Err(e) = tokio::fs::remove_file(&input_path).await {
                eprintln!("Failed to cleanup temp file {}: {}", input_path.display(), e);
            }
            return Err(format!("ffmpeg failed: {}", stderr).into());
        }
    }

    // Read output
    let thumbnail_data = tokio::fs::read(&output_path).await.map_err(|e| {
        format!("Failed to read ffmpeg output {:?}: {}", output_path, e)
    })?;

    // Cleanup temp files (log failures but don't error - thumbnail was generated successfully)
    if let Err(e) = tokio::fs::remove_file(&input_path).await {
        eprintln!("Failed to cleanup temp file {}: {}", input_path.display(), e);
    }
    if let Err(e) = tokio::fs::remove_file(&output_path).await {
        eprintln!("Failed to cleanup temp file {}: {}", output_path.display(), e);
    }

    Ok(thumbnail_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thumbnail_path_generation() {
        assert_eq!(
            get_thumbnail_path("image/user_1/2025-01-01/123456.png"),
            "thumbnails/user_1/2025-01-01/123456.jpg"
        );
        assert_eq!(
            get_thumbnail_path("video/user_1/2025-01-01/123456.mp4"),
            "thumbnails/user_1/2025-01-01/123456.jpg"
        );
    }
}
