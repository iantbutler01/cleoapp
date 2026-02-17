mod agent;
mod constants;
mod domain;
mod frames;
mod models;
mod routes;
mod services;
mod storage;
mod thumbnails;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderName, HeaderValue, Method, header},
    routing::get,
};
use chrono::{DateTime, Utc};
use google_cloud_storage::client::Storage;
use reson_agentic::providers::GoogleGenAIClient;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::{cors::CorsLayer, set_header::SetResponseHeaderLayer};

use constants::{BUCKET_NAME, MAX_CAPTURE_UPLOAD_SIZE};
use services::twitter::TwitterClient;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub gcs: Option<Storage>,
    pub twitter: TwitterClient,
    /// Optional local storage path - if set, captures are written to disk instead of GCS
    pub local_storage_path: Option<PathBuf>,
    /// Secret key for signing JWT access tokens
    pub jwt_secret: Vec<u8>,
    /// Gemini client for AI agent (optional)
    pub gemini: Option<GoogleGenAIClient>,
    /// Optional allowlist of Twitter usernames that can log in (lowercase)
    /// If None, anyone can log in. If Some, only listed usernames are allowed.
    pub allowed_users: Option<std::collections::HashSet<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ActivityEvent {
    #[serde(rename = "ForegroundSwitch")]
    ForegroundSwitch {
        #[serde(rename = "newActive")]
        new_active: String,
        #[serde(rename = "windowTitle")]
        window_title: String,
    },
    #[serde(rename = "MouseClick")]
    MouseClick,
}

#[derive(Debug, Deserialize)]
pub struct Activity {
    timestamp: DateTime<Utc>,
    #[serde(rename = "intervalId")]
    interval_id: i64,
    event: ActivityEvent,
}

#[derive(Serialize)]
pub struct BatchCaptureResponse {
    ids: Vec<i64>,
    uploaded: usize,
    failed: usize,
    successful_indices: Vec<usize>,
}

