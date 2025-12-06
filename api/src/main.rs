mod agent;
mod twitter;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use google_cloud_storage::client::Storage;
use reson_agentic::providers::GoogleGenAIClient;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;

use twitter::TwitterClient;

const BUCKET_NAME: &str = "cleo_multimedia_data";

#[derive(Clone)]
struct AppState {
    db: PgPool,
    gcs: Storage,
    gemini: GoogleGenAIClient,
    twitter: TwitterClient,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ActivityEvent {
    ForegroundSwitch {
        #[serde(rename = "newApplication")]
        new_application: String,
        #[serde(rename = "newWindow")]
        new_window: String,
    },
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
struct CaptureResponse {
    id: i64,
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

async fn capture(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<CaptureResponse>, StatusCode> {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream");

    let interval_id: i64 = headers
        .get("x-interval-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let user_id: i64 = headers
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let media_type = if content_type.starts_with("image/") {
        "image"
    } else if content_type.starts_with("video/") {
        "video"
    } else {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    };

    let now = Utc::now();
    let day_bucket = now.format("%Y-%m-%d").to_string();
    let timestamp = now.timestamp_millis();
    let ext = get_extension(content_type);

    // Path: video/user_123/2025-12-06/1733500000000.mp4
    // Path: image/user_123/2025-12-06/1733500000000.png
    let gcs_path = format!(
        "{}/user_{}/{}/{}.{}",
        media_type, user_id, day_bucket, timestamp, ext
    );

    // Upload to GCS
    let bucket = format!("projects/_/buckets/{}", BUCKET_NAME);
    state
        .gcs
        .write_object(&bucket, &gcs_path, body)
        .send_buffered()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Store reference in DB
    let row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO captures (interval_id, user_id, media_type, content_type, gcs_path, captured_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(interval_id)
    .bind(user_id)
    .bind(media_type)
    .bind(content_type)
    .bind(&gcs_path)
    .bind(now)
    .fetch_one(&state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(CaptureResponse { id: row.0 }))
}

async fn activity(
    State(state): State<Arc<AppState>>,
    Json(activities): Json<Vec<Activity>>,
) -> Result<StatusCode, StatusCode> {
    for activity in activities {
        let (event_type, application, window) = match &activity.event {
            ActivityEvent::ForegroundSwitch {
                new_application,
                new_window,
            } => (
                "ForegroundSwitch",
                Some(new_application.as_str()),
                Some(new_window.as_str()),
            ),
            ActivityEvent::MouseClick => ("MouseClick", None, None),
        };

        sqlx::query(
            r#"
            INSERT INTO activities (timestamp, interval_id, event_type, application, window)
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
    let tweets = agent::run_collateral_job(
        state.db.clone(),
        state.gcs.clone(),
        state.gemini.clone(),
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
    let _ = twitter::save_oauth_state(&state.db, &auth_request.state, &auth_request.code_verifier).await;

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

    let tweets: Vec<PendingTweet> = sqlx::query_as::<_, (i64, String, Option<serde_json::Value>, Vec<i64>, String, DateTime<Utc>)>(
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
    .map(|(id, text, video_clip, image_capture_ids, rationale, created_at)| PendingTweet {
        id,
        text,
        video_clip,
        image_capture_ids,
        rationale,
        created_at,
    })
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

/// GET /me - Get current user info
async fn get_me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<twitter::User>, StatusCode> {
    let user_id: i64 = headers
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let user = twitter::get_user_by_id(&state.db, user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(user))
}

#[tokio::main]
async fn main() {
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://cleo:cleo@localhost/cleo".to_string());

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

    // Gemini client for File API operations
    let gemini_api_key = std::env::var("GOOGLE_GEMINI_API_KEY")
        .expect("GOOGLE_GEMINI_API_KEY must be set");
    let gemini = GoogleGenAIClient::new(&gemini_api_key, "gemini-2.0-flash");

    // Twitter OAuth 2.0 client
    let twitter_client_id = std::env::var("TWITTER_CLIENT_ID")
        .expect("TWITTER_CLIENT_ID must be set");
    let twitter_client_secret = std::env::var("TWITTER_CLIENT_SECRET")
        .expect("TWITTER_CLIENT_SECRET must be set");
    let twitter_redirect_uri = std::env::var("TWITTER_REDIRECT_URI")
        .unwrap_or_else(|_| "http://localhost:3000/auth/twitter/callback".to_string());
    let twitter = TwitterClient::new(&twitter_client_id, &twitter_client_secret, &twitter_redirect_uri);

    let state = Arc::new(AppState {
        db: pool.clone(),
        gcs: gcs.clone(),
        gemini: gemini.clone(),
        twitter,
    });

    // Start background scheduler for idle user processing
    // Checks every 5 minutes for users idle for 30+ minutes
    tokio::spawn(agent::start_background_scheduler(
        pool,
        gcs,
        gemini,
        30,  // idle_minutes
        300, // check_interval_secs (5 min)
    ));

    let app = Router::new()
        // Health
        .route("/health", get(health))
        // Capture & Activity
        .route("/capture", post(capture))
        .route("/activity", post(activity))
        // Agent
        .route("/agent/run", post(run_agent))
        // Auth
        .route("/auth/twitter", get(auth_twitter))
        .route("/auth/twitter/token", post(auth_twitter_token))
        // User
        .route("/me", get(get_me))
        // Tweets
        .route("/tweets", get(list_tweets))
        .route("/tweets/{id}/post", post(post_tweet))
        .route("/tweets/{id}", axum::routing::delete(dismiss_tweet))
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind to {}: {}", addr, e));

    println!("Listening on http://{}", addr);
    println!("[scheduler] Background scheduler started (30min idle, 5min check)");
    axum::serve(listener, app).await.expect("Server failed");
}
