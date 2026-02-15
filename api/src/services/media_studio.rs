//! Media Studio service - core editing functions for trim/crop operations.
//!
//! Used by:
//! - Web UI via WebSocket for interactive editing
//! - Agent for automated media suggestions

use bytes::Bytes;
use chrono::Utc;
use google_cloud_storage::client::Storage;
use image::ImageReader;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

use crate::constants::BUCKET_NAME;
use crate::domain::captures;
use crate::get_extension;

/// Error types for media studio operations
#[derive(Debug)]
pub enum MediaStudioError {
    NotFound,
    InvalidMediaType(String),
    Storage(String),
    Processing(String),
    Database(sqlx::Error),
    InvalidParams(String),
}

impl std::fmt::Display for MediaStudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaStudioError::NotFound => write!(f, "Capture not found or access denied"),
            MediaStudioError::InvalidMediaType(s) => write!(f, "Invalid media type: {}", s),
            MediaStudioError::Storage(s) => write!(f, "Storage error: {}", s),
            MediaStudioError::Processing(s) => write!(f, "Processing error: {}", s),
            MediaStudioError::Database(e) => write!(f, "Database error: {}", e),
            MediaStudioError::InvalidParams(s) => write!(f, "Invalid parameters: {}", s),
        }
    }
}

impl std::error::Error for MediaStudioError {}

impl From<sqlx::Error> for MediaStudioError {
    fn from(e: sqlx::Error) -> Self {
        MediaStudioError::Database(e)
    }
}

/// Parameters for cropping an image (normalized 0.0-1.0)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropParams {
    /// X offset (0.0 = left edge, 1.0 = right edge)
    pub x: f64,
    /// Y offset (0.0 = top edge, 1.0 = bottom edge)
    pub y: f64,
    /// Width (0.0-1.0 of original)
    pub width: f64,
    /// Height (0.0-1.0 of original)
    pub height: f64,
}

impl CropParams {
    pub fn validate(&self) -> Result<(), MediaStudioError> {
        if self.x < 0.0 || self.x > 1.0 {
            return Err(MediaStudioError::InvalidParams("x must be between 0 and 1".into()));
        }
        if self.y < 0.0 || self.y > 1.0 {
            return Err(MediaStudioError::InvalidParams("y must be between 0 and 1".into()));
        }
        if self.width <= 0.0 || self.width > 1.0 {
            return Err(MediaStudioError::InvalidParams("width must be between 0 and 1".into()));
        }
        if self.height <= 0.0 || self.height > 1.0 {
            return Err(MediaStudioError::InvalidParams("height must be between 0 and 1".into()));
        }
        if self.x + self.width > 1.0 {
            return Err(MediaStudioError::InvalidParams("x + width exceeds image bounds".into()));
        }
        if self.y + self.height > 1.0 {
            return Err(MediaStudioError::InvalidParams("y + height exceeds image bounds".into()));
        }
        Ok(())
    }
}

/// Parameters for trimming a video
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrimParams {
    /// Start timestamp in HH:MM:SS or SS format
    pub start_timestamp: String,
    /// Duration in seconds
    pub duration_secs: f64,
}

impl TrimParams {
    pub fn validate(&self) -> Result<(), MediaStudioError> {
        if self.duration_secs <= 0.0 {
            return Err(MediaStudioError::InvalidParams("duration must be positive".into()));
        }
        // Validate timestamp format (simple check)
        if self.start_timestamp.is_empty() {
            return Err(MediaStudioError::InvalidParams("start_timestamp is required".into()));
        }
        Ok(())
    }
}

/// Edit parameters stored with derived captures
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EditParams {
    Crop(CropParams),
    Trim(TrimParams),
}

/// Media Studio service for editing operations
pub struct MediaStudio {
    db: PgPool,
    gcs: Option<Storage>,
    local_storage_path: Option<PathBuf>,
}

impl MediaStudio {
    pub fn new(db: PgPool, gcs: Option<Storage>, local_storage_path: Option<PathBuf>) -> Self {
        Self {
            db,
            gcs,
            local_storage_path,
        }
    }

