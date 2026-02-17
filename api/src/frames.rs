//! Frame extraction background worker
//!
//! Extracts frames from video captures and screenshots, deduplicates with pHash,
//! saves half-resolution versions for the agent pipeline.

use image::ImageReader;
use image_hasher::{HashAlg, HasherConfig, ImageHash};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::env;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

use crate::models::CaptureForThumbnail;
use crate::storage;

const MAX_ATTEMPTS: i32 = 5;
const DEFAULT_CONCURRENCY: usize = 12;
const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
const DEFAULT_LEASE_SECS: i64 = 900;
const DEFAULT_FFMPEG_THREADS: usize = 1;
const HALF_RES_WIDTH: u32 = 960;
const HALF_RES_HEIGHT: u32 = 540;
const PHASH_DISTANCE_THRESHOLD: u32 = 10;

/// Frame metadata within a manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameEntry {
    pub index: usize,
    pub filename: String,
    pub timestamp_secs: f64,
    pub phash: String,
}

/// Manifest file stored alongside extracted frames
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameManifest {
    pub capture_id: i64,
    pub media_type: String,
    pub frame_count: usize,
    pub duration_secs: Option<f64>,
    pub frames: Vec<FrameEntry>,
}

/// Start the frame extraction worker.
/// Poll interval, concurrency, and lease TTL are env-configurable.
pub async fn run_frame_worker(
    pool: PgPool,
    gcs: Option<google_cloud_storage::client::Storage>,
    local_storage_path: Option<PathBuf>,
    bucket_name: String,
) {
    let concurrency = frame_worker_concurrency();
    let poll_interval_secs = frame_poll_interval_secs();
    let lease_secs = frame_lease_secs();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(poll_interval_secs));

    println!(
        "[frames] Worker starting ({}s poll, {} concurrency, {}s lease)",
        poll_interval_secs, concurrency, lease_secs
    );

    loop {
        interval.tick().await;

        let mut total_processed = 0;
        let mut total_failed = 0;
        let mut tasks = tokio::task::JoinSet::new();
        let mut claim_failed = false;

        // Keep concurrency tasks in flight at all times, refill as each completes.
        // If claiming fails, drain in-flight tasks before ending the cycle.
        loop {
            let needed = concurrency.saturating_sub(tasks.len());
            if needed > 0 && !claim_failed {
                let captures = match claim_frame_captures(&pool, needed as i64, lease_secs).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[frames] Claim error: {}", e);
                        claim_failed = true;
                        Vec::new()
                    }
                };

                for capture in captures {
                    let pool = pool.clone();
                    let gcs = gcs.clone();
                    let local_path = local_storage_path.clone();
                    let bucket = bucket_name.clone();

                    tasks.spawn(async move {
                        let result = process_capture(
                            &pool,
                            gcs.as_ref(),
                            local_path.as_ref(),
                            &bucket,
                            &capture,
                        )
                        .await;

                        match &result {
                            Ok(()) => {
                                println!("[frames] Extracted frames for capture {}", capture.id);
                            }
                            Err(e) => {
                                eprintln!(
                                    "[frames] Failed capture {} type={}: {}",
                                    capture.id, capture.media_type, e
                                );
                                // Increment attempts on actual failure
                                let _ = sqlx::query(
                                    "UPDATE captures
                                     SET frames_processing = FALSE,
                                         frames_processing_started_at = NULL,
                                         frame_attempts = frame_attempts + 1
                                     WHERE id = $1 AND captured_at = $2",
                                )
                                .bind(capture.id)
                                .bind(capture.captured_at)
                                .execute(&pool)
                                .await;
                            }
                        }

                        (capture.id, result.is_ok())
                    });
                }
            }

            if tasks.is_empty() {
                break;
            }

            if let Some(result) = tasks.join_next().await {
                match result {
                    Ok((_capture_id, true)) => total_processed += 1,
                    Ok((_capture_id, false)) => total_failed += 1,
                    Err(e) => {
                        eprintln!("[frames] Task panicked: {}", e);
                        total_failed += 1;
                    }
                }
            }
        }

        if total_processed > 0 || total_failed > 0 {
            println!(
                "[frames] Cycle complete: {} processed, {} failed",
                total_processed, total_failed
            );
        }
    }
}

