use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response, multipart};
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};

use crate::interval::current_interval_id;

/// Errors that can occur while interacting with the remote capture API.
#[derive(Debug)]
pub enum ApiError {
    Http(reqwest::Error),
    UnexpectedStatus { status: StatusCode, body: String },
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Http(err) => write!(f, "http error: {err}"),
            ApiError::UnexpectedStatus { status, body } => {
                write!(f, "unexpected status {status}: {body}")
            }
        }
    }
}

impl std::error::Error for ApiError {}

/// Result from batch upload endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchUploadResult {
    pub uploaded: usize,
    pub failed: usize,
    #[serde(default)]
    pub successful_indices: Vec<usize>,
}

/// Recording limits fetched from the API.
#[derive(Debug, Clone, Deserialize)]
pub struct RecordingLimits {
    /// Maximum duration of a single recording in seconds
    pub max_recording_duration_secs: u64,
    /// Recording budget per hour in seconds
    pub recording_budget_secs: u64,
    /// Inactivity duration before recording stops in seconds
    pub inactivity_timeout_secs: u64,
    /// Total storage limit in bytes
    pub storage_limit_bytes: u64,
    /// Current storage used in bytes
    pub storage_used_bytes: u64,
}

impl RecordingLimits {
    /// Returns remaining storage in bytes
    pub fn storage_remaining(&self) -> u64 {
        self.storage_limit_bytes
            .saturating_sub(self.storage_used_bytes)
    }

    /// Returns true if storage limit has been exceeded
    pub fn storage_exceeded(&self) -> bool {
        self.storage_used_bytes >= self.storage_limit_bytes
    }
}

impl From<reqwest::Error> for ApiError {
    fn from(value: reqwest::Error) -> Self {
        ApiError::Http(value)
    }
}

/// Blocking API client that knows how to hit Cleo's capture endpoints.
#[derive(Debug, Clone)]
pub struct ApiClient {
    base_url: String,
    http: Client,
    auth_token: Option<String>,
}

impl ApiClient {
    /// Create a new client targeting the provided base URL.
    pub fn new(base_url: impl Into<String>, auth_token: Option<String>) -> Result<Self, ApiError> {
        let http = Client::builder().timeout(Duration::from_secs(60)).build()?;

        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
            auth_token,
        })
    }

    /// Uploads a batch of images to the `/captures/batch` endpoint.
    pub fn upload_images(
        &self,
        captures: Vec<(Vec<u8>, ImageFormat)>,
    ) -> Result<BatchUploadResult, ApiError> {
        let parts: Vec<_> = captures
            .into_iter()
            .map(|(b, f)| (b, f.mime_type()))
            .collect();
        self.upload_batch(parts)
    }

    /// Uploads a batch of videos to the `/captures/batch` endpoint.
    pub fn upload_videos(
        &self,
        captures: Vec<(Vec<u8>, VideoFormat)>,
    ) -> Result<BatchUploadResult, ApiError> {
        let parts: Vec<_> = captures
            .into_iter()
            .map(|(b, f)| (b, f.mime_type()))
            .collect();
        self.upload_batch(parts)
    }

    fn upload_batch(
        &self,
        captures: Vec<(Vec<u8>, &'static str)>,
    ) -> Result<BatchUploadResult, ApiError> {
        if captures.is_empty() {
            return Ok(BatchUploadResult {
                uploaded: 0,
                failed: 0,
                successful_indices: vec![],
            });
        }

        let url = format!("{}/captures/batch", self.base_url);
        let interval_id = current_interval_id();

        let mut form = multipart::Form::new();
        for (i, (bytes, mime_type)) in captures.into_iter().enumerate() {
            let part = multipart::Part::bytes(bytes)
                .mime_str(mime_type)
                .map_err(|e| ApiError::Http(e.into()))?
                .file_name(format!("file_{}", i));
            form = form.part("file", part);
        }

        let request = self
            .http
            .post(url)
            .header("X-Interval-ID", interval_id.to_string())
            .multipart(form);
        let response = self.authorized(request).send()?;

        if response.status().is_success() {
            let result: BatchUploadResult = response.json().unwrap_or(BatchUploadResult {
                uploaded: 0,
                failed: 0,
                successful_indices: vec![],
            });
            Ok(result)
        } else {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            Err(ApiError::UnexpectedStatus { status, body })
        }
    }

    /// Sends a batch of activity events to the `/activity` endpoint.
    pub fn upload_activity(&self, events: &[ActivityEntry]) -> Result<(), ApiError> {
        let url = format!("{}/activity", self.base_url);
        let request = self.http.post(url).json(events);
        let response = self.authorized(request).send()?;
        Self::handle_response(response)
    }

    /// Fetches recording limits from the `/me/limits` endpoint.
    pub fn fetch_limits(&self) -> Result<RecordingLimits, ApiError> {
        let url = format!("{}/me/limits", self.base_url);
        let request = self.http.get(url);
        let response = self.authorized(request).send()?;

        if response.status().is_success() {
            response.json().map_err(ApiError::from)
        } else {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            Err(ApiError::UnexpectedStatus { status, body })
        }
    }

    /// Returns the base URL configured for this client.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn handle_response(response: Response) -> Result<(), ApiError> {
        if response.status().is_success() {
            return Ok(());
        }

        let status = response.status();
        let body = response.text().unwrap_or_default();
        Err(ApiError::UnexpectedStatus { status, body })
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        if let Some(token) = &self.auth_token {
            request.header(AUTHORIZATION, format!("Bearer {}", token))
        } else {
            request
        }
    }
}

/// Known image MIME types supported by the capture endpoint.
#[derive(Debug, Clone, Copy)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl ImageFormat {
    pub fn mime_type(&self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Gif => "image/gif",
            ImageFormat::Webp => "image/webp",
        }
    }
}

/// Known video MIME types supported by the capture endpoint.
#[derive(Debug, Clone, Copy)]
pub enum VideoFormat {
    Mp4,
    QuickTime,
    Webm,
}

impl VideoFormat {
    pub fn mime_type(&self) -> &'static str {
        match self {
            VideoFormat::Mp4 => "video/mp4",
            VideoFormat::QuickTime => "video/quicktime",
            VideoFormat::Webm => "video/webm",
        }
    }
}

/// Activity payload envelope that mirrors the `/activity` schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub timestamp: DateTime<Utc>,
    #[serde(rename = "intervalId")]
    pub interval_id: u64,
    pub event: ActivityEvent,
}

impl ActivityEntry {
    pub fn new(timestamp: DateTime<Utc>, interval_id: u64, event: ActivityEvent) -> Self {
        Self {
            timestamp,
            interval_id,
            event,
        }
    }
}

/// Specific activity types supported by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl ActivityEvent {
    pub fn foreground_switch(
        new_active: impl Into<String>,
        window_title: impl Into<String>,
    ) -> Self {
        ActivityEvent::ForegroundSwitch {
            new_active: new_active.into(),
            window_title: window_title.into(),
        }
    }

    pub fn mouse_click() -> Self {
        ActivityEvent::MouseClick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_event_serializes_as_expected() {
        let entry = ActivityEntry::new(
            Utc::now(),
            42,
            ActivityEvent::foreground_switch("Chrome", "Docs - Meeting"),
        );
        let json = serde_json::to_string(&entry).expect("serialize entry");
        assert!(json.contains("\"type\":\"ForegroundSwitch\""));
        assert!(json.contains("\"newActive\":\"Chrome\""));
        assert!(json.contains("\"windowTitle\":\"Docs - Meeting\""));
        assert!(json.contains("\"intervalId\":42"));
    }
}