    /// Crop an image capture, creating a new capture
    ///
    /// Returns the new capture ID
    pub async fn crop_image(
        &self,
        user_id: i64,
        source_capture_id: i64,
        crop: CropParams,
    ) -> Result<i64, MediaStudioError> {
        crop.validate()?;

        // 1. Verify user owns source capture and it's an image
        let source = captures::get_capture_info(&self.db, source_capture_id, user_id)
            .await?
            .ok_or(MediaStudioError::NotFound)?;

        if !source.content_type.starts_with("image/") {
            return Err(MediaStudioError::InvalidMediaType(format!(
                "Expected image, got {}",
                source.content_type
            )));
        }

        // 2. Download source image
        let data = self.download_capture(&source.gcs_path).await?;

        // 3. Apply crop
        let cropped_data = self.apply_image_crop(&data, &crop)?;

        // 4. Upload cropped image
        let extension = get_extension(&source.content_type);
        let new_path = self.generate_edited_path(user_id, "image", extension);
        self.upload_capture(&new_path, &cropped_data).await?;

        // 5. Create new capture record
        let edit_params = serde_json::to_value(EditParams::Crop(crop))
            .map_err(|e| MediaStudioError::Processing(e.to_string()))?;

        let new_id = self
            .insert_edited_capture(
                user_id,
                "image",
                &source.content_type,
                &new_path,
                source_capture_id,
                edit_params,
            )
            .await?;

        println!(
            "[media_studio] Cropped image {} -> {} for user {}",
            source_capture_id, new_id, user_id
        );

        Ok(new_id)
    }

    /// Trim a video capture, creating a new capture
    ///
    /// Returns the new capture ID
    pub async fn trim_video(
        &self,
        user_id: i64,
        source_capture_id: i64,
        trim: TrimParams,
    ) -> Result<i64, MediaStudioError> {
        trim.validate()?;

        // 1. Verify user owns source capture and it's a video
        let source = captures::get_capture_info(&self.db, source_capture_id, user_id)
            .await?
            .ok_or(MediaStudioError::NotFound)?;

        if !source.content_type.starts_with("video/") {
            return Err(MediaStudioError::InvalidMediaType(format!(
                "Expected video, got {}",
                source.content_type
            )));
        }

        // 2. Download source video
        let data = self.download_capture(&source.gcs_path).await?;

        // 3. Apply trim using ffmpeg
        let trimmed_data = self.apply_video_trim(&data, &trim).await?;

        // 4. Upload trimmed video
        let extension = get_extension(&source.content_type);
        let new_path = self.generate_edited_path(user_id, "video", extension);
        self.upload_capture(&new_path, &trimmed_data).await?;

        // 5. Create new capture record
        let edit_params = serde_json::to_value(EditParams::Trim(trim))
            .map_err(|e| MediaStudioError::Processing(e.to_string()))?;

        let new_id = self
            .insert_edited_capture(
                user_id,
                "video",
                &source.content_type,
                &new_path,
                source_capture_id,
                edit_params,
            )
            .await?;

        println!(
            "[media_studio] Trimmed video {} -> {} for user {}",
            source_capture_id, new_id, user_id
        );

        Ok(new_id)
    }

    // ============== Private helpers ==============

    async fn download_capture(&self, gcs_path: &str) -> Result<Vec<u8>, MediaStudioError> {
        if let Some(local_path) = &self.local_storage_path {
            let full_path = local_path.join(gcs_path);
            tokio::fs::read(&full_path)
                .await
                .map_err(|e| MediaStudioError::Storage(format!("Local read failed: {}", e)))
        } else if let Some(ref gcs) = self.gcs {
            let bucket = format!("projects/_/buckets/{}", BUCKET_NAME);
            let mut resp = gcs
                .read_object(&bucket, gcs_path)
                .send()
                .await
                .map_err(|e| MediaStudioError::Storage(format!("GCS read failed: {}", e)))?;

            let mut data = Vec::new();
            while let Some(chunk) = resp.next().await {
                let bytes = chunk
                    .map_err(|e| MediaStudioError::Storage(format!("GCS stream error: {}", e)))?;
                data.extend_from_slice(&bytes);
            }
            Ok(data)
        } else {
            Err(MediaStudioError::Storage("No storage backend configured".to_string()))
        }
    }

