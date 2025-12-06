use std::fmt;

use block::{ConcreteBlock, RcBlock};
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use objc_id::ShareId;

const NSEVENT_TYPE_LEFT_MOUSE_DOWN: u64 = 1;
const NSEVENT_TYPE_RIGHT_MOUSE_DOWN: u64 = 3;
const NSEVENT_TYPE_OTHER_MOUSE_DOWN: u64 = 25;

fn mouse_down_mask() -> u64 {
    (1 << NSEVENT_TYPE_LEFT_MOUSE_DOWN)
        | (1 << NSEVENT_TYPE_RIGHT_MOUSE_DOWN)
        | (1 << NSEVENT_TYPE_OTHER_MOUSE_DOWN)
}

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
    monitor: ShareId<Object>,
    _handler: RcBlock<(*mut Object,), ()>,
}

impl MouseTracker {
    pub fn start<F>(handler: F) -> Result<Self, MouseTrackerError>
    where
        F: Fn() + Send + 'static,
    {
        let block = ConcreteBlock::new(move |_event: *mut Object| {
            handler();
        })
        .copy();

        let mask = mouse_down_mask();
        let monitor: *mut Object = unsafe {
            msg_send![
                class!(NSEvent),
                addGlobalMonitorForEventsMatchingMask: mask
                handler: &*block
            ]
        };

        if monitor.is_null() {
            return Err(MouseTrackerError::MonitorUnavailable);
        }

        Ok(Self {
            monitor: unsafe { ShareId::from_ptr(monitor) },
            _handler: block,
        })
    }
}

impl Drop for MouseTracker {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![class!(NSEvent), removeMonitor:&*self.monitor];
        }
    }
}
