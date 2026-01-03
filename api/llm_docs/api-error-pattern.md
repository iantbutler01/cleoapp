# Unified API Error Type Pattern

This document outlines a future improvement to standardize error handling across the API.

## Current State

All handlers return `Result<T, StatusCode>` with error context discarded:

```rust
.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?  // Error context lost
.map_err(|_| StatusCode::UNAUTHORIZED)?           // No distinction between errors
```

**Problems:**
- Lost error context - can't log what actually went wrong
- No structured error responses for clients
- Inconsistent error messages
- Difficult debugging in production

## Proposed Solution

Create an `ApiError` enum that implements `IntoResponse`:

```rust
// api/src/error.rs

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Debug)]
pub enum ApiError {
    /// 404 - Resource not found
    NotFound(String),

    /// 401 - Authentication required or failed
    Unauthorized,

    /// 400 - Invalid request data
    BadRequest(String),

    /// 500 - Internal server error (logs the actual error)
    Internal(String),

    /// 403 - Authenticated but not authorized
    Forbidden,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match &self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg.clone()),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", "Authentication required".into()),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "bad_request", msg.clone()),
            ApiError::Internal(msg) => {
                // Log the actual error server-side
                tracing::error!("Internal error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", "An internal error occurred".into())
            }
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "Access denied".into()),
        };

        let body = Json(ErrorResponse {
            error: error_type.into(),
            message,
        });

        (status, body).into_response()
    }
}
```

## Usage Examples

### Before (current)
```rust
async fn get_tweet(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Tweet>, StatusCode> {
    let tweet = sqlx::query_as::<_, Tweet>("SELECT * FROM tweets WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?  // What went wrong?
        .ok_or(StatusCode::NOT_FOUND)?;  // Generic 404

    Ok(Json(tweet))
}
```

### After (proposed)
```rust
async fn get_tweet(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Tweet>, ApiError> {
    let tweet = sqlx::query_as::<_, Tweet>("SELECT * FROM tweets WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::Internal(format!("Database error fetching tweet {}: {}", id, e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Tweet {} not found", id)))?;

    Ok(Json(tweet))
}
```

## Client Response Format

All errors return JSON with consistent structure:

```json
{
  "error": "not_found",
  "message": "Tweet 123 not found"
}
```

## Implementation Steps

1. Add `tracing` crate to dependencies (for structured logging)
2. Create `api/src/error.rs` with the `ApiError` enum
3. Add `mod error;` to `api/src/lib.rs` or `main.rs`
4. Gradually migrate handlers from `Result<T, StatusCode>` to `Result<T, ApiError>`
5. Start with new routes, then migrate existing ones

## Benefits

- **Debugging**: Actual errors logged server-side with context
- **Consistency**: Clients always get the same error format
- **Type safety**: Compiler ensures all error cases are handled
- **Extensibility**: Easy to add new error types (e.g., `RateLimited`, `ValidationError`)

## Priority

Medium - Not blocking any features, but improves maintainability and debugging. Consider implementing when:
- Adding significant new routes
- Debugging production issues becomes painful
- Building client SDKs that need predictable errors
