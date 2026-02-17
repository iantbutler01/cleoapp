use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::Serialize;
use std::sync::Arc;

use super::auth::AuthUser;
use crate::AppState;
use crate::agent;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/agent/run", post(trigger_run))
        .route("/agent/status", get(run_status))
}

#[derive(Serialize)]
struct RunResponse {
    status: &'static str,
    run_id: Option<i64>,
}

/// POST /agent/run - trigger an immediate agent run for the current user
async fn trigger_run(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
) -> Result<Json<RunResponse>, StatusCode> {
    let db = state.db.clone();
    let gcs = state.gcs.clone();
    let gemini = state.gemini.clone();
    let local_storage_path = state.local_storage_path.clone();

    tokio::spawn(async move {
        match agent::run_collateral_job(db, gcs, gemini, user_id, local_storage_path).await {
            Ok(tweets) => {
                println!(
                    "[agent/run] User {} - manual run generated {} tweets",
                    user_id,
                    tweets.len()
                );
            }
            Err(e) => {
                eprintln!("[agent/run] User {} - manual run error: {}", user_id, e);
            }
        }
    });

    Ok(Json(RunResponse {
        status: "started",
        run_id: None,
    }))
}

#[derive(Serialize)]
struct StatusResponse {
    running: bool,
}

/// GET /agent/status - check if an agent run is currently active for this user
async fn run_status(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
) -> Result<Json<StatusResponse>, StatusCode> {
    let running = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE user_id = $1 AND status = 'running' AND started_at > NOW() - INTERVAL '30 minutes')"
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        eprintln!("[agent/status] DB error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(StatusResponse { running }))
}
