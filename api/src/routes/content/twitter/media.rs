//! Shared media upload utilities for Twitter routes

use std::sync::Arc;
use tokio::sync::mpsc;
use crate::constants::BUCKET_NAME;
use crate::domain::captures;
use crate::domain::twitter::TweetForPosting;
use crate::AppState;

/// Progress message for WebSocket
#[derive(Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum UploadProgress {
    #[serde(rename = "uploading")]
    Uploading { segment: usize, total: usize, percent: u8 },
    #[serde(rename = "processing")]
    Processing,
}

/// Upload media for a tweet and return Twitter media IDs
pub async fn upload_tweet_media(
    state: &Arc<AppState>,
    user_id: i64,
    tweet: &TweetForPosting,
    access_token: &str,
) -> Result<Vec<String>, String> {
    let mut media_ids = Vec::new();

    // Handle video clip (mutually exclusive with images on Twitter)
    if let Some(ref video_clip) = tweet.video_clip {
        let capture_id = video_clip
            .get("source_capture_id")
            .and_then(|v| v.as_i64())
            .ok_or("Invalid video_clip format")?;

        let (data, content_type) = fetch_capture_data(state, user_id, capture_id).await?;

        let media_id = state
            .twitter
            .upload_media(access_token, &data, &content_type)
            .await
            .map_err(|e| format!("Failed to upload video: {}", e))?;

        media_ids.push(media_id);
    } else if !tweet.image_capture_ids.is_empty() {
        // Upload images (max 4 per Twitter rules)
        let capture_ids: Vec<i64> = tweet.image_capture_ids.iter().take(4).copied().collect();

        // Batch fetch all capture metadata in one query
        let captures = fetch_captures_batch(state, user_id, &capture_ids).await?;

        for capture_id in &capture_ids {
            let capture_info = captures
                .get(capture_id)
                .ok_or_else(|| format!("Capture {} not found", capture_id))?;

            let data = fetch_capture_data_from_path(state, &capture_info.gcs_path).await?;

            let media_id = state
                .twitter
                .upload_media(access_token, &data, &capture_info.content_type)
                .await
                .map_err(|e| format!("Failed to upload image {}: {}", capture_id, e))?;

            media_ids.push(media_id);
        }
    }

    Ok(media_ids)
}

/// Upload media for a tweet with progress reporting via channel
pub async fn upload_tweet_media_with_progress<T: From<UploadProgress> + Send + 'static>(
    state: &Arc<AppState>,
    user_id: i64,
    tweet: &TweetForPosting,
    access_token: &str,
    progress_tx: mpsc::Sender<T>,
) -> Result<Vec<String>, String> {
    let mut media_ids = Vec::new();

    // Handle video clip (mutually exclusive with images on Twitter)
    if let Some(ref video_clip) = tweet.video_clip {
        let capture_id = video_clip
            .get("source_capture_id")
            .and_then(|v| v.as_i64())
            .ok_or("Invalid video_clip format")?;

        let (data, content_type) = fetch_capture_data(state, user_id, capture_id).await?;

        // For videos, use chunked upload with progress
        if content_type.starts_with("video/") {
            let progress_tx_clone = progress_tx.clone();
            let media_id = state
                .twitter
                .upload_media_chunked_with_progress(
                    access_token,
                    &data,
                    &content_type,
                    move |segment, total| {
                        let percent = if total > 0 {
                            ((segment as f32 / total as f32) * 100.0) as u8
                        } else {
                            0
                        };
                        let _ = progress_tx_clone.try_send(UploadProgress::Uploading {
                            segment,
                            total,
                            percent,
                        }.into());
                    },
                )
                .await
                .map_err(|e| format!("Failed to upload video: {}", e))?;

            // Send processing status
            let _ = progress_tx.send(UploadProgress::Processing.into()).await;

            media_ids.push(media_id);
        } else {
            let media_id = state
                .twitter
                .upload_media(access_token, &data, &content_type)
                .await
                .map_err(|e| format!("Failed to upload media: {}", e))?;
            media_ids.push(media_id);
        }
    } else if !tweet.image_capture_ids.is_empty() {
        // Upload images (max 4 per Twitter rules)
        let capture_ids: Vec<i64> = tweet.image_capture_ids.iter().take(4).copied().collect();
        let total_images = capture_ids.len();

        // Batch fetch all capture metadata in one query
        let captures = fetch_captures_batch(state, user_id, &capture_ids).await?;

        for (idx, capture_id) in capture_ids.iter().enumerate() {
            // Send progress for image uploads
            let percent = ((idx as f32 / total_images as f32) * 100.0) as u8;
            let _ = progress_tx.send(UploadProgress::Uploading {
                segment: idx,
                total: total_images,
                percent,
            }.into()).await;

            let capture_info = captures
                .get(capture_id)
                .ok_or_else(|| format!("Capture {} not found", capture_id))?;

            let data = fetch_capture_data_from_path(state, &capture_info.gcs_path).await?;

            let media_id = state
                .twitter
                .upload_media(access_token, &data, &capture_info.content_type)
                .await
                .map_err(|e| format!("Failed to upload image {}: {}", capture_id, e))?;

            media_ids.push(media_id);
        }
    }

    Ok(media_ids)
}