pub fn get_extension(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        _ => "bin",
    }
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cleo:cleo@localhost/cleo".to_string());

    // Pool size: 25 connections is a good default for most workloads
    // - Enough for concurrent requests + background workers
    // - Well under typical PostgreSQL max_connections (100)
    // - Can be tuned via DB_POOL_SIZE env var if needed
    let pool_size: u32 = std::env::var("DB_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(25);

    let pool = PgPoolOptions::new()
        .max_connections(pool_size)
        .connect(&database_url)
        .await
        .expect("Failed to connect to database");

    println!("[startup] Database pool: {} max connections", pool_size);

    // GCS client (optional - requires GOOGLE_APPLICATION_CREDENTIALS)
    let gcs = match Storage::builder().build().await {
        Ok(client) => {
            println!("[startup] GCS client initialized");
            Some(client)
        }
        Err(e) => {
            println!(
                "[startup] GCS client not available: {} (local storage only)",
                e
            );
            None
        }
    };

    // Gemini client for File API operations (optional - if not set, background agent is disabled)
    let gemini = match std::env::var("GOOGLE_GEMINI_API_KEY") {
        Ok(key) => {
            println!("[startup] Gemini API key found, AI agent enabled");
            Some(GoogleGenAIClient::new(&key, "gemini-2.5-flash"))
        }
        Err(_) => {
            println!("[startup] GOOGLE_GEMINI_API_KEY not set, AI agent disabled");
            None
        }
    };

    // Public web origin used for callback URLs and CORS assumptions
    let app_origin = std::env::var("APP_ORIGIN")
        .unwrap_or_else(|_| "http://localhost:5173".to_string())
        .trim_end_matches('/')
        .to_string();

    // Twitter OAuth 2.0 client
    let twitter_client_id =
        std::env::var("TWITTER_CLIENT_ID").expect("TWITTER_CLIENT_ID must be set");
    let twitter_client_secret =
        std::env::var("TWITTER_CLIENT_SECRET").expect("TWITTER_CLIENT_SECRET must be set");
    let twitter_redirect_uri = std::env::var("TWITTER_REDIRECT_URI").unwrap_or_else(|_| {
        let callback_path = std::env::var("TWITTER_CALLBACK_PATH")
            .unwrap_or_else(|_| "/auth/twitter/callback".to_string());
        let callback_path = if callback_path.is_empty() {
            "/auth/twitter/callback".to_string()
        } else if callback_path.starts_with('/') {
            callback_path
        } else {
            format!("/{}", callback_path)
        };
        format!("{}{}", app_origin, callback_path)
    });
    let twitter = TwitterClient::new(
        &twitter_client_id,
        &twitter_client_secret,
        &twitter_redirect_uri,
    );

    // Optional local storage path - if set, captures are saved locally instead of GCS
    let local_storage_path = std::env::var("LOCAL_STORAGE_PATH").ok().map(PathBuf::from);
    if let Some(ref path) = local_storage_path {
        println!("[startup] LOCAL_STORAGE_PATH set: {:?}", path);
        println!("[startup] Captures will be saved locally instead of GCS");
    }

    // JWT secret for session tokens - REQUIRED for web auth
    // Sessions won't persist across restarts without a stable secret
    let jwt_secret = std::env::var("JWT_SECRET")
        .expect("JWT_SECRET environment variable must be set for session authentication")
        .into_bytes();

    if jwt_secret.len() < 32 {
        panic!("JWT_SECRET must be at least 32 bytes for security");
    }

    // Optional allowlist of Twitter usernames that can log in
    // Comma-separated, case-insensitive. If not set, anyone can log in.
    let allowed_users: Option<std::collections::HashSet<String>> =
        std::env::var("ALLOWED_USERS").ok().map(|s| {
            let users: std::collections::HashSet<String> = s
                .split(',')
                .map(|u| u.trim().to_lowercase())
                .filter(|u| !u.is_empty())
                .collect();
            println!("[startup] ALLOWED_USERS set: {:?}", users);
            users
        });
    if allowed_users.is_none() {
        println!("[startup] ALLOWED_USERS not set - anyone can log in");
    }

    let state = Arc::new(AppState {
        db: pool.clone(),
        gcs: gcs.clone(),
        twitter,
        local_storage_path: local_storage_path.clone(),
        jwt_secret,
        gemini: gemini.clone(),
        allowed_users,
    });

    // Background agent scheduler configuration (override via env if needed)
    let agent_idle_minutes: i64 = std::env::var("AGENT_IDLE_MINUTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&v| v >= 1)
        .unwrap_or(20);
    let agent_check_interval_secs: u64 = std::env::var("AGENT_CHECK_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&v| v >= 30)
        .unwrap_or(5 * 60);

    // Start background scheduler for idle user processing
    // Runs when either Gemini API key or LOCAL_LLM is configured
    let local_llm_configured = std::env::var("LOCAL_LLM").is_ok();
    if state.gemini.is_some() || local_llm_configured {
        let backend = if local_llm_configured {
            "local LLM"
        } else {
            "Gemini"
        };
        tokio::spawn(agent::start_background_scheduler(
            pool.clone(),
            gcs.clone(),
            state.gemini.clone(),
            agent_idle_minutes,
            agent_check_interval_secs,
            local_storage_path.clone(),
        ));
        println!(
            "[scheduler] Background scheduler started ({} backend, {}min idle, {}s check)",
            backend, agent_idle_minutes, agent_check_interval_secs
        );
    } else {
        println!("[scheduler] Background scheduler DISABLED (no Gemini API key or LOCAL_LLM)");
    }

    // Start thumbnail background worker
    tokio::spawn(thumbnails::run_thumbnail_worker(
        pool.clone(),
        gcs.clone(),
        local_storage_path.clone(),
        BUCKET_NAME.to_string(),
    ));

    // Start frame extraction background worker
    tokio::spawn(frames::run_frame_worker(
        pool.clone(),
        gcs.clone(),
        local_storage_path.clone(),
        BUCKET_NAME.to_string(),
    ));

    // CORS configuration - allow web frontend origin
    let cors_origin = std::env::var("CORS_ORIGIN").unwrap_or_else(|_| app_origin.clone());
    let cors = CorsLayer::new()
        .allow_origin(
            cors_origin
                .parse::<HeaderValue>()
                .unwrap_or_else(|_| HeaderValue::from_static("http://localhost:5173")),
        )
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, header::ACCEPT])
        .allow_credentials(true);

    // Security headers
    let x_frame_options = SetResponseHeaderLayer::overriding(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    let x_content_type_options = SetResponseHeaderLayer::overriding(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    let x_xss_protection = SetResponseHeaderLayer::overriding(
        HeaderName::from_static("x-xss-protection"),
        HeaderValue::from_static("1; mode=block"),
    );

    let app = Router::new()
        .route("/health", get(health))
        .merge(routes::build_routes())
        .layer(DefaultBodyLimit::max(MAX_CAPTURE_UPLOAD_SIZE))
        .layer(cors)
        .layer(x_frame_options)
        .layer(x_content_type_options)
        .layer(x_xss_protection)
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind to {}: {}", addr, e));

    println!("Listening on http://{}", addr);
    Ok(axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .expect("Server failed"))
}
