use std::ffi::c_void;
use std::ptr;
use std::sync::OnceLock;

use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use log::info;
use objc2::declare::ClassBuilder;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{msg_send, sel, ClassType};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSObject, NSString};

use crate::accessibility::ActiveWindowInfo;

type Callback = Box<dyn Fn(ActiveWindowInfo) + 'static>;

pub struct WorkspaceTracker {
    observer: Retained<AnyObject>,
    state: *mut WorkspaceState,
}

#[derive(Debug)]
pub enum WorkspaceTrackerError {
    ObserverUnavailable,
}

impl std::fmt::Display for WorkspaceTrackerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceTrackerError::ObserverUnavailable => {
                write!(f, "Failed to attach workspace activation observer")
            }
        }
    }
}

impl std::error::Error for WorkspaceTrackerError {}

struct WorkspaceState {
    handler: Callback,
    system_element: AXUIElementRef,
}

impl WorkspaceTracker {
    pub fn start<F>(handler: F) -> Result<Self, WorkspaceTrackerError>
    where
        F: Fn(ActiveWindowInfo) + 'static,
    {
        let system_element = unsafe { AXUIElementCreateSystemWide() };
        let state = Box::into_raw(Box::new(WorkspaceState {
            handler: Box::new(handler),
            system_element,
        }));
        let observer = unsafe { register_workspace_observer(state) }.map_err(|err| {
            unsafe {
                CFRelease(system_element as CFTypeRef);
                drop(Box::from_raw(state));
            }
            err
        })?;
        Ok(Self { observer, state })
    }
}

impl Drop for WorkspaceTracker {
    fn drop(&mut self) {
        unsafe {
            let workspace = NSWorkspace::sharedWorkspace();
            let center = workspace.notificationCenter();
            let _: () = msg_send![&center, removeObserver: &*self.observer];
            CFRelease((*self.state).system_element as CFTypeRef);
            drop(Box::from_raw(self.state));
        }
    }
}

impl WorkspaceState {
    unsafe fn handle_activation(&self) {
        if let Some(info) = unsafe { self.current_window_info() } {
            info!(
                target: "workspace",
                "Activated app: {} - {}",
                info.app_name,
                info.window_title
            );
            (self.handler)(info);
        }
    }

    unsafe fn current_window_info(&self) -> Option<ActiveWindowInfo> {
        let app_name = frontmost_app_name().unwrap_or_else(|| "Unknown App".to_string());
        let window_title = focused_window_title(self.system_element)
            .unwrap_or_else(|| "Unknown Window".to_string());
        Some(ActiveWindowInfo {
            app_name,
            window_title,
        })
    }
}

unsafe fn register_workspace_observer(
    state: *mut WorkspaceState,
) -> Result<Retained<AnyObject>, WorkspaceTrackerError> {
    let cls = workspace_observer_class();
    let observer: *mut AnyObject = msg_send![cls, new];
    if observer.is_null() {
        return Err(WorkspaceTrackerError::ObserverUnavailable);
    }

    // Set the ivar
    unsafe {
        let ivar = (*cls).instance_variable(c"cleoWorkspaceState").unwrap();
        let ptr = ivar.load_ptr::<*mut c_void>(&*observer);
        *ptr = state as *mut c_void;
    }

    let workspace = NSWorkspace::sharedWorkspace();
    let center = workspace.notificationCenter();
    let name = NSString::from_str("NSWorkspaceDidActivateApplicationNotification");
    let _: () = msg_send![
        &center,
        addObserver: observer,
        selector: sel!(handleWorkspaceActivation:),
        name: &*name,
        object: ptr::null::<AnyObject>()
    ];
    Ok(unsafe { Retained::retain(observer).unwrap() })
}

fn frontmost_app_name() -> Option<String> {
    unsafe {
        let workspace = NSWorkspace::sharedWorkspace();
        let app = workspace.frontmostApplication()?;
        let name = app.localizedName()?;
        Some(name.to_string())
    }
}

fn focused_window_title(system_element: AXUIElementRef) -> Option<String> {
    let window = copy_attribute_element(system_element, ax_focused_window_attribute())?;
    let title = copy_attribute_string(window, ax_title_attribute());
    unsafe {
        CFRelease(window as CFTypeRef);
    }
    title
}

fn copy_attribute_element(
    element: AXUIElementRef,
    attribute: CFStringRef,
) -> Option<AXUIElementRef> {
    let mut value: CFTypeRef = ptr::null();
    let status = unsafe { AXUIElementCopyAttributeValue(element, attribute, &mut value) };
    if status == KAX_ERROR_SUCCESS && !value.is_null() {
        Some(value as AXUIElementRef)
    } else {
        None
    }
}

fn copy_attribute_string(element: AXUIElementRef, attribute: CFStringRef) -> Option<String> {
    let mut value: CFTypeRef = ptr::null();
    let status = unsafe { AXUIElementCopyAttributeValue(element, attribute, &mut value) };
    if status != KAX_ERROR_SUCCESS || value.is_null() {
        return None;
    }
    let string = unsafe { CFString::wrap_under_create_rule(value as CFStringRef) };
    Some(string.to_string())
}

fn workspace_observer_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = NSObject::class();
        let mut builder =
            ClassBuilder::new(c"CleoWorkspaceActivationObserver", superclass).expect("class");
        builder.add_ivar::<*mut c_void>(c"cleoWorkspaceState");

        unsafe extern "C" fn handle_workspace_activation(
            this: *mut AnyObject,
            _: Sel,
            _: *mut AnyObject,
        ) {
            unsafe {
                let cls = (*this).class();
                let ivar = cls.instance_variable(c"cleoWorkspaceState").unwrap();
                let state_ptr = *ivar.load_ptr::<*mut c_void>(&*this) as *mut WorkspaceState;
                if !state_ptr.is_null() {
                    (*state_ptr).handle_activation();
                }
            }
        }

        unsafe {
            builder.add_method(
                sel!(handleWorkspaceActivation:),
                handle_workspace_activation
                    as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
        }
        builder.register()
    })
}

fn ax_focused_window_attribute() -> CFStringRef {
    static VALUE: OnceLock<StaticCFString> = OnceLock::new();
    VALUE
        .get_or_init(|| StaticCFString::from_str("AXFocusedWindow"))
        .0
}

fn ax_title_attribute() -> CFStringRef {
    static VALUE: OnceLock<StaticCFString> = OnceLock::new();
    VALUE.get_or_init(|| StaticCFString::from_str("AXTitle")).0
}

struct StaticCFString(CFStringRef);

impl StaticCFString {
    fn from_str(value: &'static str) -> Self {
        let cf = CFString::from_static_string(value);
        let ptr = cf.as_concrete_TypeRef();
        std::mem::forget(cf);
        Self(ptr)
    }
}

unsafe impl Send for StaticCFString {}
unsafe impl Sync for StaticCFString {}

const KAX_ERROR_SUCCESS: AXError = 0;

type AXError = i32;
type AXUIElementRef = *mut c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
}
