//! Thumbnail generation background job using apalis
//!
//! Runs as a scheduled cron job that batch-processes captures without thumbnails.

use apalis::prelude::*;
use apalis_cron::{CronStream, Schedule};
use apalis_sql::postgres::PostgresStorage;
use image::ImageReader;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::env;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Stdio;
use std::str::FromStr;
use tokio::process::Command;

use crate::models::CaptureForThumbnail;
use crate::storage;

const THUMBNAIL_WIDTH: u32 = 300;
const THUMBNAIL_QUALITY: u8 = 80;
const MAX_ATTEMPTS: i32 = 5;
const CLAIM_BATCH_SIZE: i64 = 64;
const DEFAULT_CONCURRENCY: usize = 12;
const DEFAULT_CRON_SECONDS: u64 = 5;
const DEFAULT_LEASE_SECONDS: i64 = 900;
const DEFAULT_FFMPEG_THREADS: usize = 1;

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
    pub gcs: Option<google_cloud_storage::client::Storage>,
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
                println!(
                    "[thumbnails] Batch complete: {} processed, {} failed",
                    processed, failed
                );
            }
        }
        Err(e) => {
            eprintln!("[thumbnails] Batch error (will retry): {}", e);
        }
    }
    Ok(())
}

/// Start the thumbnail worker
pub async fn run_thumbnail_worker(
    pool: PgPool,
    gcs: Option<google_cloud_storage::client::Storage>,
    local_storage_path: Option<PathBuf>,
    bucket_name: String,
) {
    let ctx = ThumbnailContext {
        pool: pool.clone(),
        gcs,
        local_storage_path,
        bucket_name,
    };

    let cron_seconds = thumbnail_cron_seconds();
    let concurrency = thumbnail_concurrency();
    let lease_seconds = thumbnail_lease_seconds();
    let schedule_expr = format!("*/{} * * * * *", cron_seconds);

    // Run apalis migrations
    PostgresStorage::setup(&pool)
        .await
        .expect("Failed to set up apalis storage");

    let storage: PostgresStorage<ThumbnailJob> = PostgresStorage::new(pool.clone());
    let schedule = Schedule::from_str(&schedule_expr).expect("Invalid thumbnail worker schedule");
    let cron = CronStream::new(schedule);
    let backend = cron.pipe_to_storage(storage);

    println!(
        "[thumbnails] Apalis worker starting (every {}s, {} concurrency, {}s lease)",
        cron_seconds, concurrency, lease_seconds
    );

    let worker = WorkerBuilder::new("thumbnail-worker")
        .data(ctx)
        .backend(backend)
        .build_fn(process_thumbnail_job);

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
    let mut processed = 0;
    let mut failed = 0;
    let concurrency = thumbnail_concurrency();
    let lease_seconds = thumbnail_lease_seconds();

    let mut tasks = tokio::task::JoinSet::new();
    let mut claim_failed = false;

    loop {
        let needed = concurrency.saturating_sub(tasks.len());
        if needed > 0 && !claim_failed {
            let claim_limit = std::cmp::min(CLAIM_BATCH_SIZE, needed as i64);
            let captures =
                match claim_thumbnail_captures(&ctx.pool, claim_limit, lease_seconds).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[thumbnails] Claim error: {}", e);
                        claim_failed = true;
                        Vec::new()
                    }
                };

            for capture in captures {
                let pool = ctx.pool.clone();
                let gcs = ctx.gcs.clone();
                let local_path = ctx.local_storage_path.clone();
                let bucket = ctx.bucket_name.clone();

                tasks.spawn(async move {
                    let data = match storage::download_capture(
                        gcs.as_ref(),
                        local_path.as_ref(),
                        &bucket,
                        &capture.gcs_path,
                    )
                    .await
                    {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            eprintln!(
                                "[thumbnails] Failed to download capture {} ({}): {}",
                                capture.id, capture.gcs_path, e
                            );
                            increment_attempts(&pool, &capture).await;
                            return Err(());
                        }
                    };

                    let result = process_single_capture(
                        &pool,
                        gcs.as_ref(),
                        local_path.as_ref(),
                        &bucket,
                        &capture,
                        &data,
                    )
                    .await;

                    if let Err(ref e) = result {
                        eprintln!(
                            "[thumbnails] Failed capture {} type={} gcs_path={}: {}",
                            capture.id, capture.media_type, capture.gcs_path, e
                        );
                        increment_attempts(&pool, &capture).await;
                    }

                    result.map(|_| capture.id).map_err(|_| ())
                });
            }
        }

        if tasks.is_empty() {
            break;
        }

        if let Some(result) = tasks.join_next().await {
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
    }

    Ok((processed, failed))
}

