//! Tweet action endpoints (/tweets/*)

use axum::{
    Json, Router,
    extract::{Path, Query, State, WebSocketUpgrade, ws::{Message, WebSocket}},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use axum_extra::extract::CookieJar;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::AppState;
use crate::domain::twitter::{tweets, queries::threads as thread_queries};
use crate::routes::auth::AuthUser;
use crate::routes::nudges::get_sanitized_nudges;
use crate::services::{auth, error::LogErr, session, twitter};
use crate::constants::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE};
use reson_agentic::providers::{GenerationConfig, InferenceClient};
use reson_agentic::types::ChatMessage;
use reson_agentic::utils::ConversationMessage;
use super::dto::TweetResponse;
use super::media::{upload_tweet_media, upload_tweet_media_with_progress, UploadProgress};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/tweets", get(list_tweets))
        .route("/tweets/{id}/publish", post(post_tweet))
        .route("/tweets/{id}/publish/ws", get(publish_tweet_ws))
        .route("/tweets/{id}", delete(dismiss_tweet))
        .route("/tweets/{id}/regenerate", post(regenerate_tweet))
}

#[derive(Deserialize)]
struct ListTweetsQuery {
    limit: Option<i64>,
    offset: Option<i64>,
    status: Option<String>,
}

#[derive(Serialize)]
struct ListTweetsResponse {
    tweets: Vec<TweetResponse>,
    total: i64,
    has_more: bool,
}

/// GET /tweets - List pending tweets for a user with pagination
async fn list_tweets(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Query(query): Query<ListTweetsQuery>,
) -> Result<Json<ListTweetsResponse>, StatusCode> {
    let limit = query.limit.unwrap_or(DEFAULT_PAGE_SIZE).min(MAX_PAGE_SIZE);
    let offset = query.offset.unwrap_or(0);
    let status_filter = query.status.as_deref();

    let total = tweets::count_standalone_tweets(&state.db, user_id, status_filter)
        .await
        .log_500("Count tweets error")?;

    let result = tweets::list_pending_tweets_paginated(&state.db, user_id, status_filter, limit, offset)
        .await
        .log_500("List tweets error")?;

    let has_more = offset + (result.len() as i64) < total;

    Ok(Json(ListTweetsResponse {
        tweets: result.into_iter().map(TweetResponse::from).collect(),
        total,
        has_more,
    }))
}

#[derive(Serialize)]
struct PostTweetResponse {
    tweet_id: String,
    text: String,
}

