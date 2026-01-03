//! Session management: JWT access tokens and refresh tokens

#![allow(dead_code)] // Functions will be used as we implement auth

use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// JWT claims for access tokens
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // user_id as string
    pub exp: i64,    // expiry timestamp
    pub iat: i64,    // issued at
}

#[derive(Debug)]
pub enum SessionError {
    InvalidToken,
    Expired,
    DatabaseError(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::InvalidToken => write!(f, "Invalid token"),
            SessionError::Expired => write!(f, "Token expired"),
            SessionError::DatabaseError(e) => write!(f, "Database error: {}", e),
        }
    }
}

const ACCESS_TOKEN_EXPIRY_MINUTES: i64 = 10;
const REFRESH_TOKEN_EXPIRY_DAYS: i64 = 30;

/// Create a JWT access token valid for 10 minutes
pub fn create_access_token(user_id: i64, secret: &[u8]) -> Result<String, SessionError> {
    let now = Utc::now();
    let exp = now + Duration::minutes(ACCESS_TOKEN_EXPIRY_MINUTES);

    let claims = Claims {
        sub: user_id.to_string(),
        exp: exp.timestamp(),
        iat: now.timestamp(),
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .map_err(|_| SessionError::InvalidToken)
}

/// Validate a JWT access token and return the user_id
pub fn validate_access_token(token: &str, secret: &[u8]) -> Result<i64, SessionError> {
    // Explicitly validate with HS256 algorithm only to prevent algorithm confusion attacks
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["exp", "sub", "iat"]);

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret),
        &validation,
    )
    .map_err(|e| {
        eprintln!("JWT decode error: {:?}", e);
        match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => SessionError::Expired,
            _ => SessionError::InvalidToken,
        }
    })?;

    token_data.claims.sub.parse::<i64>().map_err(|_| SessionError::InvalidToken)
}

/// Create a random refresh token and store it in the database
pub async fn create_refresh_token(user_id: i64, db: &PgPool) -> Result<String, SessionError> {
    // Generate a random 32-byte token as hex
    // Use thread_rng().random() in a non-async block since ThreadRng is not Send
    let token = {
        use rand::Rng;
        let bytes: [u8; 32] = rand::rng().random();
        hex::encode(bytes.as_slice())
    };

    let expires_at = Utc::now() + Duration::days(REFRESH_TOKEN_EXPIRY_DAYS);

    sqlx::query(
        r#"
        INSERT INTO refresh_tokens (id, user_id, expires_at)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(&token)
    .bind(user_id)
    .bind(expires_at)
    .execute(db)
    .await
    .map_err(|e| SessionError::DatabaseError(e.to_string()))?;

    Ok(token)
}

/// Rotate a refresh token: validate the old token, delete it, and create a new one.
/// Returns (user_id, new_refresh_token) on success.
/// This prevents token reuse attacks - each refresh token can only be used once.
/// Uses a transaction to ensure the user isn't logged out if new token creation fails.
pub async fn rotate_refresh_token(
    old_token: &str,
    db: &PgPool,
) -> Result<(i64, String), SessionError> {
    let now = Utc::now();

    // Use a transaction to ensure atomicity:
    // If new token creation fails, the old token is NOT deleted
    let mut tx = db
        .begin()
        .await
        .map_err(|e| SessionError::DatabaseError(e.to_string()))?;

    // Check and delete the old token atomically to prevent race conditions
    // If two requests try to use the same token, only one will succeed
    let row: Option<(i64,)> = sqlx::query_as(
        r#"
        DELETE FROM refresh_tokens
        WHERE id = $1 AND expires_at > $2
        RETURNING user_id
        "#,
    )
    .bind(old_token)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| SessionError::DatabaseError(e.to_string()))?;

    let user_id = row.ok_or(SessionError::InvalidToken)?.0;

    // Generate new token
    let new_token = {
        use rand::Rng;
        let bytes: [u8; 32] = rand::rng().random();
        hex::encode(bytes.as_slice())
    };
    let expires_at = Utc::now() + chrono::Duration::days(REFRESH_TOKEN_EXPIRY_DAYS);

    // Insert new token within the same transaction
    sqlx::query(
        r#"
        INSERT INTO refresh_tokens (id, user_id, expires_at)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(&new_token)
    .bind(user_id)
    .bind(expires_at)
    .execute(&mut *tx)
    .await
    .map_err(|e| SessionError::DatabaseError(e.to_string()))?;

    // Commit - if this fails, both operations are rolled back
    tx.commit()
        .await
        .map_err(|e| SessionError::DatabaseError(e.to_string()))?;

    Ok((user_id, new_token))
}

/// Delete a specific refresh token (logout from one device)
pub async fn revoke_refresh_token(token: &str, db: &PgPool) -> Result<(), SessionError> {
    sqlx::query("DELETE FROM refresh_tokens WHERE id = $1")
        .bind(token)
        .execute(db)
        .await
        .map_err(|e| SessionError::DatabaseError(e.to_string()))?;

    Ok(())
}

/// Delete all refresh tokens for a user (logout everywhere)
pub async fn revoke_all_user_tokens(user_id: i64, db: &PgPool) -> Result<(), SessionError> {
    sqlx::query("DELETE FROM refresh_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(db)
        .await
        .map_err(|e| SessionError::DatabaseError(e.to_string()))?;

    Ok(())
}

/// Clean up expired refresh tokens (call periodically via cron)
pub async fn cleanup_expired_tokens(db: &PgPool) -> Result<u64, SessionError> {
    let result = sqlx::query("DELETE FROM refresh_tokens WHERE expires_at < NOW()")
        .execute(db)
        .await
        .map_err(|e| SessionError::DatabaseError(e.to_string()))?;

    Ok(result.rows_affected())
}

// Hex encoding helper since we don't want to add another dependency
mod hex {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

    pub fn encode(bytes: &[u8]) -> String {
        let mut result = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            result.push(HEX_CHARS[(byte >> 4) as usize] as char);
            result.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
        }
        result
    }
}