async fn increment_attempts(pool: &PgPool, capture: &CaptureForThumbnail) {
    if let Err(db_err) = sqlx::query(
        "UPDATE captures
         SET thumbnail_attempts = thumbnail_attempts + 1,
             thumbnail_processing = FALSE,
             thumbnail_processing_started_at = NULL
         WHERE id = $1 AND captured_at = $2",
    )
    .bind(capture.id)
    .bind(capture.captured_at)
    .execute(pool)
    .await
    {
        eprintln!(
            "[thumbnails] CRITICAL: Failed to increment retry counter for capture {}: {}",
            capture.id, db_err
        );
    }
}

async fn process_single_capture(
    pool: &PgPool,
    gcs: Option<&google_cloud_storage::client::Storage>,
    local_storage_path: Option<&PathBuf>,
    bucket_name: &str,
    capture: &CaptureForThumbnail,
    data: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let thumbnail_data = if capture.media_type == "video" {
        generate_video_thumbnail(data).await?
    } else {
        generate_image_thumbnail(data)?
    };

    let thumbnail_path = get_thumbnail_path(&capture.gcs_path);
    storage::upload_data(
        gcs,
        local_storage_path,
        bucket_name,
        &thumbnail_path,
        &thumbnail_data,
    )
    .await?;

    let db_result = sqlx::query(
        "UPDATE captures
         SET thumbnail_path = $1,
             thumbnail_processing = FALSE,
             thumbnail_processing_started_at = NULL
         WHERE id = $2 AND captured_at = $3",
    )
    .bind(&thumbnail_path)
    .bind(capture.id)
    .bind(capture.captured_at)
    .execute(pool)
    .await;

    if let Err(e) = db_result {
        if let Err(cleanup_err) =
            delete_thumbnail(gcs, local_storage_path, bucket_name, &thumbnail_path).await
        {
            eprintln!(
                "[thumbnails] Failed to clean up orphaned thumbnail {}: {}",
                thumbnail_path, cleanup_err
            );
        } else {
            eprintln!(
                "[thumbnails] Cleaned up orphaned thumbnail: {}",
                thumbnail_path
            );
        }
        return Err(Box::new(e));
    }

    Ok(())
}

/// Delete a thumbnail from storage (for cleanup on DB failure)
async fn delete_thumbnail(
    _gcs: Option<&google_cloud_storage::client::Storage>,
    local_storage_path: Option<&PathBuf>,
    bucket_name: &str,
    thumbnail_path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(local_path) = local_storage_path {
        let full_path = local_path.join(thumbnail_path);
        tokio::fs::remove_file(&full_path).await?;
    } else {
        let client = cloud_storage::Client::default();
        client.object().delete(bucket_name, thumbnail_path).await?;
    }
    Ok(())
}

fn get_thumbnail_path(original_path: &str) -> String {
    let path = std::path::Path::new(original_path);
    let components: Vec<_> = path.components().collect();
    if components.len() < 2 {
        return format!("thumbnails/{}.jpg", original_path);
    }
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

    let thumbnail = img.thumbnail(THUMBNAIL_WIDTH, THUMBNAIL_WIDTH * 2);

    let mut output = Cursor::new(Vec::new());
    thumbnail.write_to(&mut output, image::ImageFormat::Jpeg)?;

    Ok(output.into_inner())
}

