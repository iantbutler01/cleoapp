//! Application constants

/// GCS bucket name for multimedia storage
pub const BUCKET_NAME: &str = "cleo_multimedia_data";

/// Maximum upload size for captures (200 MB)
pub const MAX_CAPTURE_UPLOAD_SIZE: usize = 200 * 1024 * 1024;

/// Signed URL expiry time in seconds (15 minutes)
pub const SIGNED_URL_EXPIRY_SECS: u32 = 15 * 60;

/// Default page size for paginated list endpoints
pub const DEFAULT_PAGE_SIZE: i64 = 50;

/// Maximum page size for paginated list endpoints
pub const MAX_PAGE_SIZE: i64 = 100;
