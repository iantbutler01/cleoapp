//! Thread action endpoints (/threads/*, /tweets/:id/collateral)

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::domain::{captures, twitter::threads};
use crate::domain::twitter::ThreadStatus;
use crate::services::{auth, error::LogErr, twitter as twitter_service};
use crate::AppState;
use crate::routes::auth::AuthUser;
use crate::constants::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE};
use super::dto::{ThreadResponse, ThreadWithTweetsResponse};
use super::media::upload_tweet_media;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/threads", post(create_thread).get(list_threads))
        .route("/threads/{id}", get(get_thread).put(update_thread).delete(delete_thread))
        .route("/threads/{id}/tweets", post(add_tweet_to_thread))
        .route("/threads/{thread_id}/tweets/{tweet_id}", delete(remove_tweet_from_thread))
        .route("/threads/{id}/publish", post(post_thread))
        .route("/tweets/{id}/collateral", put(update_tweet_collateral))
}

#[derive(Deserialize)]
struct CreateThreadRequest {
    title: Option<String>,
    tweet_ids: Vec<i64>,
}

#[derive(Serialize)]
struct CreateThreadResponse {
    id: i64,
    title: Option<String>,
    tweet_count: usize,
}

/// POST /threads - Create a new thread from tweet IDs
async fn create_thread(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Json(payload): Json<CreateThreadRequest>,
) -> Result<(StatusCode, Json<CreateThreadResponse>), StatusCode> {

    if payload.tweet_ids.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify all tweets belong to this user and are unposted
    let valid = threads::verify_tweets_for_thread(&state.db, &payload.tweet_ids, user_id)
        .await
        .log_500("Verify tweets for thread error")?;

    if !valid {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Use transaction for atomic thread creation + tweet assignment
    let mut tx = state.db.begin().await.log_500("Begin transaction error")?;

    let thread_id = threads::create_thread(&mut *tx, user_id, payload.title.as_deref())
        .await
        .log_500("Create thread error")?;

    threads::assign_tweets_to_thread(&mut *tx, thread_id, &payload.tweet_ids, user_id)
        .await
        .log_500("Assign tweets to thread error")?;

    tx.commit().await.log_500("Commit transaction error")?;

    Ok((StatusCode::CREATED, Json(CreateThreadResponse {
        id: thread_id,
        title: payload.title,
        tweet_count: payload.tweet_ids.len(),
    })))
}

#[derive(Deserialize)]
struct ListThreadsQuery {
    limit: Option<i64>,
    offset: Option<i64>,
    status: Option<String>,
}

#[derive(Serialize)]
struct ListThreadsResponse {
    threads: Vec<ThreadResponse>,
    total: i64,
    has_more: bool,
}

/// GET /threads - List user's threads with pagination
async fn list_threads(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Query(query): Query<ListThreadsQuery>,
) -> Result<Json<ListThreadsResponse>, StatusCode> {
    let limit = query.limit.unwrap_or(DEFAULT_PAGE_SIZE).min(MAX_PAGE_SIZE);
    let offset = query.offset.unwrap_or(0);
    let status_filter = query.status.as_deref();

    let total = threads::count_threads(&state.db, user_id, status_filter)
        .await
        .log_500("Count threads error")?;

    let result = threads::list_threads_paginated(&state.db, user_id, status_filter, limit, offset)
        .await
        .log_500("List threads error")?;

    let has_more = offset + (result.len() as i64) < total;

    Ok(Json(ListThreadsResponse {
        threads: result.into_iter().map(ThreadResponse::from).collect(),
        total,
        has_more,
    }))
}

/// GET /threads/:id - Get thread with its tweets
async fn get_thread(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path(thread_id): Path<i64>,
) -> Result<Json<ThreadWithTweetsResponse>, StatusCode> {

    let result = threads::get_thread_with_tweets(&state.db, thread_id, user_id)
        .await
        .log_500("Get thread error")?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Also fetch the tweets for the thread
    let tweets = threads::get_thread_tweets(&state.db, thread_id, user_id)
        .await
        .log_500("Get thread tweets error")?;

    let result_with_tweets = crate::domain::twitter::ThreadWithTweets {
        thread: result.thread,
        tweets,
    };

    Ok(Json(ThreadWithTweetsResponse::from(result_with_tweets)))
}

#[derive(Deserialize)]
struct UpdateThreadRequest {
    title: Option<String>,
    tweet_ids: Option<Vec<i64>>,
}

/// PUT /threads/:id - Update thread (rename, reorder tweets)
async fn update_thread(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path(thread_id): Path<i64>,
    Json(payload): Json<UpdateThreadRequest>,
) -> Result<StatusCode, StatusCode> {

    let status = threads::get_thread_status(&state.db, thread_id, user_id)
        .await
        .log_500("Get thread status error")?
        .ok_or(StatusCode::NOT_FOUND)?;

    if status == ThreadStatus::Posting || status == ThreadStatus::Posted {
        return Err(StatusCode::CONFLICT);
    }

    // Use transaction for atomic title update + reorder
    let mut tx = state.db.begin().await.log_500("Begin transaction error")?;

    if let Some(ref title) = payload.title {
        threads::update_thread_title(&mut *tx, thread_id, user_id, title)
            .await
            .log_500("Update thread title error")?;
    }

    // Reorder tweets if new order provided
    if let Some(ref tweet_ids) = payload.tweet_ids {
        let valid = threads::verify_tweets_in_thread(&mut *tx, tweet_ids, thread_id, user_id)
            .await
            .log_500("Verify tweets in thread error")?;

        if !valid {
            return Err(StatusCode::BAD_REQUEST);
        }

        threads::reorder_thread_tweets(&mut *tx, thread_id, user_id, tweet_ids)
            .await
            .log_500("Reorder thread tweets error")?;
    }

    tx.commit().await.log_500("Commit transaction error")?;

    Ok(StatusCode::OK)
}

/// DELETE /threads/:id - Delete thread (unlinks tweets, doesn't delete them)
async fn delete_thread(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path(thread_id): Path<i64>,
) -> Result<StatusCode, StatusCode> {

    // Use transaction for atomic check + unlink + delete
    let mut tx = state.db.begin().await.log_500("Begin transaction error")?;

    let exists = threads::thread_exists(&mut *tx, thread_id, user_id)
        .await
        .log_500("Check thread exists error")?;

    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }

    threads::unlink_all_tweets_from_thread(&mut *tx, thread_id, user_id)
        .await
        .log_500("Unlink tweets from thread error")?;

    threads::delete_thread_record(&mut *tx, thread_id, user_id)
        .await
        .log_500("Delete thread record error")?;

    tx.commit().await.log_500("Commit transaction error")?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AddTweetToThreadRequest {
    tweet_id: i64,
    position: Option<i32>,
}

/// POST /threads/:id/tweets - Add a tweet to thread
async fn add_tweet_to_thread(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path(thread_id): Path<i64>,
    Json(payload): Json<AddTweetToThreadRequest>,
) -> Result<StatusCode, StatusCode> {

    let status = threads::get_thread_status(&state.db, thread_id, user_id)
        .await
        .log_500("Get thread status error")?
        .ok_or(StatusCode::NOT_FOUND)?;

    if status != ThreadStatus::Draft {
        return Err(StatusCode::CONFLICT);
    }

    // Verify tweet exists, belongs to user, is unposted, and not in another thread
    let existing_thread_id = threads::get_tweet_thread_info(&state.db, payload.tweet_id, user_id)
        .await
        .log_500("Get tweet thread info error")?
        .ok_or(StatusCode::NOT_FOUND)?;

    if existing_thread_id.is_some() && existing_thread_id != Some(thread_id) {
        return Err(StatusCode::CONFLICT);
    }

    // Use transaction for atomic position shifting + assignment
    let mut tx = state.db.begin().await.log_500("Begin transaction error")?;

    let final_position = if let Some(pos) = payload.position {
        // Shift existing positions up to make room
        threads::shift_positions_up(&mut *tx, thread_id, user_id, pos)
            .await
            .log_500("Shift positions up error")?;
        pos
    } else {
        // Get max position and append
        let max_pos = threads::get_max_thread_position(&mut *tx, thread_id, user_id)
            .await
            .log_500("Get max thread position error")?;
        max_pos.map(|p| p + 1).unwrap_or(0)
    };

    threads::assign_tweet_to_thread(&mut *tx, payload.tweet_id, thread_id, user_id, final_position)
        .await
        .log_500("Assign tweet to thread error")?;

    tx.commit().await.log_500("Commit transaction error")?;

    Ok(StatusCode::CREATED)
}

/// DELETE /threads/:thread_id/tweets/:tweet_id - Remove tweet from thread
async fn remove_tweet_from_thread(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path((thread_id, tweet_id)): Path<(i64, i64)>,
) -> Result<StatusCode, StatusCode> {

    let status = threads::get_thread_status(&state.db, thread_id, user_id)
        .await
        .log_500("Get thread status error")?
        .ok_or(StatusCode::NOT_FOUND)?;

    if status != ThreadStatus::Draft {
        return Err(StatusCode::CONFLICT);
    }

    // Use transaction for atomic unlink + position reordering
    let mut tx = state.db.begin().await.log_500("Begin transaction error")?;

    // Get current position before unlinking
    let position = threads::get_tweet_position_in_thread(&mut *tx, tweet_id, thread_id, user_id)
        .await
        .log_500("Get tweet position error")?;

    let position = match position {
        Some(pos) => pos,
        None => return Err(StatusCode::NOT_FOUND), // Tweet not in this thread
    };

    // Unlink tweet from thread
    threads::unlink_tweet_from_thread(&mut *tx, tweet_id, user_id)
        .await
        .log_500("Unlink tweet from thread error")?;

    // Shift positions down to fill the gap
    if let Some(pos) = position {
        threads::shift_positions_down(&mut *tx, thread_id, user_id, pos)
            .await
            .log_500("Shift positions down error")?;
    }

    tx.commit().await.log_500("Commit transaction error")?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct PostThreadTweetResult {
    id: i64,
    twitter_id: String,
    reply_to: Option<String>,
}

#[derive(Serialize)]
struct PostThreadResponse {
    status: String,
    tweets: Vec<PostThreadTweetResult>,
}

/// POST /threads/:id/publish - Post entire thread to Twitter as a reply chain
///
/// Uses local-before-remote pattern:
/// 1. Record intent (set status to 'posting') in transaction
/// 2. Make external API calls
/// 3. Record results in transaction
/// 4. Attempt compensation on failure
async fn post_thread(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path(thread_id): Path<i64>,
) -> Result<Json<PostThreadResponse>, StatusCode> {
    // Phase 1: Validate and record intent
    let status = threads::get_thread_status(&state.db, thread_id, user_id)
        .await
        .log_500("Get thread status error")?
        .ok_or(StatusCode::NOT_FOUND)?;

    if status != ThreadStatus::Draft {
        return Err(StatusCode::CONFLICT);
    }

    let tokens = twitter_service::get_user_tokens(&state.db, user_id)
        .await
        .log_500("Get user tokens error")?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Ensure token is valid (refresh if needed)
    let access_token = auth::ensure_valid_access_token(&state.db, &state.twitter, user_id, tokens).await?;

    // Record intent in transaction
    let mut tx = state.db.begin().await.log_500("Begin transaction error")?;

    threads::set_thread_posting(&mut *tx, thread_id, user_id)
        .await
        .log_500("Set thread posting error")?;

    let tweet_list = threads::get_tweets_for_posting(&mut *tx, thread_id, user_id)
        .await
        .log_500("Get tweets for posting error")?;

    if tweet_list.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    tx.commit().await.log_500("Commit intent transaction error")?;

    // Phase 2: External API calls with compensation tracking
    let mut results = Vec::new();
    let mut posted_twitter_ids: Vec<String> = Vec::new();
    let mut previous_tweet_id: Option<String> = None;
    let mut failed = false;

    for tweet in tweet_list {
        // Upload media for this tweet
        let media_ids = match upload_tweet_media(&state, user_id, &tweet, &access_token).await {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!("Failed to upload media for tweet {}: {}", tweet.id, e);
                failed = true;
                break;
            }
        };

        let media_ids_ref: Option<Vec<String>> = if media_ids.is_empty() {
            None
        } else {
            Some(media_ids)
        };

        // Post to Twitter
        let post_result = state
            .twitter
            .post_tweet(
                &access_token,
                &tweet.text,
                previous_tweet_id.as_deref(),
                media_ids_ref.as_deref(),
            )
            .await;

        match post_result {
            Ok(twitter_response) => {
                let twitter_id = twitter_response.id.clone();
                posted_twitter_ids.push(twitter_id.clone());

                results.push((tweet.id, twitter_id.clone(), previous_tweet_id.clone()));
                previous_tweet_id = Some(twitter_id);
            }
            Err(e) => {
                eprintln!("Failed to post tweet in thread: {}", e);
                failed = true;
                break;
            }
        }
    }

    // Phase 3: Record results in transaction
    let mut tx = state.db.begin().await.log_500("Begin results transaction error")?;

    for (collateral_id, twitter_id, reply_to) in &results {
        threads::mark_thread_tweet_posted(&mut *tx, *collateral_id, user_id, twitter_id, reply_to.as_deref())
            .await
            .log_500("Mark thread tweet posted error")?;
    }

    let final_status = if failed { "partial_failed" } else { "posted" };
    let first_tweet_id = results.first().map(|(_, twitter_id, _)| twitter_id.as_str());

    threads::update_thread_status(&mut *tx, thread_id, user_id, final_status, first_tweet_id)
        .await
        .log_500("Update thread status error")?;

    tx.commit().await.log_500("Commit results transaction error")?;

    // Phase 4: Compensation on failure
    // TODO: Add delete_tweet to TwitterClient to enable compensation for orphaned tweets
    // For now, we log the orphaned tweet IDs so they can be manually cleaned up
    if failed && !posted_twitter_ids.is_empty() {
        eprintln!(
            "Thread {} partially failed. {} tweets were posted to Twitter but thread failed: {:?}",
            thread_id,
            posted_twitter_ids.len(),
            posted_twitter_ids
        );
    }

    let response_tweets = results
        .into_iter()
        .map(|(id, twitter_id, reply_to)| PostThreadTweetResult {
            id,
            twitter_id,
            reply_to,
        })
        .collect();

    Ok(Json(PostThreadResponse {
        status: final_status.to_string(),
        tweets: response_tweets,
    }))
}

/// Strongly-typed video clip for request validation
#[derive(Deserialize)]
struct VideoClipInput {
    source_capture_id: i64,
    start_timestamp: String,
    duration_secs: f64,
}

#[derive(Deserialize)]
struct UpdateCollateralRequest {
    image_capture_ids: Option<Vec<i64>>,
    video_clip: Option<VideoClipInput>,
}

/// PUT /tweets/:id/collateral - Update tweet's media attachments
async fn update_tweet_collateral(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path(tweet_id): Path<i64>,
    Json(payload): Json<UpdateCollateralRequest>,
) -> Result<StatusCode, StatusCode> {

    let exists = threads::verify_tweet_exists_unposted(&state.db, tweet_id, user_id)
        .await
        .log_500("Verify tweet exists error")?;

    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }

    if let Some(ref capture_ids) = payload.image_capture_ids
        && !capture_ids.is_empty()
    {
        let valid = captures::verify_captures_owned(&state.db, capture_ids, user_id)
            .await
            .log_500("Verify captures owned error")?;

        if !valid {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    // Convert typed VideoClipInput to JSON for storage
    let video_clip_json: Option<serde_json::Value> = payload.video_clip.as_ref().map(|vc| {
        serde_json::json!({
            "source_capture_id": vc.source_capture_id,
            "start_timestamp": vc.start_timestamp,
            "duration_secs": vc.duration_secs
        })
    });

    let updated = threads::update_tweet_collateral(
        &state.db,
        tweet_id,
        user_id,
        payload.image_capture_ids.as_ref(),
        video_clip_json.as_ref(),
    )
    .await
    .log_500("Update collateral error")?;

    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(StatusCode::OK)
}