    async fn upload_capture(&self, path: &str, data: &[u8]) -> Result<(), MediaStudioError> {
        if let Some(local_path) = &self.local_storage_path {
            let full_path = local_path.join(path);
            if let Some(parent) = full_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| MediaStudioError::Storage(format!("Failed to create dir: {}", e)))?;
            }
            tokio::fs::write(&full_path, data)
                .await
                .map_err(|e| MediaStudioError::Storage(format!("Local write failed: {}", e)))?;
            println!("[media_studio] LOCAL: Saved edited capture to {:?}", full_path);
        } else if let Some(ref gcs) = self.gcs {
            let bucket = format!("projects/_/buckets/{}", BUCKET_NAME);
            let bytes = Bytes::copy_from_slice(data);
            gcs
                .write_object(&bucket, path, bytes)
                .send_buffered()
                .await
                .map_err(|e| MediaStudioError::Storage(format!("GCS write failed: {}", e)))?;
            println!("[media_studio] GCS: Uploaded edited capture to {}", path);
        } else {
            return Err(MediaStudioError::Storage("No storage backend configured".to_string()));
        }
        Ok(())
    }

    fn generate_edited_path(&self, user_id: i64, media_type: &str, extension: &str) -> String {
        let now = Utc::now();
        let date = now.format("%Y-%m-%d");
        let timestamp = now.timestamp_millis();
        format!(
            "{}/user_{}/{}/edited_{}.{}",
            media_type, user_id, date, timestamp, extension
        )
    }

    async fn insert_edited_capture(
        &self,
        user_id: i64,
        media_type: &str,
        content_type: &str,
        gcs_path: &str,
        source_capture_id: i64,
        edit_params: serde_json::Value,
    ) -> Result<i64, MediaStudioError> {
        // Use a synthetic interval_id of 0 for edited captures
        let interval_id = 0i64;
        let captured_at = Utc::now();

        let result: (i64,) = sqlx::query_as(
            r#"
            INSERT INTO captures (interval_id, user_id, media_type, content_type, gcs_path, captured_at, source_capture_id, edit_params)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id
            "#,
        )
        .bind(interval_id)
        .bind(user_id)
        .bind(media_type)
        .bind(content_type)
        .bind(gcs_path)
        .bind(captured_at)
        .bind(source_capture_id)
        .bind(edit_params)
        .fetch_one(&self.db)
        .await?;

        Ok(result.0)
    }

    fn apply_image_crop(&self, data: &[u8], crop: &CropParams) -> Result<Vec<u8>, MediaStudioError> {
        let img = ImageReader::new(Cursor::new(data))
            .with_guessed_format()
            .map_err(|e| MediaStudioError::Processing(format!("Failed to read image: {}", e)))?
            .decode()
            .map_err(|e| MediaStudioError::Processing(format!("Failed to decode image: {}", e)))?;

        let width = img.width();
        let height = img.height();

        // Convert normalized coords to pixels
        let crop_x = (crop.x * width as f64) as u32;
        let crop_y = (crop.y * height as f64) as u32;
        let crop_w = (crop.width * width as f64) as u32;
        let crop_h = (crop.height * height as f64) as u32;

        // Ensure we don't exceed bounds (shouldn't happen after validation, but be safe)
        let crop_w = crop_w.min(width - crop_x);
        let crop_h = crop_h.min(height - crop_y);

        let cropped = img.crop_imm(crop_x, crop_y, crop_w, crop_h);

        // Encode back (use PNG for lossless, or detect original format)
        let mut output = Cursor::new(Vec::new());
        cropped
            .write_to(&mut output, image::ImageFormat::Png)
            .map_err(|e| MediaStudioError::Processing(format!("Failed to encode image: {}", e)))?;

        Ok(output.into_inner())
    }

    async fn apply_video_trim(
        &self,
        data: &[u8],
        trim: &TrimParams,
    ) -> Result<Vec<u8>, MediaStudioError> {
        let temp_dir = std::env::temp_dir();
        let input_path = temp_dir.join(format!("cleo_trim_input_{}.tmp", rand::random::<u64>()));
        let output_path = temp_dir.join(format!("cleo_trim_output_{}.mp4", rand::random::<u64>()));

        // Write input to temp file
        tokio::fs::write(&input_path, data)
            .await
            .map_err(|e| MediaStudioError::Processing(format!("Failed to write temp input: {}", e)))?;

        // Run ffmpeg to trim
        // -ss before -i for fast seeking, -t for duration
        // -c copy for fast stream copy (no re-encoding)
        let output = Command::new("ffmpeg")
            .args([
                "-ss",
                &trim.start_timestamp,
                "-i",
                input_path.to_str().unwrap(),
                "-t",
                &trim.duration_secs.to_string(),
                "-c",
                "copy",
                "-y",
                output_path.to_str().unwrap(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| MediaStudioError::Processing(format!("Failed to spawn ffmpeg: {}", e)))?;

        // Clean up input
        let _ = tokio::fs::remove_file(&input_path).await;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = tokio::fs::remove_file(&output_path).await;
            return Err(MediaStudioError::Processing(format!(
                "ffmpeg trim failed: {}",
                stderr
            )));
        }

        // Read output
        let trimmed_data = tokio::fs::read(&output_path)
            .await
            .map_err(|e| MediaStudioError::Processing(format!("Failed to read trimmed output: {}", e)))?;

        // Clean up output
        let _ = tokio::fs::remove_file(&output_path).await;

        Ok(trimmed_data)
    }
}
