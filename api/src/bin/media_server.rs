//! Standalone media file server for local LLM access.
//!
//! Serves capture files (videos, images) over HTTP so that locally-hosted
//! vision models (e.g. Qwen3-VL via vLLM) can fetch them by URL.
//!
//! Supports two storage backends:
//! - **Local disk**: reads from `LOCAL_STORAGE_PATH`
//! - **GCS**: fetches from Google Cloud Storage (requires `GOOGLE_APPLICATION_CREDENTIALS`)
//!
//! Tries local first, falls back to GCS.
//!
//! ## Environment Variables
//! - `LOCAL_STORAGE_PATH` - directory containing capture files (optional)
//! - `GCS_BUCKET_NAME` - GCS bucket name (default: `cleo_multimedia_data`)
//! - `MEDIA_SERVER_PORT` - port to listen on (default: `3001`)

use axum::{
    Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use google_cloud_storage::client::Storage;
use std::path::PathBuf;
use std::sync::Arc;

struct MediaState {
    local_storage_path: Option<PathBuf>,
    gcs: Option<Storage>,
    gcs_bucket: String,
}

fn content_type_for(path: &str) -> &'static str {
    if path.ends_with(".mp4") {
        "video/mp4"
    } else if path.ends_with(".webm") {
        "video/webm"
    } else if path.ends_with(".mov") {
        "video/quicktime"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".webp") {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

async fn serve_file(
    State(state): State<Arc<MediaState>>,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    // Path traversal protection
    if path.contains("..") || path.contains('\0') || path.starts_with('/') {
        return Err(StatusCode::FORBIDDEN);
    }

    let content_type = content_type_for(&path);

    // Try local storage first
    if let Some(ref local_path) = state.local_storage_path {
        let full_path = local_path.join(&path);
        if let Ok(canonical) = full_path.canonicalize() {
            if let Ok(storage_canonical) = local_path.canonicalize() {
                if canonical.starts_with(&storage_canonical) {
                    if let Ok(bytes) = tokio::fs::read(&canonical).await {
                        return Ok((
                            [(header::CONTENT_TYPE, content_type)],
                            bytes,
                        ));
                    }
                }
            }
        }
    }

    // Fall back to GCS
    if let Some(ref gcs) = state.gcs {
        let bucket = format!("projects/_/buckets/{}", state.gcs_bucket);
        match gcs.read_object(&bucket, &path).send().await {
            Ok(mut resp) => {
                let mut data = Vec::new();
                while let Some(chunk) = resp.next().await {
                    data.extend_from_slice(
                        &chunk.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
                    );
                }
                return Ok((
                    [(header::CONTENT_TYPE, content_type)],
                    data,
                ));
            }
            Err(_) => return Err(StatusCode::NOT_FOUND),
        }
    }

    Err(StatusCode::NOT_FOUND)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = std::env::var("MEDIA_SERVER_PORT").unwrap_or_else(|_| "3001".to_string());
    let local_storage_path = std::env::var("LOCAL_STORAGE_PATH").ok().map(PathBuf::from);
    let gcs_bucket = std::env::var("GCS_BUCKET_NAME")
        .unwrap_or_else(|_| "cleo_multimedia_data".to_string());

    // Initialize GCS client if credentials are available
    let gcs = match Storage::builder().build().await {
        Ok(client) => {
            println!("[media-server] GCS client initialized");
            Some(client)
        }
        Err(e) => {
            println!("[media-server] GCS not available: {}", e);
            None
        }
    };

    if local_storage_path.is_none() && gcs.is_none() {
        eprintln!("[media-server] WARNING: No storage backend configured.");
        eprintln!("[media-server] Set LOCAL_STORAGE_PATH and/or GOOGLE_APPLICATION_CREDENTIALS.");
    }

    if let Some(ref path) = local_storage_path {
        println!("[media-server] Local storage: {:?}", path);
    }

    let state = Arc::new(MediaState {
        local_storage_path,
        gcs,
        gcs_bucket,
    });

    let app = Router::new()
        .route("/{*path}", get(serve_file))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("[media-server] Listening on http://{}", addr);

    axum::serve(listener, app).await?;
    Ok(())
}
