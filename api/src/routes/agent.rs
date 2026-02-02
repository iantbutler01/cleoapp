use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use std::sync::Arc;

use crate::{agent, AppState};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/agent/trigger", post(trigger_agent))
}

/// Manually trigger the agent for testing
async fn trigger_agent(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, StatusCode> {
    let gemini = state.gemini.as_ref().ok_or_else(|| {
        eprintln!("[agent/trigger] No Gemini client configured");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Hardcode user_id 1 for testing
    let user_id = 1i64;

    println!("[agent/trigger] Manually triggering agent for user {}", user_id);

    match agent::run_collateral_job(
        state.db.clone(),
        state.gcs.clone(),
        gemini.clone(),
        user_id,
        state.local_storage_path.clone(),
    )
    .await
    {
        Ok(tweets) => {
            println!("[agent/trigger] Generated {} tweets", tweets.len());
            Ok(Json(serde_json::json!({
                "success": true,
                "tweets_generated": tweets.len()
            })))
        }
        Err(e) => {
            eprintln!("[agent/trigger] Error: {}", e);
            Ok(Json(serde_json::json!({
                "success": false,
                "error": e.to_string()
            })))
        }
    }
}