async fn generate_video_thumbnail(
    data: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let ffmpeg_threads = ffmpeg_threads().to_string();
    let temp_dir = std::env::temp_dir();
    let input_path = temp_dir.join(format!("cleo_thumb_input_{}.tmp", rand::random::<u64>()));
    let output_path = temp_dir.join(format!("cleo_thumb_output_{}.jpg", rand::random::<u64>()));

    tokio::fs::write(&input_path, data)
        .await
        .map_err(|e| format!("Failed to write temp input file {:?}: {}", input_path, e))?;

    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(["-threads", &ffmpeg_threads])
        .args(["-ss", "00:00:01"])
        .args(["-i", input_path.to_str().unwrap()])
        .args(["-an", "-sn"])
        .args(["-frames:v", "1"])
        .args(["-vf", &format!("scale={}:-1", THUMBNAIL_WIDTH)])
        .args(["-q:v", &THUMBNAIL_QUALITY.to_string()])
        .args(["-y", output_path.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to spawn ffmpeg: {}", e))?;

    // If seeking to 1s failed (video too short), try first frame
    if !output.status.success() || !output_path.exists() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!(
            "[thumbnails] ffmpeg first attempt failed (trying without seek): {}",
            stderr
        );

        let retry_output = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(["-threads", &ffmpeg_threads])
            .args(["-i", input_path.to_str().unwrap()])
            .args(["-an", "-sn"])
            .args(["-frames:v", "1"])
            .args(["-vf", &format!("scale={}:-1", THUMBNAIL_WIDTH)])
            .args(["-q:v", &THUMBNAIL_QUALITY.to_string()])
            .args(["-y", output_path.to_str().unwrap()])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("Failed to spawn ffmpeg (retry): {}", e))?;

        if !retry_output.status.success() {
            let stderr = String::from_utf8_lossy(&retry_output.stderr);
            if let Err(e) = tokio::fs::remove_file(&input_path).await {
                eprintln!(
                    "Failed to cleanup temp file {}: {}",
                    input_path.display(),
                    e
                );
            }
            return Err(format!("ffmpeg failed: {}", stderr).into());
        }
    }

    let thumbnail_data = tokio::fs::read(&output_path)
        .await
        .map_err(|e| format!("Failed to read ffmpeg output {:?}: {}", output_path, e))?;

    if let Err(e) = tokio::fs::remove_file(&input_path).await {
        eprintln!(
            "Failed to cleanup temp file {}: {}",
            input_path.display(),
            e
        );
    }
    if let Err(e) = tokio::fs::remove_file(&output_path).await {
        eprintln!(
            "Failed to cleanup temp file {}: {}",
            output_path.display(),
            e
        );
    }

    Ok(thumbnail_data)
}

async fn claim_thumbnail_captures(
    pool: &PgPool,
    limit: i64,
    lease_seconds: i64,
) -> Result<Vec<CaptureForThumbnail>, sqlx::Error> {
    sqlx::query_as(
        r#"
        WITH claimed AS (
            SELECT id, captured_at
            FROM captures
            WHERE thumbnail_path IS NULL
              AND thumbnail_attempts < $1
              AND (
                  thumbnail_processing = FALSE
                  OR (
                      thumbnail_processing = TRUE
                      AND thumbnail_processing_started_at IS NOT NULL
                      AND thumbnail_processing_started_at < NOW() - ($2::text || ' seconds')::interval
                  )
              )
            ORDER BY captured_at ASC
            LIMIT $3
            FOR UPDATE SKIP LOCKED
        )
        UPDATE captures c
        SET thumbnail_processing = TRUE,
            thumbnail_processing_started_at = NOW()
        FROM claimed
        WHERE c.id = claimed.id
          AND c.captured_at = claimed.captured_at
        RETURNING c.id, c.media_type, c.gcs_path, c.captured_at
        "#,
    )
    .bind(MAX_ATTEMPTS)
    .bind(lease_seconds)
    .bind(limit)
    .fetch_all(pool)
    .await
}

fn thumbnail_concurrency() -> usize {
    env::var("THUMBNAIL_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_CONCURRENCY)
}

fn thumbnail_cron_seconds() -> u64 {
    env::var("THUMBNAIL_CRON_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v| *v > 0 && *v <= 59)
        .unwrap_or(DEFAULT_CRON_SECONDS)
}

fn thumbnail_lease_seconds() -> i64 {
    env::var("THUMBNAIL_LEASE_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_LEASE_SECONDS)
}

fn ffmpeg_threads() -> usize {
    env::var("FFMPEG_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_FFMPEG_THREADS)
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
