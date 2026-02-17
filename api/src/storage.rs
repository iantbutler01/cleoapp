//! Shared storage utilities for downloading/uploading capture data.
//!
//! Used by thumbnails, frames, and agent pipelines to avoid duplicating
//! download/upload logic across the codebase.

use bytes::Bytes;
use std::path::PathBuf;

/// Download a single capture from local storage or GCS.
pub async fn download_capture(
    gcs: Option<&google_cloud_storage::client::Storage>,
    local_storage_path: Option<&PathBuf>,
    bucket_name: &str,
    gcs_path: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(local_path) = local_storage_path {
        let full_path = local_path.join(gcs_path);
        Ok(tokio::fs::read(&full_path).await?)
    } else if let Some(gcs) = gcs {
        let bucket = format!("projects/_/buckets/{}", bucket_name);
        let mut resp = gcs.read_object(&bucket, gcs_path).send().await?;
        let mut data = Vec::new();
        while let Some(chunk) = resp.next().await {
            data.extend_from_slice(&chunk?);
        }
        Ok(data)
    } else {
        Err("No storage backend configured (set LOCAL_STORAGE_PATH or GOOGLE_APPLICATION_CREDENTIALS)".into())
    }
}

/// Upload data to local storage or GCS.
pub async fn upload_data(
    gcs: Option<&google_cloud_storage::client::Storage>,
    local_storage_path: Option<&PathBuf>,
    bucket_name: &str,
    path: &str,
    data: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(local_path) = local_storage_path {
        let full_path = local_path.join(path);
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&full_path, data).await?;
    } else if let Some(gcs) = gcs {
        let bucket = format!("projects/_/buckets/{}", bucket_name);
        let bytes = Bytes::copy_from_slice(data);
        gcs.write_object(&bucket, path, bytes)
            .send_buffered()
            .await?;
    } else {
        return Err("No storage backend configured".into());
    }
    Ok(())
}
