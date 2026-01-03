use base64::Engine;
use chrono::{DateTime, Utc};
use rand::Rng;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

#[allow(dead_code)]
pub enum TwitterStatsResponse {
    Tweets(Vec<TweetResponse>),
    AggregatedStats(PublicTweetMetrics),
    TweetsWithStats((Vec<TweetResponse>, PublicTweetMetrics)),
}

#[derive(Clone)]
pub struct TwitterClient {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    http: Client,
}

impl TwitterClient {
    pub fn new(client_id: &str, client_secret: &str, redirect_uri: &str) -> Self {
        Self {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            redirect_uri: redirect_uri.to_string(),
            http: Client::new(),
        }
    }

    /// Generate PKCE code verifier and challenge
    fn generate_pkce() -> (String, String) {
        // Generate random 32 bytes for code verifier
        let verifier_bytes: [u8; 32] = rand::rng().random();
        let code_verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);

        // Create code challenge (SHA256 hash of verifier, base64url encoded)
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let hash = hasher.finalize();
        let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);

        (code_verifier, code_challenge)
    }

    /// Generate random state for CSRF protection
    fn generate_state() -> String {
        let bytes: [u8; 16] = rand::rng().random();
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Build Basic auth header for OAuth token requests
    fn basic_auth_header(&self) -> String {
        let credentials = format!("{}:{}", self.client_id, self.client_secret);
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        )
    }

    /// Step 1: Build authorization URL and return state + verifier to store
    pub fn get_authorize_url(&self, scopes: &[&str]) -> AuthorizeRequest {
        let state = Self::generate_state();
        let (code_verifier, code_challenge) = Self::generate_pkce();

        let scope = scopes.join("%20");

        let url = format!(
            "https://x.com/i/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
            percent_encode(&self.client_id),
            percent_encode(&self.redirect_uri),
            scope,
            percent_encode(&state),
            percent_encode(&code_challenge)
        );

        AuthorizeRequest {
            url,
            state,
            code_verifier,
        }
    }

    /// Step 2: Exchange authorization code for access token
    pub async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<TokenResponse, TwitterError> {
        let url = "https://api.x.com/2/oauth2/token";

        let params = [
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", &self.redirect_uri),
            ("code_verifier", code_verifier),
        ];

        let resp = self
            .http
            .post(url)
            .header("Authorization", self.basic_auth_header())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&params)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await?;
            return Err(TwitterError::Api(text));
        }

        let token: TokenResponse = resp.json().await?;
        Ok(token)
    }

    /// Refresh an access token
    pub async fn refresh_token(&self, refresh_token: &str) -> Result<TokenResponse, TwitterError> {
        let url = "https://api.x.com/2/oauth2/token";

        let params = [
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];

        let resp = self
            .http
            .post(url)
            .header("Authorization", self.basic_auth_header())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&params)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await?;
            return Err(TwitterError::Api(text));
        }

        let token: TokenResponse = resp.json().await?;
        Ok(token)
    }

    /// Get the authenticated user's info
    pub async fn get_me(&self, access_token: &str) -> Result<TwitterUser, TwitterError> {
        let url = "https://api.x.com/2/users/me";

        let resp = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await?;
            return Err(TwitterError::Api(text));
        }

        let wrapper: UserResponse = resp.json().await?;
        Ok(wrapper.data)
    }

    /// Post a tweet to Twitter.
    ///
    /// # Arguments
    /// * `access_token` - OAuth 2.0 bearer token for the user
    /// * `text` - The tweet text content
    /// * `in_reply_to` - If posting as part of a thread, the Twitter ID of the previous tweet to chain to
    /// * `media_ids` - Twitter media IDs to attach (uploaded via `upload_media`). Max 4 images OR 1 video.
    pub async fn post_tweet(
        &self,
        access_token: &str,
        text: &str,
        in_reply_to: Option<&str>,
        media_ids: Option<&[String]>,
    ) -> Result<TweetResponse, TwitterError> {
        let url = "https://api.x.com/2/tweets";

        let mut body = serde_json::json!({ "text": text });

        if let Some(parent_id) = in_reply_to {
            body["reply"] = serde_json::json!({
                "in_reply_to_tweet_id": parent_id
            });
        }

        if let Some(ids) = media_ids {
            if !ids.is_empty() {
                body["media"] = serde_json::json!({
                    "media_ids": ids
                });
            }
        }

        let resp = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await?;
            return Err(TwitterError::Api(text));
        }

        let wrapper: TweetResponseWrapper = resp.json().await?;
        Ok(wrapper.data)
    }

    #[allow(dead_code)]
    pub async fn get_tweet(
        &self,
        access_token: &str,
        tweet_id: &str,
    ) -> Result<TweetResponse, TwitterError> {
        let url = format!("https://api.x.com/2/tweets/{tweet_id}&tweet.fields=public_metrics");

        let resp = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await?;
            return Err(TwitterError::Api(text));
        }

        let wrapper: TweetResponseWrapper = resp.json().await?;

        Ok(wrapper.data)
    }

    #[allow(dead_code)]
    pub async fn get_tweets_with_stats(
        &self,
        access_token: &str,
        tweet_ids: Vec<&str>,
        with_aggregated: bool,
    ) -> Result<TwitterStatsResponse, TwitterError> {
        let tweet_id_str = tweet_ids.join(",");

        let url =
            format!("https://api.x.com/2/tweets?ids={tweet_id_str}&tweet.fields=public_metrics");

        let resp = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await?;
            return Err(TwitterError::Api(text));
        }

        let wrapper: TweetListResponseWrapper = resp.json().await?;

        if with_aggregated {
            let aggregated_metrics =
                wrapper
                    .data
                    .iter()
                    .fold(PublicTweetMetrics::default(), |mut acc, d| {
                        match &d.public_metrics {
                            Some(pub_met) => {
                                acc.like_count += pub_met.like_count;
                                acc.quote_count += pub_met.quote_count;
                                acc.reply_count += pub_met.reply_count;
                                acc.retweet_count += pub_met.retweet_count;

                                acc
                            }
                            None => acc,
                        }
                    });

            Ok(TwitterStatsResponse::TweetsWithStats((
                wrapper.data,
                aggregated_metrics,
            )))
        } else {
            Ok(TwitterStatsResponse::Tweets(wrapper.data))
        }
    }


    /// Upload media to Twitter using the v2 API
    /// For images: uses simple upload
    /// For videos: uses chunked upload (INIT/APPEND/FINALIZE)
    pub async fn upload_media(
        &self,
        access_token: &str,
        data: &[u8],
        media_type: &str,
    ) -> Result<String, TwitterError> {
        // Videos require chunked upload
        if media_type.starts_with("video/") {
            return self.upload_media_chunked(access_token, data, media_type).await;
        }

        // Simple upload for images
        let url = "https://api.x.com/2/media/upload";

        let media_category = if media_type == "image/gif" {
            "tweet_gif"
        } else {
            "tweet_image"
        };

        let part = reqwest::multipart::Part::bytes(data.to_vec())
            .mime_str(media_type)
            .map_err(|e| TwitterError::Api(format!("Invalid mime type: {}", e)))?;

        let form = reqwest::multipart::Form::new()
            .text("media_category", media_category.to_string())
            .text("media_type", media_type.to_string())
            .part("media", part);

        let resp = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {}", access_token))
            .multipart(form)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            return Err(TwitterError::Api(format!("Status {}: {}", status, text)));
        }

        let wrapper: MediaUploadResponse = serde_json::from_str(&text)
            .map_err(|e| TwitterError::Api(format!("Failed to parse response: {} - body: {}", e, text)))?;
        Ok(wrapper.data.id)
    }

    /// Upload media using chunked upload via dedicated v2 endpoints
    /// Required for videos, works for any media type
    async fn upload_media_chunked(
        &self,
        access_token: &str,
        data: &[u8],
        media_type: &str,
    ) -> Result<String, TwitterError> {
        self.upload_media_chunked_with_progress(access_token, data, media_type, |_, _| {})
            .await
    }

    /// Upload media with progress callback
    /// Callback receives (current_segment, total_segments)
    pub async fn upload_media_chunked_with_progress<F>(
        &self,
        access_token: &str,
        data: &[u8],
        media_type: &str,
        on_progress: F,
    ) -> Result<String, TwitterError>
    where
        F: Fn(usize, usize),
    {
        // Twitter v2 API doesn't accept video/quicktime, map to mp4
        let media_type = if media_type == "video/quicktime" {
            "video/mp4"
        } else {
            media_type
        };

        let media_category = if media_type.starts_with("video/") {
            "tweet_video"
        } else if media_type == "image/gif" {
            "tweet_gif"
        } else {
            "tweet_image"
        };

        // Step 1: INIT via /2/media/upload/initialize (JSON body)
        println!("[upload_media_chunked] INIT: media_type={}, total_bytes={}, media_category={}",
            media_type, data.len(), media_category);

        let init_body = serde_json::json!({
            "media_type": media_type,
            "total_bytes": data.len(),
            "media_category": media_category
        });

        let resp = self
            .http
            .post("https://api.x.com/2/media/upload/initialize")
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&init_body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            return Err(TwitterError::Api(format!("INIT failed - Status {}: {}", status, text)));
        }

        let init_response: MediaUploadResponse = serde_json::from_str(&text)
            .map_err(|e| TwitterError::Api(format!("Failed to parse INIT response: {} - body: {}", e, text)))?;
        let media_id = init_response.data.id;

        println!("[upload_media_chunked] Got media_id: {}", media_id);

        // Step 2: APPEND via /2/media/upload/{media_id}/append (multipart)
        const CHUNK_SIZE: usize = 1 * 1024 * 1024; // 1MB
        let chunks: Vec<_> = data.chunks(CHUNK_SIZE).collect();
        let total_segments = chunks.len();

        for (segment_index, chunk) in chunks.into_iter().enumerate() {
            println!("[upload_media_chunked] APPEND segment {}/{} ({} bytes)", segment_index + 1, total_segments, chunk.len());

            // Report progress before uploading segment
            on_progress(segment_index, total_segments);

            let part = reqwest::multipart::Part::bytes(chunk.to_vec())
                .mime_str(media_type)
                .map_err(|e| TwitterError::Api(format!("Invalid mime type: {}", e)))?;

            let append_form = reqwest::multipart::Form::new()
                .text("segment_index", segment_index.to_string())
                .part("media", part);

            let resp = self
                .http
                .post(format!("https://api.x.com/2/media/upload/{}/append", media_id))
                .header("Authorization", format!("Bearer {}", access_token))
                .multipart(append_form)
                .send()
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await?;
                return Err(TwitterError::Api(format!("APPEND failed at segment {} - Status {}: {}", segment_index, status, text)));
            }
        }

        // Final progress update
        on_progress(total_segments, total_segments);

        // Step 3: FINALIZE via /2/media/upload/{media_id}/finalize
        println!("[upload_media_chunked] FINALIZE");

        let resp = self
            .http
            .post(format!("https://api.x.com/2/media/upload/{}/finalize", media_id))
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            return Err(TwitterError::Api(format!("FINALIZE failed - Status {}: {}", status, text)));
        }

        let finalize_response: MediaUploadResponse = serde_json::from_str(&text)
            .map_err(|e| TwitterError::Api(format!("Failed to parse FINALIZE response: {} - body: {}", e, text)))?;

        // Step 4: Poll STATUS if processing is needed
        if let Some(ref processing_info) = finalize_response.data.processing_info {
            if processing_info.state != "succeeded" {
                self.wait_for_processing(access_token, &media_id).await?;
            }
        }

        println!("[upload_media_chunked] Complete, media_id: {}", media_id);
        Ok(media_id)
    }

    /// Poll the STATUS endpoint until processing completes
    async fn wait_for_processing(
        &self,
        access_token: &str,
        media_id: &str,
    ) -> Result<(), TwitterError> {
        let url = format!(
            "https://api.x.com/2/media/upload?command=STATUS&media_id={}",
            media_id
        );

        loop {
            let resp = self
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {}", access_token))
                .send()
                .await?;

            let status = resp.status();
            let text = resp.text().await?;

            if !status.is_success() {
                return Err(TwitterError::Api(format!("STATUS check failed - Status {}: {}", status, text)));
            }

            let status_response: MediaUploadResponse = serde_json::from_str(&text)
                .map_err(|e| TwitterError::Api(format!("Failed to parse STATUS response: {} - body: {}", e, text)))?;

            if let Some(processing_info) = status_response.data.processing_info {
                match processing_info.state.as_str() {
                    "succeeded" => return Ok(()),
                    "failed" => return Err(TwitterError::Api("Media processing failed".to_string())),
                    _ => {
                        // Wait before polling again
                        let wait_secs = processing_info.check_after_secs.unwrap_or(5);
                        tokio::time::sleep(tokio::time::Duration::from_secs(wait_secs as u64)).await;
                    }
                }
            } else {
                // No processing_info means it's done
                return Ok(());
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct MediaUploadResponse {
    data: MediaUploadData,
}

#[derive(Debug, Deserialize)]
struct MediaUploadData {
    id: String,
    #[allow(dead_code)]
    media_key: Option<String>,
    #[allow(dead_code)]
    expires_after_secs: Option<i64>,
    processing_info: Option<MediaProcessingInfo>,
}

#[derive(Debug, Deserialize)]
struct MediaProcessingInfo {
    state: String,
    #[allow(dead_code)]
    progress_percent: Option<i32>,
    check_after_secs: Option<i32>,
}

fn percent_encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

#[derive(Debug)]
pub struct AuthorizeRequest {
    pub url: String,
    pub state: String,
    pub code_verifier: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub refresh_token: Option<String>,
    pub scope: String,
}

#[derive(Debug, Deserialize)]
struct UserResponse {
    data: TwitterUser,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TwitterUser {
    pub id: String,
    pub name: String,
    pub username: String,
}

#[derive(Debug, Deserialize)]
struct TweetResponseWrapper {
    data: TweetResponse,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct TweetListResponseWrapper {
    data: Vec<TweetResponse>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PublicTweetMetrics {
    pub retweet_count: i64,
    pub reply_count: i64,
    pub like_count: i64,
    pub quote_count: i64,
}

impl Default for PublicTweetMetrics {
    fn default() -> Self {
        Self {
            like_count: 0,
            retweet_count: 0,
            reply_count: 0,
            quote_count: 0,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TweetResponse {
    pub id: String,
    pub text: String,
    pub public_metrics: Option<PublicTweetMetrics>,
}

#[derive(Debug)]
pub enum TwitterError {
    Http(reqwest::Error),
    Api(String),
}

impl From<reqwest::Error> for TwitterError {
    fn from(e: reqwest::Error) -> Self {
        TwitterError::Http(e)
    }
}

impl std::fmt::Display for TwitterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TwitterError::Http(e) => write!(f, "HTTP error: {}", e),
            TwitterError::Api(s) => write!(f, "Twitter API error: {}", s),
        }
    }
}

impl std::error::Error for TwitterError {}

// Database operations

#[derive(Debug, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    #[allow(dead_code)] // Fetched from DB but intentionally not exposed in API responses
    pub twitter_id: String,
    pub twitter_username: String,
    pub twitter_name: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn save_oauth_state(
    db: &PgPool,
    state: &str,
    code_verifier: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO oauth_states (state, code_verifier)
        VALUES ($1, $2)
        "#,
    )
    .bind(state)
    .bind(code_verifier)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn get_oauth_state(db: &PgPool, state: &str) -> Result<Option<String>, sqlx::Error> {
    // Atomic DELETE + RETURNING prevents race conditions where two requests
    // could get the same state before either deletes it
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        DELETE FROM oauth_states
        WHERE state = $1 AND created_at > NOW() - INTERVAL '10 minutes'
        RETURNING code_verifier
        "#,
    )
    .bind(state)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|r| r.0))
}

pub async fn upsert_user(
    db: &PgPool,
    twitter_id: &str,
    twitter_username: &str,
    twitter_name: Option<&str>,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO users (twitter_id, twitter_username, twitter_name, access_token, refresh_token, token_expires_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (twitter_id) DO UPDATE SET
            twitter_username = $2,
            twitter_name = $3,
            access_token = $4,
            refresh_token = COALESCE($5, users.refresh_token),
            token_expires_at = $6,
            updated_at = NOW()
        RETURNING id
        "#,
    )
    .bind(twitter_id)
    .bind(twitter_username)
    .bind(twitter_name)
    .bind(access_token)
    .bind(refresh_token)
    .bind(expires_at)
    .fetch_one(db)
    .await?;

    Ok(row.0)
}

pub async fn get_user_by_id(db: &PgPool, user_id: i64) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT id, twitter_id, twitter_username, twitter_name, created_at
        FROM users WHERE id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct UserTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_expires_at: DateTime<Utc>,
}

pub async fn get_user_tokens(db: &PgPool, user_id: i64) -> Result<Option<UserTokens>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT access_token, refresh_token, token_expires_at
        FROM users WHERE id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

pub async fn update_user_tokens(
    db: &PgPool,
    user_id: i64,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE users SET
            access_token = $2,
            refresh_token = COALESCE($3, refresh_token),
            token_expires_at = $4,
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(user_id)
    .bind(access_token)
    .bind(refresh_token)
    .bind(expires_at)
    .execute(db)
    .await?;
    Ok(())
}

/// Generate a new API token for a user
pub fn generate_api_token() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    format!(
        "cleo_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

/// Set a user's API token
pub async fn set_user_api_token(
    db: &PgPool,
    user_id: i64,
    api_token: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE users SET api_token = $2, updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(user_id)
    .bind(api_token)
    .execute(db)
    .await?;
    Ok(())
}

/// Get user ID by API token (for bearer auth)
pub async fn get_user_by_api_token(
    db: &PgPool,
    api_token: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let row: Option<(i64,)> = sqlx::query_as(
        r#"
        SELECT id FROM users WHERE api_token = $1
        "#,
    )
    .bind(api_token)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.0))
}

/// Get a user's current API token
pub async fn get_user_api_token(db: &PgPool, user_id: i64) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        r#"
        SELECT api_token FROM users WHERE id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|r| r.0))
}