/// POST /tweets/:id/publish - Post a tweet to Twitter
async fn post_tweet(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path(tweet_collateral_id): Path<i64>,
) -> Result<Json<PostTweetResponse>, StatusCode> {
    println!("[post_tweet] Handler called for tweet_collateral_id={}", tweet_collateral_id);
    println!("[post_tweet] user_id={}", user_id);

    // Get the tweet with media info
    let tweet = tweets::get_tweet_for_posting(&state.db, tweet_collateral_id, user_id)
        .await
        .log_500("Get tweet for posting error")?
        .ok_or(StatusCode::NOT_FOUND)?;

    let can_publish = tweets::set_tweet_posting(&state.db, tweet_collateral_id, user_id)
        .await
        .log_500("Set tweet posting status error")?;
    if !can_publish {
        return Err(StatusCode::CONFLICT);
    }

    // Get user tokens
    let tokens = twitter::get_user_tokens(&state.db, user_id)
        .await
        .log_500("Get user tokens error")?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Ensure token is valid (refresh if needed)
    let access_token = auth::ensure_valid_access_token(&state.db, &state.twitter, user_id, tokens).await?;

    let publish_result = (|| async {
        // Upload media if present
        println!(
            "[post_tweet] About to upload media. video_clip={:?}, image_ids={:?}",
            tweet.video_clip.is_some(),
            tweet.image_capture_ids
        );

        let media_ids = upload_tweet_media(&state, user_id, &tweet, &access_token)
            .await
            .map_err(|e| e.to_string())?;

        println!("[post_tweet] Media upload complete, got {} media_ids", media_ids.len());

        let media_ids_ref: Option<Vec<String>> = if media_ids.is_empty() {
            None
        } else {
            Some(media_ids)
        };

        // Post the tweet with media
        let twitter_response = state
            .twitter
            .post_tweet(&access_token, &tweet.text, None, media_ids_ref.as_deref())
            .await
            .map_err(|e| format!("Failed to post tweet: {}", e))?;

        tweets::mark_tweet_posted(&state.db, tweet_collateral_id, &twitter_response.id)
            .await
            .map_err(|e| format!("Failed to mark posted: {}", e))?;

        Ok::<(String, String), String>((twitter_response.id, twitter_response.text))
    })()
    .await;

    match publish_result {
        Ok((tweet_id, text)) => Ok(Json(PostTweetResponse { tweet_id, text })),
        Err(error) => {
            let _ = tweets::mark_tweet_publish_failed(
                &state.db,
                tweet_collateral_id,
                user_id,
                &error,
            )
            .await;
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// DELETE /tweets/:id - Dismiss a pending tweet without posting
async fn dismiss_tweet(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path(tweet_collateral_id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    let deleted = tweets::delete_tweet(&state.db, tweet_collateral_id, user_id)
        .await
        .log_500("Delete tweet error")?;

    if !deleted {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}

/// WebSocket progress messages
#[derive(Serialize)]
#[serde(tag = "type")]
enum WsProgress {
    #[serde(rename = "uploading")]
    Uploading { segment: usize, total: usize, percent: u8 },
    #[serde(rename = "processing")]
    Processing,
    #[serde(rename = "posting")]
    Posting,
    #[serde(rename = "complete")]
    Complete { tweet_id: String, text: String },
    #[serde(rename = "error")]
    Error { message: String },
}

impl From<UploadProgress> for WsProgress {
    fn from(p: UploadProgress) -> Self {
        match p {
            UploadProgress::Uploading { segment, total, percent } => {
                WsProgress::Uploading { segment, total, percent }
            }
            UploadProgress::Processing => WsProgress::Processing,
        }
    }
}

/// GET /tweets/:id/publish/ws - Publish tweet via WebSocket with progress
async fn publish_tweet_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Path(tweet_collateral_id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    // Validate JWT from cookie
    let access_token = jar
        .get("access_token")
        .map(|c| c.value())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let user_id = session::validate_access_token(access_token, &state.jwt_secret)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    Ok(ws.on_upgrade(move |socket| handle_publish_ws(socket, state, user_id, tweet_collateral_id)))
}

async fn handle_publish_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    user_id: i64,
    tweet_collateral_id: i64,
) {
    let (mut sender, _receiver) = socket.split();

    // Create channel for progress updates
    let (progress_tx, mut progress_rx) = mpsc::channel::<WsProgress>(32);

    // Spawn task to forward progress to WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = progress_rx.recv().await {
            let json = serde_json::to_string(&msg).unwrap();
            if sender.send(Message::Text(json.into())).await.is_err() {
                break;
            }
        }
        sender
    });

    // Do the actual work
    let result = do_publish_with_progress(&state, user_id, tweet_collateral_id, progress_tx.clone()).await;

    // Send final message
    match result {
        Ok((tweet_id, text)) => {
            let _ = progress_tx.send(WsProgress::Complete { tweet_id, text }).await;
        }
        Err(e) => {
            let _ = progress_tx.send(WsProgress::Error { message: e }).await;
        }
    }

    // Close channel and wait for sender to finish
    drop(progress_tx);
    let mut sender = send_task.await.unwrap();
    let _ = sender.close().await;
}

async fn do_publish_with_progress(
    state: &Arc<AppState>,
    user_id: i64,
    tweet_collateral_id: i64,
    progress_tx: mpsc::Sender<WsProgress>,
) -> Result<(String, String), String> {
    // Get the tweet with media info
    let tweet = tweets::get_tweet_for_posting(&state.db, tweet_collateral_id, user_id)
        .await
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or("Tweet not found")?;

    let can_publish = tweets::set_tweet_posting(&state.db, tweet_collateral_id, user_id)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
    if !can_publish {
        return Err("Tweet is already posting or posted".into());
    }

    // Get the tweet with media info
    let publish_result = (|| async {
        // Get user tokens
        let tokens = twitter::get_user_tokens(&state.db, user_id)
            .await
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Not authenticated with Twitter")?;

        // Ensure token is valid (refresh if needed)
        let access_token = auth::ensure_valid_access_token_str(&state.db, &state.twitter, user_id, tokens).await?;

        // Upload media with progress
        let media_ids = upload_tweet_media_with_progress(
            state,
            user_id,
            &tweet,
            &access_token,
            progress_tx.clone(),
        )
        .await
        .map_err(|e| format!("Media upload error: {}", e))?;

        // Send posting status
        let _ = progress_tx.send(WsProgress::Posting).await;

        let media_ids_ref: Option<Vec<String>> = if media_ids.is_empty() {
            None
        } else {
            Some(media_ids)
        };

        // Post the tweet
        let twitter_response = state
            .twitter
            .post_tweet(&access_token, &tweet.text, None, media_ids_ref.as_deref())
            .await
            .map_err(|e| format!("Failed to post tweet: {}", e))?;

        // Mark as posted (atomic - ignores result since tweet is already on Twitter)
        tweets::mark_tweet_posted(&state.db, tweet_collateral_id, &twitter_response.id)
            .await
            .map_err(|e| format!("Failed to mark posted: {}", e))?;

        Ok::<(String, String), String>((twitter_response.id, twitter_response.text))
    })()
    .await;

    match publish_result {
        Ok((tweet_id, text)) => Ok((tweet_id, text)),
        Err(error) => {
            let _ = tweets::mark_tweet_publish_failed(
                &state.db,
                tweet_collateral_id,
                user_id,
                &error,
            )
            .await;
            Err(error)
        }
    }
}

#[derive(Serialize)]
struct RegenerateTweetResponse {
    text: String,
}

/// POST /tweets/:id/regenerate - Generate a new variation of a tweet using AI
async fn regenerate_tweet(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path(tweet_id): Path<i64>,
) -> Result<Json<RegenerateTweetResponse>, StatusCode> {
    // Check if Gemini is available
    let gemini = state.gemini.as_ref().ok_or_else(|| {
        eprintln!("[regenerate_tweet] Gemini client not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Get the tweet with its context
    let tweet = tweets::get_tweet_for_posting(&state.db, tweet_id, user_id)
        .await
        .log_500("Get tweet error")?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Get user's style nudges for voice customization
    let nudges = get_sanitized_nudges(&state.db, user_id).await;

    // Build the prompt
    let nudges_section = match nudges {
        Some(n) if !n.trim().is_empty() => format!(
            "\n\nUser's style preferences:\n{}\n",
            n
        ),
        _ => String::new(),
    };

    let prompt = format!(
        r#"You are a tweet ghostwriter. Generate a fresh take on this tweet while keeping the same core message and context.

Original tweet:
"{}"

Why this moment matters:
{}
{}
Rules:
- Keep it under 280 characters
- No AI-sounding phrases: "excited to share", "dive into", "game-changer", "incredibly", "just"
- No emoji spam
- No over-explaining or hedging
- Keep it natural and conversational
- Maintain the same general topic/message but vary the wording

Respond with ONLY the new tweet text, nothing else."#,
        tweet.text,
        tweet.rationale,
        nudges_section
    );

    // Call Gemini
    let messages = vec![ConversationMessage::Chat(ChatMessage::user(prompt))];
    let config = GenerationConfig {
        model: "gemini-2.5-flash".to_string(),
        max_tokens: Some(100),
        temperature: Some(0.9), // Higher temperature for more variation
        top_p: None,
        tools: None,
        native_tools: false,
        reasoning_effort: None,
        thinking_budget: None,
        output_schema: None,
        output_type_name: None,
    };

    let response = gemini
        .get_generation(&messages, &config)
        .await
        .map_err(|e| {
            eprintln!("[regenerate_tweet] Gemini error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Extract the text from the response (content is already a String)
    let new_text = response.content.trim().trim_matches('"').to_string();

    // Validate length
    if new_text.is_empty() || new_text.len() > 280 {
        eprintln!("[regenerate_tweet] Invalid generated text length: {}", new_text.len());
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    // Update the tweet in the database
    thread_queries::update_tweet_collateral(
        &state.db,
        tweet_id,
        user_id,
        Some(&new_text),
        None,
        None,
    )
    .await
    .log_500("Update tweet text error")?;

    println!("[regenerate_tweet] Generated new text for tweet {}: {}", tweet_id, new_text);

    Ok(Json(RegenerateTweetResponse { text: new_text }))
}