/// Download, extract frames, upload, update DB — all in one.
/// For videos: downloads to temp file, drops the bytes, then processes from disk.
/// For images: small enough to hold in memory.
async fn process_capture(
    pool: &PgPool,
    gcs: Option<&google_cloud_storage::client::Storage>,
    local_storage_path: Option<&PathBuf>,
    bucket_name: &str,
    capture: &CaptureForThumbnail,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let hasher = HasherConfig::new()
        .hash_alg(HashAlg::Mean)
        .hash_size(8, 8)
        .to_hasher();

    let frames_dir = get_frames_dir(&capture.gcs_path);

    let (frame_count, _duration_secs) = if capture.media_type == "video" {
        // Download video to temp file, then drop the bytes
        let temp_dir = std::env::temp_dir().join(format!("cleo_frames_{}", rand::random::<u64>()));
        tokio::fs::create_dir_all(&temp_dir).await?;
        let input_path = temp_dir.join("input.mp4");

        {
            let data =
                storage::download_capture(gcs, local_storage_path, bucket_name, &capture.gcs_path)
                    .await?;
            tokio::fs::write(&input_path, &data).await?;
            // data dropped here — video bytes freed
        }

        let result = extract_and_upload_video_frames(
            &input_path,
            &temp_dir,
            &hasher,
            &frames_dir,
            gcs,
            local_storage_path,
            bucket_name,
        )
        .await;

        cleanup_temp_dir(&temp_dir).await;
        result?
    } else {
        let data =
            storage::download_capture(gcs, local_storage_path, bucket_name, &capture.gcs_path)
                .await?;
        extract_and_upload_image_frame(
            &data,
            &hasher,
            &frames_dir,
            gcs,
            local_storage_path,
            bucket_name,
        )
        .await?
    };

    if frame_count == 0 {
        return Err("No frames extracted".into());
    }

    // Update DB
    sqlx::query(
        "UPDATE captures
         SET frames_extracted = TRUE,
             frames_processing = FALSE,
             frames_processing_started_at = NULL,
             frame_count = $1
         WHERE id = $2 AND captured_at = $3",
    )
    .bind(frame_count as i32)
    .bind(capture.id)
    .bind(capture.captured_at)
    .execute(pool)
    .await
    .map_err(|e| {
        eprintln!(
            "[frames] DB update failed for capture {}: {}",
            capture.id, e
        );
        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
    })?;

    Ok(())
}

