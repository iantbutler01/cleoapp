mod agent;
mod twitter;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{DateTime, Duration, Utc};
use google_cloud_storage::client::Storage;
use reson_agentic::providers::GoogleGenAIClient;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::path::PathBuf;
use std::sync::Arc;

use twitter::TwitterClient;

const BUCKET_NAME: &str = "cleo_multimedia_data";
const MAX_CAPTURE_UPLOAD_SIZE: usize = 200 * 1024 * 1024; // 200 MB limit for uploads

#[derive(Clone)]
struct AppState {
    db: PgPool,
    gcs: Storage,
    gemini: Option<GoogleGenAIClient>,
    twitter: TwitterClient,
    /// Optional local storage path - if set, captures are written to disk instead of GCS
    local_storage_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ActivityEvent {
    #[serde(rename = "ForegroundSwitch")]
    ForegroundSwitch {
        #[serde(rename = "newActive")]
        new_active: String,
        #[serde(rename = "windowTitle")]
        window_title: String,
    },
    #[serde(rename = "MouseClick")]
    MouseClick,
}

#[derive(Debug, Deserialize)]
struct Activity {
    timestamp: DateTime<Utc>,
    #[serde(rename = "intervalId")]
    interval_id: i64,
    event: ActivityEvent,
}

#[derive(Serialize)]
struct BatchCaptureResponse {
    ids: Vec<i64>,
    uploaded: usize,
    failed: usize,
}

fn get_extension(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        _ => "bin",
    }
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
) -> Result<Json<BatchCaptureResponse>, StatusCode> {
    let user_id = get_user_id_from_bearer(&state.db, &headers).await?;

    let interval_id: i64 = headers
        .get("x-interval-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let mut ids = Vec::new();
    let mut failed = 0usize;

    while let Some(field) = multipart.next_field().await.map_err(|_| StatusCode::BAD_REQUEST)? {
        let content_type = field
            .content_type()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let media_type = if content_type.starts_with("image/") {
            "image"
        } else if content_type.starts_with("video/") {
            "video"
        } else {
            eprintln!("[capture_batch] Skipping unsupported content type: {}", content_type);
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
                    eprintln!("[capture_batch] Failed to create directory {:?}: {}", parent, e);
                    failed += 1;
                    continue;
                }
            }
            match tokio::fs::write(&full_path, &body).await {
                Ok(()) => {
                    println!("[capture_batch] LOCAL: Saved {} bytes to {:?}", body.len(), full_path);
                    Ok(())
                }
                Err(e) => {
                    eprintln!("[capture_batch] Failed to write file {:?}: {}", full_path, e);
                    Err(e)
                }
            }
        } else {
            let bucket = format!("projects/_/buckets/{}", BUCKET_NAME);
            match state.gcs.write_object(&bucket, &relative_path, body.clone()).send_buffered().await {
                Ok(_) => {
                    println!("[capture_batch] GCS: Uploaded to {}", relative_path);
                    Ok(())
                }
                Err(e) => {
                    eprintln!("[capture_batch] GCS upload failed: {}", e);
                    Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
                }
            }
        };

        if write_result.is_err() {
            failed += 1;
            continue;
        }

        // Store reference in DB
        match sqlx::query_as::<_, (i64,)>(
            r#"
            INSERT INTO captures (interval_id, user_id, media_type, content_type, gcs_path, captured_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#,
        )
        .bind(interval_id)
        .bind(user_id)
        .bind(media_type)
        .bind(&content_type)
        .bind(&relative_path)
        .bind(now)
        .fetch_one(&state.db)
        .await
        {
            Ok(row) => {
                ids.push(row.0);
            }
            Err(e) => {
                eprintln!("[capture_batch] DB insert failed: {}", e);
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

    Ok(Json(BatchCaptureResponse {
        ids: ids.clone(),
        uploaded: ids.len(),
        failed,
    }))
}

async fn activity(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(activities): Json<Vec<Activity>>,
) -> Result<StatusCode, StatusCode> {
    // Authenticate via bearer token
    let _user_id = get_user_id_from_bearer(&state.db, &headers).await?;

    for activity in activities {
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

        sqlx::query(
            r#"
            INSERT INTO activities (timestamp, interval_id, event_type, application, "window")
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(activity.timestamp)
        .bind(activity.interval_id)
        .bind(event_type)
        .bind(application)
        .bind(window)
        .execute(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(StatusCode::CREATED)
}

#[derive(Deserialize)]
struct RunAgentRequest {
    user_id: i64,
}

#[derive(Serialize)]
struct RunAgentResponse {
    tweets_generated: usize,
    tweets: Vec<agent::TweetCollateral>,
}

async fn run_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RunAgentRequest>,
) -> Result<Json<RunAgentResponse>, StatusCode> {
    let gemini = state.gemini.clone().ok_or_else(|| {
        eprintln!("Agent error: Gemini API key not configured");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    let tweets = agent::run_collateral_job(
        state.db.clone(),
        state.gcs.clone(),
        gemini,
        req.user_id,
    )
    .await
    .map_err(|e| {
        eprintln!("Agent error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(RunAgentResponse {
        tweets_generated: tweets.len(),
        tweets,
    }))
}

async fn health() -> &'static str {
    "ok"
}

// ============== Twitter OAuth Endpoints ==============

#[derive(Serialize)]
struct AuthUrlResponse {
    url: String,
}

/// GET /auth/twitter - Start OAuth flow, returns URL to redirect user to
async fn auth_twitter(State(state): State<Arc<AppState>>) -> Json<AuthUrlResponse> {
    let auth_request = state.twitter.get_authorize_url(&[
        "tweet.read",
        "tweet.write",
        "users.read",
        "offline.access",
    ]);

    // Store state and code_verifier for callback
    let _ = twitter::save_oauth_state(&state.db, &auth_request.state, &auth_request.code_verifier)
        .await;

    Json(AuthUrlResponse {
        url: auth_request.url,
    })
}

#[derive(Deserialize)]
struct TokenRequest {
    code: String,
    state: String,
}

#[derive(Serialize)]
struct LoginResponse {
    user_id: i64,
    username: String,
}

/// POST /auth/twitter/token - Exchange OAuth code for session
async fn auth_twitter_token(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TokenRequest>,
) -> Result<Json<LoginResponse>, StatusCode> {
    // Retrieve and validate state
    let code_verifier = twitter::get_oauth_state(&state.db, &req.state)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Exchange code for tokens
    let token_response = state
        .twitter
        .exchange_code(&req.code, &code_verifier)
        .await
        .map_err(|e| {
            eprintln!("Token exchange error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Get user info
    let twitter_user = state
        .twitter
        .get_me(&token_response.access_token)
        .await
        .map_err(|e| {
            eprintln!("Get me error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Calculate token expiry
    let expires_at = Utc::now() + Duration::seconds(token_response.expires_in);

    // Upsert user
    let user_id = twitter::upsert_user(
        &state.db,
        &twitter_user.id,
        &twitter_user.username,
        Some(&twitter_user.name),
        &token_response.access_token,
        token_response.refresh_token.as_deref(),
        expires_at,
    )
    .await
    .map_err(|e| {
        eprintln!("Upsert user error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(LoginResponse {
        user_id,
        username: twitter_user.username,
    }))
}

// ============== Tweet Management Endpoints ==============

#[derive(Serialize)]
struct PendingTweet {
    id: i64,
    text: String,
    video_clip: Option<serde_json::Value>,
    image_capture_ids: Vec<i64>,
    rationale: String,
    created_at: DateTime<Utc>,
}

/// GET /tweets - List pending tweets for a user
async fn list_tweets(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<PendingTweet>>, StatusCode> {
    let user_id: i64 = headers
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let tweets: Vec<PendingTweet> = sqlx::query_as::<
        _,
        (
            i64,
            String,
            Option<serde_json::Value>,
            Vec<i64>,
            String,
            DateTime<Utc>,
        ),
    >(
        r#"
        SELECT id, text, video_clip, image_capture_ids, rationale, created_at
        FROM tweet_collateral
        WHERE user_id = $1 AND posted_at IS NULL
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .into_iter()
    .map(
        |(id, text, video_clip, image_capture_ids, rationale, created_at)| PendingTweet {
            id,
            text,
            video_clip,
            image_capture_ids,
            rationale,
            created_at,
        },
    )
    .collect();

    Ok(Json(tweets))
}

#[derive(Serialize)]
struct PostTweetResponse {
    tweet_id: String,
    text: String,
}

/// POST /tweets/:id/post - Post a tweet to Twitter
async fn post_tweet(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tweet_collateral_id): Path<i64>,
) -> Result<Json<PostTweetResponse>, StatusCode> {
    let user_id: i64 = headers
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Get the tweet collateral
    let tweet: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT text FROM tweet_collateral
        WHERE id = $1 AND user_id = $2 AND posted_at IS NULL
        "#,
    )
    .bind(tweet_collateral_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let tweet_text = tweet.ok_or(StatusCode::NOT_FOUND)?.0;

    // Get user tokens
    let tokens = twitter::get_user_tokens(&state.db, user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Check if token is expired and refresh if needed
    let access_token = if tokens.token_expires_at < Utc::now() {
        if let Some(refresh_token) = &tokens.refresh_token {
            let new_tokens = state
                .twitter
                .refresh_token(refresh_token)
                .await
                .map_err(|e| {
                    eprintln!("Token refresh error: {}", e);
                    StatusCode::UNAUTHORIZED
                })?;

            let expires_at = Utc::now() + Duration::seconds(new_tokens.expires_in);
            twitter::update_user_tokens(
                &state.db,
                user_id,
                &new_tokens.access_token,
                new_tokens.refresh_token.as_deref(),
                expires_at,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

            new_tokens.access_token
        } else {
            return Err(StatusCode::UNAUTHORIZED);
        }
    } else {
        tokens.access_token
    };

    // Post the tweet
    let twitter_response = state
        .twitter
        .post_tweet(&access_token, &tweet_text)
        .await
        .map_err(|e| {
            eprintln!("Post tweet error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Update the tweet_collateral record
    sqlx::query(
        r#"
        UPDATE tweet_collateral
        SET posted_at = NOW(), tweet_id = $1
        WHERE id = $2
        "#,
    )
    .bind(&twitter_response.id)
    .bind(tweet_collateral_id)
    .execute(&state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(PostTweetResponse {
        tweet_id: twitter_response.id,
        text: twitter_response.text,
    }))
}

/// DELETE /tweets/:id - Dismiss a pending tweet without posting
async fn dismiss_tweet(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tweet_collateral_id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    let user_id: i64 = headers
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let result = sqlx::query(
        r#"
        DELETE FROM tweet_collateral
        WHERE id = $1 AND user_id = $2 AND posted_at IS NULL
        "#,
    )
    .bind(tweet_collateral_id)
    .bind(user_id)
    .execute(&state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}

// ============== Capture Media Endpoints ==============

#[derive(Serialize)]
struct SignedUrlResponse {
    url: String,
    content_type: String,
}

/// GET /captures/:id/url - Get a signed URL for a capture
async fn get_capture_url(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(capture_id): Path<i64>,
) -> Result<Json<SignedUrlResponse>, StatusCode> {
    let user_id = get_user_id_from_headers(&headers)?;

    // Get capture info and verify ownership
    let capture: Option<(String, String)> = sqlx::query_as(
        r#"
        SELECT gcs_path, content_type FROM captures
        WHERE id = $1 AND user_id = $2
        "#,
    )
    .bind(capture_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let (gcs_path, content_type) = capture.ok_or(StatusCode::NOT_FOUND)?;

    // If local storage is configured, return a local URL
    if state.local_storage_path.is_some() {
        // Return a URL that points to our /media endpoint
        let url = format!("/media/{}", gcs_path);
        return Ok(Json(SignedUrlResponse { url, content_type }));
    }

    // Generate signed URL (15 min expiry) using cloud-storage crate
    let client = cloud_storage::Client::default();
    let object = client.object().read(BUCKET_NAME, &gcs_path)
        .await
        .map_err(|e| {
            eprintln!("Object read error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let signed_url = object.download_url(15 * 60).map_err(|e| {
        eprintln!("Signed URL error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(SignedUrlResponse {
        url: signed_url,
        content_type,
    }))
}

/// GET /media/*path - Serve local media files
async fn serve_media(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let local_path = state
        .local_storage_path
        .as_ref()
        .ok_or(StatusCode::NOT_FOUND)?;

    let full_path = local_path.join(&path);

    // Security: ensure the path doesn't escape the storage directory
    let canonical = full_path
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let storage_canonical = local_path
        .canonicalize()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !canonical.starts_with(&storage_canonical) {
        return Err(StatusCode::FORBIDDEN);
    }

    // Read file
    let bytes = tokio::fs::read(&canonical)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

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

    Ok((
        [(header::CONTENT_TYPE, content_type)],
        bytes,
    ))
}

/// GET /me - Get current user info
async fn get_me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<twitter::User>, StatusCode> {
    let user_id = get_user_id_from_headers(&headers)?;

    let user = twitter::get_user_by_id(&state.db, user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(user))
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
        max_recording_duration_secs: 5 * 60,   // 5 minutes
        recording_budget_secs: 30 * 60,        // 30 minutes per hour
        inactivity_timeout_secs: 30,           // 30 seconds of inactivity
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

#[derive(Serialize)]
struct ApiTokenResponse {
    api_token: String,
}

/// POST /me/token - Generate a new API token for the daemon
async fn generate_api_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ApiTokenResponse>, StatusCode> {
    let user_id = get_user_id_from_headers(&headers)?;

    let token = twitter::generate_api_token();
    twitter::set_user_api_token(&state.db, user_id, &token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(ApiTokenResponse { api_token: token }))
}

/// GET /me/token - Get current API token (if exists)
async fn get_api_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Option<String>>, StatusCode> {
    let user_id = get_user_id_from_headers(&headers)?;

    let token = twitter::get_user_api_token(&state.db, user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(token))
}

/// Helper to extract user_id from X-User-Id header (for frontend session auth)
fn get_user_id_from_headers(headers: &HeaderMap) -> Result<i64, StatusCode> {
    headers
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or(StatusCode::UNAUTHORIZED)
}

/// Helper to extract user_id from Bearer token (for daemon auth)
async fn get_user_id_from_bearer(db: &PgPool, headers: &HeaderMap) -> Result<i64, StatusCode> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(StatusCode::UNAUTHORIZED)?;

    twitter::get_user_by_api_token(db, token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)
}

#[tokio::main]
async fn main() {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cleo:cleo@localhost/cleo".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to database");

    // GCS client uses GOOGLE_APPLICATION_CREDENTIALS env var
    let gcs = Storage::builder()
        .build()
        .await
        .expect("Failed to create GCS client");

    // Gemini client for File API operations (optional - if not set, background agent is disabled)
    let gemini = match std::env::var("GOOGLE_GEMINI_API_KEY") {
        Ok(key) => {
            println!("[startup] Gemini API key found, AI agent enabled");
            Some(GoogleGenAIClient::new(&key, "gemini-2.0-flash"))
        }
        Err(_) => {
            println!("[startup] GOOGLE_GEMINI_API_KEY not set, AI agent disabled");
            None
        }
    };

    // Twitter OAuth 2.0 client
    let twitter_client_id =
        std::env::var("TWITTER_CLIENT_ID").expect("TWITTER_CLIENT_ID must be set");
    let twitter_client_secret =
        std::env::var("TWITTER_CLIENT_SECRET").expect("TWITTER_CLIENT_SECRET must be set");
    let twitter_redirect_uri = std::env::var("TWITTER_REDIRECT_URI")
        .unwrap_or_else(|_| "http://localhost:3000/auth/twitter/callback".to_string());
    let twitter = TwitterClient::new(
        &twitter_client_id,
        &twitter_client_secret,
        &twitter_redirect_uri,
    );

    // Optional local storage path - if set, captures are saved locally instead of GCS
    let local_storage_path = std::env::var("LOCAL_STORAGE_PATH").ok().map(PathBuf::from);
    if let Some(ref path) = local_storage_path {
        println!("[startup] LOCAL_STORAGE_PATH set: {:?}", path);
        println!("[startup] Captures will be saved locally instead of GCS");
    }

    let state = Arc::new(AppState {
        db: pool.clone(),
        gcs: gcs.clone(),
        gemini: gemini.clone(),
        twitter,
        local_storage_path,
    });

    // Start background scheduler for idle user processing (only if Gemini is configured)
    if let Some(gemini_client) = gemini {
        // Checks every 5 minutes for users idle for 30+ minutes
        tokio::spawn(agent::start_background_scheduler(
            pool, gcs, gemini_client, 1,  // idle_minutes
            30, // check_interval_secs (5 min)
        ));
        println!("[scheduler] Background scheduler started (30min idle, 5min check)");
    } else {
        println!("[scheduler] Background scheduler DISABLED (no Gemini API key)");
    }

    let app = Router::new()
        // Health
        .route("/health", get(health))
        // Capture & Activity
        .route("/captures/batch", post(capture_batch))
        .route("/activity", post(activity))
        // Agent
        .route("/agent/run", post(run_agent))
        // Auth
        .route("/auth/twitter", get(auth_twitter))
        .route("/auth/twitter/token", post(auth_twitter_token))
        // User
        .route("/me", get(get_me))
        .route("/me/limits", get(get_limits))
        .route("/me/token", get(get_api_token).post(generate_api_token))
        // Captures
        .route("/captures/{id}/url", get(get_capture_url))
        // Local media serving (when LOCAL_STORAGE_PATH is set)
        .route("/media/{*path}", get(serve_media))
        // Tweets
        .route("/tweets", get(list_tweets))
        .route("/tweets/{id}/post", post(post_tweet))
        .route("/tweets/{id}", axum::routing::delete(dismiss_tweet))
        .layer(DefaultBodyLimit::max(MAX_CAPTURE_UPLOAD_SIZE))
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind to {}: {}", addr, e));

    println!("Listening on http://{}", addr);
    axum::serve(listener, app).await.expect("Server failed");
}
