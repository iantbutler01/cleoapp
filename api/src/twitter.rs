use base64::Engine;
use chrono::{DateTime, Utc};
use rand::Rng;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

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

        // Build Basic auth header for confidential client
        let credentials = format!("{}:{}", self.client_id, self.client_secret);
        let auth_header = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        );

        let params = [
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", &self.redirect_uri),
            ("code_verifier", code_verifier),
        ];

        let resp = self
            .http
            .post(url)
            .header("Authorization", auth_header)
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
    pub async fn refresh_token(
        &self,
        refresh_token: &str,
    ) -> Result<TokenResponse, TwitterError> {
        let url = "https://api.x.com/2/oauth2/token";

        let credentials = format!("{}:{}", self.client_id, self.client_secret);
        let auth_header = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        );

        let params = [
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];

        let resp = self
            .http
            .post(url)
            .header("Authorization", auth_header)
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

    /// Post a tweet
    pub async fn post_tweet(
        &self,
        access_token: &str,
        text: &str,
    ) -> Result<TweetResponse, TwitterError> {
        let url = "https://api.x.com/2/tweets";

        let body = serde_json::json!({ "text": text });

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
}

fn percent_encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(
        s,
        percent_encoding::NON_ALPHANUMERIC,
    )
    .to_string()
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

#[derive(Debug, Deserialize, Serialize)]
pub struct TweetResponse {
    pub id: String,
    pub text: String,
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

#[derive(Debug, sqlx::FromRow, Serialize)]
pub struct User {
    pub id: i64,
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
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT code_verifier FROM oauth_states
        WHERE state = $1 AND created_at > NOW() - INTERVAL '10 minutes'
        "#,
    )
    .bind(state)
    .fetch_optional(db)
    .await?;

    // Clean up the used state
    sqlx::query("DELETE FROM oauth_states WHERE state = $1")
        .bind(state)
        .execute(db)
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
