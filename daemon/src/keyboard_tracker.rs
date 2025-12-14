use std::fmt;

use block::{ConcreteBlock, RcBlock};
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use objc_id::ShareId;

const NSEVENT_TYPE_KEY_DOWN: u64 = 10;

fn key_down_mask() -> u64 {
    1 << NSEVENT_TYPE_KEY_DOWN
}

#[derive(Debug)]
pub enum KeyboardTrackerError {
    MonitorUnavailable,
}

impl fmt::Display for KeyboardTrackerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyboardTrackerError::MonitorUnavailable => {
                write!(f, "Failed to install global keyboard monitor")
            }
        }
    }
}

impl std::error::Error for KeyboardTrackerError {}

pub struct KeyboardTracker {
    monitor: ShareId<Object>,
    _handler: RcBlock<(*mut Object,), ()>,
}

impl KeyboardTracker {
    pub fn start<F>(handler: F) -> Result<Self, KeyboardTrackerError>
    where
        F: Fn() + Send + 'static,
    {
        // Just fire the handler - don't capture the actual keystroke
        let block = ConcreteBlock::new(move |_event: *mut Object| {
            handler();
        })
        .copy();

        let mask = key_down_mask();
        let monitor: *mut Object = unsafe {
            msg_send![
                class!(NSEvent),
                addGlobalMonitorForEventsMatchingMask: mask
                handler: &*block
            ]
        };

        if monitor.is_null() {
            return Err(KeyboardTrackerError::MonitorUnavailable);
        }

        Ok(Self {
            monitor: unsafe { ShareId::from_ptr(monitor) },
            _handler: block,
        })
    }
}

impl Drop for KeyboardTracker {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![class!(NSEvent), removeMonitor:&*self.monitor];
        }
    }
}
