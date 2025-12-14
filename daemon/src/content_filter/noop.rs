use anyhow::Result;
use std::path::Path;

use super::{ContentFilter, Frame};

/// No-op filter that always returns safe - for testing or opt-out
pub struct NoOpFilter;

impl NoOpFilter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoOpFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentFilter for NoOpFilter {
    fn sample(&self, _path: &Path, _interval_secs: u32) -> Result<Vec<Frame>> {
        Ok(vec![])
    }

    fn scale(&self, _rgba: &[u8], _width: u32, _height: u32) -> Result<Vec<u8>> {
        Ok(vec![])
    }

    fn classify(&self, scaled_images: &[Vec<u8>]) -> Result<Vec<bool>> {
        Ok(vec![true; scaled_images.len()]) // Always safe
    }
}