/// Fetch capture data from local storage or GCS
pub async fn fetch_capture_data(
    state: &Arc<AppState>,
    user_id: i64,
    capture_id: i64,
) -> Result<(Vec<u8>, String), String> {
    // Get capture info from domain layer
    let capture = captures::get_capture_info(&state.db, capture_id, user_id)
        .await
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| format!("Capture {} not found", capture_id))?;

    let (gcs_path, content_type) = (capture.gcs_path, capture.content_type);

    // Read from local storage or GCS
    let data = if let Some(local_path) = &state.local_storage_path {
        let full_path = local_path.join(&gcs_path);
        tokio::fs::read(&full_path)
            .await
            .map_err(|e| format!("Failed to read local file {:?}: {}", full_path, e))?
    } else {
        // Download from GCS
        let bucket = format!("projects/_/buckets/{}", BUCKET_NAME);
        let mut resp = state.gcs.read_object(&bucket, &gcs_path)
            .send()
            .await
            .map_err(|e| format!("GCS read error: {}", e))?;

        let mut data = Vec::new();
        while let Some(chunk) = resp.next().await {
            let bytes = chunk.map_err(|e| format!("GCS stream error: {}", e))?;
            data.extend_from_slice(&bytes);
        }
        data
    };

    Ok((data, content_type))
}

/// Capture metadata for batch operations (re-export from domain)
pub use crate::domain::captures::CaptureInfo;

/// Batch fetch capture metadata for multiple capture IDs (single query)
pub async fn fetch_captures_batch(
    state: &Arc<AppState>,
    user_id: i64,
    capture_ids: &[i64],
) -> Result<std::collections::HashMap<i64, CaptureInfo>, String> {
    captures::get_captures_batch(&state.db, capture_ids, user_id)
        .await
        .map_err(|e| format!("DB error: {}", e))
}

/// Fetch capture data from a known path (local storage or GCS)
pub async fn fetch_capture_data_from_path(
    state: &Arc<AppState>,
    gcs_path: &str,
) -> Result<Vec<u8>, String> {
    if let Some(local_path) = &state.local_storage_path {
        let full_path = local_path.join(gcs_path);
        tokio::fs::read(&full_path)
            .await
            .map_err(|e| format!("Failed to read local file {:?}: {}", full_path, e))
    } else {
        // Download from GCS
        let bucket = format!("projects/_/buckets/{}", BUCKET_NAME);
        let mut resp = state.gcs.read_object(&bucket, gcs_path)
            .send()
            .await
            .map_err(|e| format!("GCS read error: {}", e))?;

        let mut data = Vec::new();
        while let Some(chunk) = resp.next().await {
            let bytes = chunk.map_err(|e| format!("GCS stream error: {}", e))?;
            data.extend_from_slice(&bytes);
        }
        Ok(data)
    }
}
