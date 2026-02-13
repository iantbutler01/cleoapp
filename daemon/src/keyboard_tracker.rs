use std::fmt;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{ClassType, msg_send};
use objc2_app_kit::{NSEvent, NSEventMask};

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
    monitor: Retained<AnyObject>,
    _handler: RcBlock<dyn Fn(*mut AnyObject)>,
}

impl KeyboardTracker {
    pub fn start<F>(handler: F) -> Result<Self, KeyboardTrackerError>
    where
        F: Fn() + Send + 'static,
    {
        // Just fire the handler - don't capture the actual keystroke
        let block = RcBlock::new(move |_event: *mut AnyObject| {
            handler();
        });

        let monitor: *mut AnyObject = unsafe {
            msg_send![
                NSEvent::class(),
                addGlobalMonitorForEventsMatchingMask: NSEventMask::KeyDown,
                handler: &*block
            ]
        };

        if monitor.is_null() {
            return Err(KeyboardTrackerError::MonitorUnavailable);
        }

        Ok(Self {
            monitor: unsafe { Retained::retain(monitor).unwrap() },
            _handler: block,
        })
    }
}

impl Drop for KeyboardTracker {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![NSEvent::class(), removeMonitor: &*self.monitor];
        }
    }
}
