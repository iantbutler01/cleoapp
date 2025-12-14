use anyhow::Result;
use std::path::Path;

/// Raw frame extracted from video or screenshot
pub struct Frame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Pluggable content filter service for NSFW detection
pub trait ContentFilter: Send + Sync {
    /// Extract raw frames from video at given interval
    fn sample(&self, path: &Path, interval_secs: u32) -> Result<Vec<Frame>>;

    /// Scale image to model input size (224x224 RGB)
    fn scale(&self, rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>>;

    /// Classify batch of scaled images, returns true for each if safe
    fn classify(&self, scaled_images: &[Vec<u8>]) -> Result<Vec<bool>>;
}

mod nsfw;
mod noop;

pub use nsfw::NsfwFilter;
pub use noop::NoOpFilter;
