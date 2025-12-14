use std::fmt;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, ClassType};
use objc2_app_kit::{NSEvent, NSEventMask};

#[derive(Debug)]
pub enum MouseTrackerError {
    MonitorUnavailable,
}

impl fmt::Display for MouseTrackerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MouseTrackerError::MonitorUnavailable => {
                write!(f, "Failed to install global mouse monitor")
            }
        }
    }
}

impl std::error::Error for MouseTrackerError {}

pub struct MouseTracker {
    monitor: Retained<AnyObject>,
    _handler: RcBlock<dyn Fn(*mut AnyObject)>,
}

impl MouseTracker {
    pub fn start<F>(handler: F) -> Result<Self, MouseTrackerError>
    where
        F: Fn() + Send + 'static,
    {
        let block = RcBlock::new(move |_event: *mut AnyObject| {
            handler();
        });

        let mask = NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown;
        let monitor: *mut AnyObject = unsafe {
            msg_send![
                NSEvent::class(),
                addGlobalMonitorForEventsMatchingMask: mask,
                handler: &*block
            ]
        };

        if monitor.is_null() {
            return Err(MouseTrackerError::MonitorUnavailable);
        }

        Ok(Self {
            monitor: unsafe { Retained::retain(monitor).unwrap() },
            _handler: block,
        })
    }
}

impl Drop for MouseTracker {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![NSEvent::class(), removeMonitor: &*self.monitor];
        }
    }
}
