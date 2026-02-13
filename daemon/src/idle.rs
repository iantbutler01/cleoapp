// CGEventSourceStateID
const K_CGEVENT_SOURCE_STATE_COMBINED_SESSION_STATE: i32 = 0;

// CGEventType for idle time - kCGAnyInputEventType
const K_CGEVENT_SOURCE_STATE_HIDDEN_STATE: u32 = 0xFFFFFFFF;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state_id: i32, event_type: u32) -> f64;
}

/// Returns seconds since last user input (mouse move, click, or keyboard)
pub fn seconds_since_last_input() -> f64 {
    unsafe {
        CGEventSourceSecondsSinceLastEventType(
            K_CGEVENT_SOURCE_STATE_COMBINED_SESSION_STATE,
            K_CGEVENT_SOURCE_STATE_HIDDEN_STATE,
        )
    }
}

/// Returns true if user has been idle for at least the given duration
pub fn is_idle(idle_threshold_secs: f64) -> bool {
    seconds_since_last_input() >= idle_threshold_secs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idle_time() {
        let idle = seconds_since_last_input();
        // Should be some reasonable value
        assert!(idle >= 0.0);
        assert!(idle < 10000.0); // Less than ~3 hours
    }
}
