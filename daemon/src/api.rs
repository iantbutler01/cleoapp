use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
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
        let http = Client::builder().timeout(Duration::from_secs(10)).build()?;

        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
            auth_token,
        })
    }

    /// Uploads an image capture to the `/capture` endpoint.
    pub fn upload_image(
        &self,
        bytes: impl Into<Vec<u8>>,
        format: ImageFormat,
    ) -> Result<(), ApiError> {
        self.upload_capture(bytes.into(), format.mime_type())
    }

    /// Uploads a video capture to the `/capture` endpoint.
    pub fn upload_video(
        &self,
        bytes: impl Into<Vec<u8>>,
        format: VideoFormat,
    ) -> Result<(), ApiError> {
        self.upload_capture(bytes.into(), format.mime_type())
    }

    /// Sends a batch of activity events to the `/activity` endpoint.
    pub fn upload_activity(&self, events: &[ActivityEntry]) -> Result<(), ApiError> {
        let url = format!("{}/activity", self.base_url);
        let request = self.http.post(url).json(events);
        let response = self.authorized(request).send()?;
        Self::handle_response(response)
    }

    fn upload_capture(&self, bytes: Vec<u8>, content_type: &'static str) -> Result<(), ApiError> {
        let url = format!("{}/capture", self.base_url);
        let interval_id = current_interval_id();
        let request = self
            .http
            .post(url)
            .header(CONTENT_TYPE, content_type)
            .header("X-Interval-ID", interval_id.to_string())
            .body(bytes);
        let response = self.authorized(request).send()?;
        Self::handle_response(response)
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
