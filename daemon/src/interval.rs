use std::sync::OnceLock;
use std::time::Instant;

const INTERVAL_SECONDS: u64 = 5 * 60;

static START_TIME: OnceLock<Instant> = OnceLock::new();

/// Returns the current 5-minute interval identifier since the app booted.
pub fn current_interval_id() -> u64 {
    let start = START_TIME.get_or_init(Instant::now);
    let elapsed = start.elapsed().as_secs();
    elapsed / INTERVAL_SECONDS
}