/// Extract frames from a video, dedup with pHash, upload each frame immediately.
/// Returns (frame_count, duration_secs). No frame data accumulates in memory.
async fn extract_and_upload_video_frames(
    input_path: &PathBuf,
    temp_dir: &PathBuf,
    hasher: &image_hasher::Hasher,
    frames_dir: &str,
    gcs: Option<&google_cloud_storage::client::Storage>,
    local_storage_path: Option<&PathBuf>,
    bucket_name: &str,
) -> Result<(usize, Option<f64>), Box<dyn std::error::Error + Send + Sync>> {
    let ffmpeg_threads = ffmpeg_threads().to_string();

    // Get video duration
    let probe_output = Command::new("ffprobe")
        .args(["-v", "error"])
        .args(["-show_entries", "format=duration"])
        .args(["-of", "default=noprint_wrappers=1:nokey=1"])
        .arg(input_path.to_str().unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await?;

    let duration_secs = String::from_utf8_lossy(&probe_output.stdout)
        .trim()
        .parse::<f64>()
        .ok();

    // Extract frames at 1fps, already scaled to half-res by ffmpeg
    let vf = format!("fps=1,scale={}:{}", HALF_RES_WIDTH, HALF_RES_HEIGHT);
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(["-threads", &ffmpeg_threads])
        .args(["-i", input_path.to_str().unwrap()])
        .args(["-an", "-sn"])
        .args(["-vf", &vf])
        .args(["-q:v", "4"])
        .args(["-y", temp_dir.join("frame_%04d.jpg").to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg frame extraction failed: {}", stderr).into());
    }

    // Collect frame file paths
    let mut frame_files: Vec<PathBuf> = Vec::new();
    let mut entries = tokio::fs::read_dir(temp_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().map(|e| e == "jpg").unwrap_or(false)
            && path
                .file_name()
                .map(|n| n.to_string_lossy().starts_with("frame_"))
                .unwrap_or(false)
        {
            frame_files.push(path);
        }
    }
    frame_files.sort();

    // Process one frame at a time: dedup, upload, drop — never hold more than one in memory
    let mut manifest_frames: Vec<FrameEntry> = Vec::new();
    let mut last_hash: Option<ImageHash> = None;
    let mut kept = 0usize;

    for (i, frame_path) in frame_files.iter().enumerate() {
        let frame_data = tokio::fs::read(frame_path).await?;
        let img = match ImageReader::new(Cursor::new(&frame_data))
            .with_guessed_format()?
            .decode()
        {
            Ok(img) => img,
            Err(e) => {
                eprintln!("[frames] Failed to decode frame {}: {}", i, e);
                continue;
            }
        };

        let current_hash = hasher.hash_image(&img);
        drop(img); // free decoded image immediately

        if let Some(ref prev_hash) = last_hash {
            if prev_hash.dist(&current_hash) <= PHASH_DISTANCE_THRESHOLD {
                continue;
            }
        }

        last_hash = Some(current_hash.clone());
        let timestamp_secs = i as f64;

        // Upload immediately, then frame_data is dropped
        let filename = format!("frame_{}.jpg", kept);
        let frame_path = format!("{}/{}", frames_dir, filename);
        storage::upload_data(
            gcs,
            local_storage_path,
            bucket_name,
            &frame_path,
            &frame_data,
        )
        .await?;

        manifest_frames.push(FrameEntry {
            index: kept,
            filename,
            timestamp_secs,
            phash: current_hash.to_base64(),
        });
        kept += 1;
    }

    // Write manifest
    let manifest = FrameManifest {
        capture_id: 0, // filled by caller via DB update
        media_type: "video".to_string(),
        frame_count: kept,
        duration_secs,
        frames: manifest_frames,
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    let manifest_path = format!("{}/manifest.json", frames_dir);
    storage::upload_data(
        gcs,
        local_storage_path,
        bucket_name,
        &manifest_path,
        manifest_json.as_bytes(),
    )
    .await?;

    Ok((kept, duration_secs))
}

/// Process a screenshot: resize to half-res, hash, upload immediately.
async fn extract_and_upload_image_frame(
    data: &[u8],
    hasher: &image_hasher::Hasher,
    frames_dir: &str,
    gcs: Option<&google_cloud_storage::client::Storage>,
    local_storage_path: Option<&PathBuf>,
    bucket_name: &str,
) -> Result<(usize, Option<f64>), Box<dyn std::error::Error + Send + Sync>> {
    let img = ImageReader::new(Cursor::new(data))
        .with_guessed_format()?
        .decode()?;

    let half_res = img.resize_exact(
        HALF_RES_WIDTH,
        HALF_RES_HEIGHT,
        image::imageops::FilterType::Triangle,
    );
    drop(img);

    let hash = hasher.hash_image(&half_res);

    let mut output_buf = Cursor::new(Vec::new());
    half_res.write_to(&mut output_buf, image::ImageFormat::Jpeg)?;
    drop(half_res);

    let frame_data = output_buf.into_inner();
    let filename = "frame_0.jpg".to_string();
    let frame_path = format!("{}/{}", frames_dir, filename);
    storage::upload_data(
        gcs,
        local_storage_path,
        bucket_name,
        &frame_path,
        &frame_data,
    )
    .await?;

    let manifest = FrameManifest {
        capture_id: 0,
        media_type: "image".to_string(),
        frame_count: 1,
        duration_secs: None,
        frames: vec![FrameEntry {
            index: 0,
            filename,
            timestamp_secs: 0.0,
            phash: hash.to_base64(),
        }],
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    let manifest_path = format!("{}/manifest.json", frames_dir);
    storage::upload_data(
        gcs,
        local_storage_path,
        bucket_name,
        &manifest_path,
        manifest_json.as_bytes(),
    )
    .await?;

    Ok((1, None))
}

/// Convert gcs_path to frames directory path
/// e.g. "video/user_1/2025-01-01/123.mp4" -> "frames/user_1/2025-01-01/123"
pub fn get_frames_dir(gcs_path: &str) -> String {
    let path = std::path::Path::new(gcs_path);
    let components: Vec<_> = path.components().collect();
    if components.len() < 2 {
        return format!("frames/{}", gcs_path);
    }
    let rest: PathBuf = components[1..].iter().collect();
    let stem = rest.file_stem().unwrap_or_default().to_string_lossy();
    let parent = rest.parent().unwrap_or(std::path::Path::new(""));
    format!("frames/{}/{}", parent.display(), stem)
}

async fn cleanup_temp_dir(temp_dir: &PathBuf) {
    if let Err(e) = tokio::fs::remove_dir_all(temp_dir).await {
        eprintln!("[frames] Failed to cleanup temp dir {:?}: {}", temp_dir, e);
    }
}

async fn claim_frame_captures(
    pool: &PgPool,
    limit: i64,
    lease_secs: i64,
) -> Result<Vec<CaptureForThumbnail>, sqlx::Error> {
    sqlx::query_as(
        r#"
        WITH claimed AS (
            SELECT id, captured_at
            FROM captures
            WHERE frames_extracted = FALSE
              AND frame_attempts < $1
              AND (
                  frames_processing = FALSE
                  OR (
                      frames_processing = TRUE
                      AND frames_processing_started_at IS NOT NULL
                      AND frames_processing_started_at < NOW() - ($2::text || ' seconds')::interval
                  )
              )
            ORDER BY captured_at ASC
            LIMIT $3
            FOR UPDATE SKIP LOCKED
        )
        UPDATE captures c
        SET frames_processing = TRUE,
            frames_processing_started_at = NOW()
        FROM claimed
        WHERE c.id = claimed.id
          AND c.captured_at = claimed.captured_at
        RETURNING c.id, c.media_type, c.gcs_path, c.captured_at
        "#,
    )
    .bind(MAX_ATTEMPTS)
    .bind(lease_secs)
    .bind(limit)
    .fetch_all(pool)
    .await
}

fn frame_worker_concurrency() -> usize {
    env::var("FRAME_WORKER_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_CONCURRENCY)
}

fn frame_poll_interval_secs() -> u64 {
    env::var("FRAME_POLL_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_POLL_INTERVAL_SECS)
}

fn frame_lease_secs() -> i64 {
    env::var("FRAME_LEASE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_LEASE_SECS)
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
    fn test_frames_dir_generation() {
        assert_eq!(
            get_frames_dir("video/user_1/2025-01-01/123456.mp4"),
            "frames/user_1/2025-01-01/123456"
        );
        assert_eq!(
            get_frames_dir("image/user_1/2025-01-01/789.png"),
            "frames/user_1/2025-01-01/789"
        );
    }
}
