//! User info and limits endpoints (/me, /me/limits)

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::{DateTime, Utc};
use google_cloud_storage::client::Storage;
use serde::Serialize;
use std::sync::Arc;

use crate::constants::BUCKET_NAME;
use crate::services::twitter;
use crate::AppState;
use super::auth::AuthUser;
use super::captures::get_user_id_from_bearer;

/// User API response DTO
#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: i64,
    pub username: String,
    pub display_name: Option<String>,
    pub created_at: DateTime<Utc>,
    // twitter_id intentionally omitted - internal use only
}

impl From<twitter::User> for UserResponse {
    fn from(u: twitter::User) -> Self {
        Self {
            id: u.id,
            username: u.twitter_username,
            display_name: u.twitter_name,
            created_at: u.created_at,
        }
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/me", get(get_me))
        .route("/me/limits", get(get_limits))
}

/// GET /me - Get current user info
async fn get_me(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
) -> Result<Json<UserResponse>, StatusCode> {
    // Return 401 if user not found - a valid JWT for a deleted user is still unauthorized
    // (don't leak user existence via 404 vs 401 distinction)
    let user = twitter::get_user_by_id(&state.db, user_id)
        .await
        .map_err(|e| {
            eprintln!("Get user by ID error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    Ok(Json(UserResponse::from(user)))
}

#[derive(Serialize)]
struct RecordingLimits {
    /// Maximum duration of a single recording in seconds
    max_recording_duration_secs: u64,
    /// Recording budget per hour in seconds (regenerates over time)
    recording_budget_secs: u64,
    /// Inactivity duration before recording stops in seconds
    inactivity_timeout_secs: u64,
    /// Total storage limit in bytes for this user's tier
    storage_limit_bytes: u64,
    /// Current storage used in bytes
    storage_used_bytes: u64,
}

/// GET /me/limits - Get recording limits for the authenticated user (daemon auth)
async fn get_limits(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<RecordingLimits>, StatusCode> {
    let user_id = get_user_id_from_bearer(&state.db, &headers).await?;

    // TODO: Look up user's subscription tier and return appropriate limits
    // For now, use default free tier limits
    let storage_limit: u64 = 5 * 1024 * 1024 * 1024; // 5 GB

    // Calculate storage usage from actual storage (local folder or GCS)
    let storage_used = calculate_user_storage(&state, user_id).await;

    Ok(Json(RecordingLimits {
        max_recording_duration_secs: 5 * 60, // 5 minutes
        recording_budget_secs: 30 * 60,      // 30 minutes per hour
        inactivity_timeout_secs: 30,         // 30 seconds of inactivity
        storage_limit_bytes: storage_limit,
        storage_used_bytes: storage_used,
    }))
}

/// Calculate total storage used by a user from local folder or GCS
async fn calculate_user_storage(state: &AppState, user_id: i64) -> u64 {
    if let Some(local_path) = &state.local_storage_path {
        // Calculate from local filesystem
        calculate_local_storage(local_path, user_id).await
    } else {
        // Calculate from GCS - list objects with user prefix and sum sizes
        calculate_gcs_storage(&state.gcs, user_id).await
    }
}

async fn calculate_local_storage(base_path: &std::path::Path, user_id: i64) -> u64 {
    let mut total: u64 = 0;

    // Check both image and video directories
    for media_type in ["image", "video"] {
        let user_dir = base_path.join(format!("{}/user_{}", media_type, user_id));
        if let Ok(entries) = std::fs::read_dir(&user_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    // Day bucket directory
                    if let Ok(files) = std::fs::read_dir(entry.path()) {
                        for file in files.flatten() {
                            if let Ok(meta) = file.metadata() {
                                total += meta.len();
                            }
                        }
                    }
                }
            }
        }
    }

    total
}

async fn calculate_gcs_storage(_gcs: &Storage, user_id: i64) -> u64 {
    use futures::{StreamExt, pin_mut};

    // Use cloud-storage crate for listing (same one used for signed URLs)
    let client = cloud_storage::Client::default();
    let mut total: u64 = 0;

    for media_type in ["image", "video"] {
        let prefix = format!("{}/user_{}/", media_type, user_id);
        let request = cloud_storage::ListRequest {
            prefix: Some(prefix),
            ..Default::default()
        };

        if let Ok(stream) = client.object().list(BUCKET_NAME, request).await {
            pin_mut!(stream);
            while let Some(result) = stream.next().await {
                if let Ok(object_list) = result {
                    for obj in object_list.items {
                        total += obj.size;
                    }
                }
            }
        }
    }

    total
}
