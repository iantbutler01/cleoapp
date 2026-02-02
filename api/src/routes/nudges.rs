//! Nudges and personas endpoints for user voice/style customization

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get},
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use std::sync::Arc;

use crate::AppState;
use super::auth::AuthUser;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // System personas (public, no auth required)
        .route("/personas", get(list_system_personas))
        // User's custom personas
        .route("/me/personas", get(list_user_personas).post(create_user_persona))
        .route("/me/personas/{id}", delete(delete_user_persona))
        // User's active nudges
        .route("/me/nudges", get(get_nudges).put(update_nudges))
}

// ============================================================================
// DTOs
// ============================================================================

#[derive(Debug, Serialize, FromRow)]
pub struct PersonaResponse {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub nudges: String,
}

#[derive(Debug, Serialize, FromRow)]
pub struct UserPersonaResponse {
    pub id: i64,
    pub name: String,
    pub nudges: String,
}

#[derive(Debug, Serialize)]
pub struct NudgesResponse {
    pub nudges: Option<String>,
    pub selected_persona_id: Option<i64>,
}

#[derive(Debug, FromRow)]
struct NudgesRow {
    nudges: Option<String>,
    selected_persona_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePersonaRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNudgesRequest {
    pub nudges: String,
    pub selected_persona_id: Option<i64>,
}

// ============================================================================
// System Personas
// ============================================================================

/// GET /personas - List all system personas
async fn list_system_personas(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<PersonaResponse>>, StatusCode> {
    let personas = sqlx::query_as::<_, PersonaResponse>(
        r#"
        SELECT id, name, slug, nudges
        FROM personas
        WHERE is_system = true
        ORDER BY name
        "#
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        eprintln!("Failed to list personas: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(personas))
}

// ============================================================================
// User's Custom Personas
// ============================================================================

/// GET /me/personas - List user's custom personas
async fn list_user_personas(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
) -> Result<Json<Vec<UserPersonaResponse>>, StatusCode> {
    let personas = sqlx::query_as::<_, UserPersonaResponse>(
        r#"
        SELECT id, name, nudges
        FROM user_personas
        WHERE user_id = $1
        ORDER BY created_at DESC
        "#
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        eprintln!("Failed to list user personas: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(personas))
}

/// POST /me/personas - Save current nudges as a new custom persona
async fn create_user_persona(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Json(req): Json<CreatePersonaRequest>,
) -> Result<Json<UserPersonaResponse>, StatusCode> {
    // Get user's current nudges
    let current_nudges: Option<String> = sqlx::query_scalar(
        "SELECT nudges FROM users WHERE id = $1"
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        eprintln!("Failed to get user nudges: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let nudges = current_nudges.ok_or_else(|| {
        eprintln!("Cannot save persona: user has no nudges set");
        StatusCode::BAD_REQUEST
    })?;

    // Create the persona
    let persona = sqlx::query_as::<_, UserPersonaResponse>(
        r#"
        INSERT INTO user_personas (user_id, name, nudges)
        VALUES ($1, $2, $3)
        RETURNING id, name, nudges
        "#
    )
    .bind(user_id)
    .bind(&req.name)
    .bind(&nudges)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        eprintln!("Failed to create user persona: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(persona))
}

/// DELETE /me/personas/:id - Delete a custom persona
async fn delete_user_persona(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path(persona_id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    let result = sqlx::query(
        "DELETE FROM user_personas WHERE id = $1 AND user_id = $2"
    )
    .bind(persona_id)
    .bind(user_id)
    .execute(&state.db)
    .await
    .map_err(|e| {
        eprintln!("Failed to delete user persona: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// Nudges
// ============================================================================

/// GET /me/nudges - Get user's current nudges and selected persona
async fn get_nudges(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
) -> Result<Json<NudgesResponse>, StatusCode> {
    let row = sqlx::query_as::<_, NudgesRow>(
        "SELECT nudges, selected_persona_id FROM users WHERE id = $1"
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        eprintln!("Failed to get nudges: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(NudgesResponse {
        nudges: row.nudges,
        selected_persona_id: row.selected_persona_id,
    }))
}

/// PUT /me/nudges - Update user's nudges
async fn update_nudges(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Json(req): Json<UpdateNudgesRequest>,
) -> Result<Json<NudgesResponse>, StatusCode> {
    // Sanitize nudges
    let sanitized = sanitize_nudges(&req.nudges);

    sqlx::query(
        "UPDATE users SET nudges = $1, selected_persona_id = $2 WHERE id = $3"
    )
    .bind(&sanitized)
    .bind(req.selected_persona_id)
    .bind(user_id)
    .execute(&state.db)
    .await
    .map_err(|e| {
        eprintln!("Failed to update nudges: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(NudgesResponse {
        nudges: Some(sanitized),
        selected_persona_id: req.selected_persona_id,
    }))
}

// ============================================================================
// Prompt Injection Protection
// ============================================================================

/// Sanitize user nudges to prevent prompt injection
fn sanitize_nudges(input: &str) -> String {
    // Max length: 2000 chars
    let s: String = input.chars().take(2000).collect();

    // Normalize whitespace
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");

    // Log suspicious patterns (don't block, just monitor)
    let suspicious = ["ignore previous", "system prompt", "you are now", "disregard", "forget your instructions"];
    for pattern in suspicious {
        if s.to_lowercase().contains(pattern) {
            eprintln!("[security] Suspicious nudge pattern detected: {}", pattern);
        }
    }

    s
}

/// Get sanitized nudges for a user (used by agent)
pub async fn get_sanitized_nudges(db: &PgPool, user_id: i64) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT nudges FROM users WHERE id = $1"
    )
    .bind(user_id)
    .fetch_one(db)
    .await
    .ok()
    .flatten()
    .map(|n| sanitize_nudges(&n))
}
