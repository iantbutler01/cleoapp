//! Media Studio WebSocket routes for interactive editing

use axum::{
    Json, Router,
    extract::{State, WebSocketUpgrade, ws::{Message, WebSocket}},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;
use crate::routes::auth::AuthUser;
use crate::services::media_studio::{CropParams, MediaStudio, MediaStudioError, TrimParams};
use crate::services::session;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/media/edit/ws", get(edit_ws))
        .route("/media/crop", post(crop_image))
        .route("/media/trim", post(trim_video))
}

/// WebSocket command from client
#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
enum EditCommand {
    #[serde(rename = "crop")]
    Crop {
        capture_id: i64,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
    #[serde(rename = "trim")]
    Trim {
        capture_id: i64,
        start: String,
        duration: f64,
    },
}

/// WebSocket response to client
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum EditResponse {
    #[serde(rename = "progress")]
    Progress { percent: u8, status: String },
    #[serde(rename = "complete")]
    Complete { new_capture_id: i64 },
    #[serde(rename = "error")]
    Error { message: String },
}

/// GET /media/edit/ws - WebSocket for interactive editing
async fn edit_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Result<impl IntoResponse, StatusCode> {
    // Validate JWT from cookie
    let access_token = jar
        .get("access_token")
        .map(|c| c.value())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let user_id = session::validate_access_token(access_token, &state.jwt_secret)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    Ok(ws.on_upgrade(move |socket| handle_edit_ws(socket, state, user_id)))
}

async fn handle_edit_ws(socket: WebSocket, state: Arc<AppState>, user_id: i64) {
    let (mut sender, mut receiver) = socket.split();

    // Create MediaStudio instance
    let media_studio = MediaStudio::new(
        state.db.clone(),
        state.gcs.clone(),
        state.local_storage_path.clone(),
    );

    // Process commands from client
    while let Some(msg) = receiver.next().await {
        let msg = match msg {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) => break,
            Ok(_) => continue, // Ignore binary, ping, pong
            Err(e) => {
                eprintln!("[media_studio_ws] WebSocket error: {}", e);
                break;
            }
        };

        // Parse command
        let cmd: EditCommand = match serde_json::from_str(&msg) {
            Ok(cmd) => cmd,
            Err(e) => {
                let response = EditResponse::Error {
                    message: format!("Invalid command: {}", e),
                };
                let json = serde_json::to_string(&response).unwrap();
                let _ = sender.send(Message::Text(json.into())).await;
                continue;
            }
        };

        // Send progress (start)
        let progress = EditResponse::Progress {
            percent: 0,
            status: "Starting...".into(),
        };
        let _ = sender.send(Message::Text(serde_json::to_string(&progress).unwrap().into())).await;

        // Execute command
        let result = match cmd {
            EditCommand::Crop { capture_id, x, y, width, height } => {
                // Send progress (processing)
                let progress = EditResponse::Progress {
                    percent: 30,
                    status: "Cropping image...".into(),
                };
                let _ = sender.send(Message::Text(serde_json::to_string(&progress).unwrap().into())).await;

                media_studio
                    .crop_image(user_id, capture_id, CropParams { x, y, width, height })
                    .await
            }
            EditCommand::Trim { capture_id, start, duration } => {
                // Send progress (processing)
                let progress = EditResponse::Progress {
                    percent: 30,
                    status: "Trimming video...".into(),
                };
                let _ = sender.send(Message::Text(serde_json::to_string(&progress).unwrap().into())).await;

                media_studio
                    .trim_video(user_id, capture_id, TrimParams {
                        start_timestamp: start,
                        duration_secs: duration,
                    })
                    .await
            }
        };

        // Send result
        let response = match result {
            Ok(new_capture_id) => EditResponse::Complete { new_capture_id },
            Err(e) => EditResponse::Error { message: e.to_string() },
        };

        let json = serde_json::to_string(&response).unwrap();
        if sender.send(Message::Text(json.into())).await.is_err() {
            break;
        }
    }

    let _ = sender.close().await;
}

// ============== REST endpoints for simple operations ==============

#[derive(Debug, Deserialize)]
struct CropRequest {
    capture_id: i64,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Deserialize)]
struct TrimRequest {
    capture_id: i64,
    start: String,
    duration: f64,
}

#[derive(Debug, Serialize)]
struct EditResult {
    new_capture_id: i64,
}

/// POST /media/crop - Crop an image (REST endpoint for agent use)
async fn crop_image(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Json(req): Json<CropRequest>,
) -> Result<Json<EditResult>, StatusCode> {
    let media_studio = MediaStudio::new(
        state.db.clone(),
        state.gcs.clone(),
        state.local_storage_path.clone(),
    );

    let new_capture_id = media_studio
        .crop_image(user_id, req.capture_id, CropParams {
            x: req.x,
            y: req.y,
            width: req.width,
            height: req.height,
        })
        .await
        .map_err(|e| {
            eprintln!("[media_studio] Crop error: {}", e);
            match e {
                MediaStudioError::NotFound => StatusCode::NOT_FOUND,
                MediaStudioError::InvalidParams(_) => StatusCode::BAD_REQUEST,
                MediaStudioError::InvalidMediaType(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            }
        })?;

    Ok(Json(EditResult { new_capture_id }))
}

/// POST /media/trim - Trim a video (REST endpoint for agent use)
async fn trim_video(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Json(req): Json<TrimRequest>,
) -> Result<Json<EditResult>, StatusCode> {
    let media_studio = MediaStudio::new(
        state.db.clone(),
        state.gcs.clone(),
        state.local_storage_path.clone(),
    );

    let new_capture_id = media_studio
        .trim_video(user_id, req.capture_id, TrimParams {
            start_timestamp: req.start,
            duration_secs: req.duration,
        })
        .await
        .map_err(|e| {
            eprintln!("[media_studio] Trim error: {}", e);
            match e {
                MediaStudioError::NotFound => StatusCode::NOT_FOUND,
                MediaStudioError::InvalidParams(_) => StatusCode::BAD_REQUEST,
                MediaStudioError::InvalidMediaType(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            }
        })?;

    Ok(Json(EditResult { new_capture_id }))
}
