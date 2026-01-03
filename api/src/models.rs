//! Shared data models used across modules

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Video clip metadata for tweet media
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct VideoClip {
    pub source_capture_id: i64,
    pub start_timestamp: String,
    pub duration_secs: f64,
}

#[allow(dead_code)]
impl VideoClip {
    /// Parse from serde_json::Value (for DB reads)
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }

    /// Convert to serde_json::Value (for DB writes)
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// A capture record from the database
#[allow(dead_code)]
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CaptureRecord {
    pub id: i64,
    pub media_type: String,
    pub content_type: String,
    pub gcs_path: String,
    pub captured_at: DateTime<Utc>,
}

/// A capture record for thumbnail processing
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CaptureForThumbnail {
    pub id: i64,
    pub media_type: String,
    pub gcs_path: String,
    pub captured_at: DateTime<Utc>,
}
