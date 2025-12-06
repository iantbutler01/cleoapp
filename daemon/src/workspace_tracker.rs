use std::ffi::{CStr, c_void};
use std::os::raw::c_char;
use std::ptr;
use std::sync::OnceLock;

use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use log::info;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use objc_id::ShareId;

use crate::accessibility::ActiveWindowInfo;

type Callback = Box<dyn Fn(ActiveWindowInfo) + 'static>;

pub struct WorkspaceTracker {
    observer: ShareId<Object>,
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
            let workspace: ObjcId = msg_send![class!(NSWorkspace), sharedWorkspace];
            let center: ObjcId = msg_send![workspace, notificationCenter];
            let _: () = msg_send![center, removeObserver:&*self.observer];
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
) -> Result<ShareId<Object>, WorkspaceTrackerError> {
    let cls = workspace_observer_class();
    let observer: ObjcId = msg_send![cls, new];
    if observer.is_null() {
        return Err(WorkspaceTrackerError::ObserverUnavailable);
    }
    unsafe {
        (*observer).set_ivar("cleoWorkspaceState", state as *mut c_void);
    }

    let workspace: ObjcId = msg_send![class!(NSWorkspace), sharedWorkspace];
    if workspace.is_null() {
        let _: () = msg_send![observer, release];
        return Err(WorkspaceTrackerError::ObserverUnavailable);
    }
    let center: ObjcId = msg_send![workspace, notificationCenter];
    let name = workspace_did_activate_notification() as ObjcId;
    let _: () = msg_send![center,
        addObserver: observer
        selector: sel!(handleWorkspaceActivation:)
        name: name
        object: ptr::null_mut::<Object>()
    ];
    Ok(unsafe { ShareId::from_ptr(observer) })
}

fn frontmost_app_name() -> Option<String> {
    unsafe {
        let workspace: ObjcId = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: ObjcId = msg_send![workspace, frontmostApplication];
        if app.is_null() {
            return None;
        }
        nsstring_to_string(app)
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

unsafe fn nsstring_to_string(obj: ObjcId) -> Option<String> {
    if obj.is_null() {
        return None;
    }
    let localized: ObjcId = msg_send![obj, localizedName];
    if localized.is_null() {
        return None;
    }
    let c_str: *const c_char = msg_send![localized, UTF8String];
    if c_str.is_null() {
        return None;
    }
    Some(
        unsafe { CStr::from_ptr(c_str) }
            .to_string_lossy()
            .into_owned(),
    )
}

fn workspace_observer_class() -> &'static Class {
    static CLASS: OnceLock<&'static Class> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = class!(NSObject);
        let mut decl =
            ClassDecl::new("CleoWorkspaceActivationObserver", superclass).expect("class");
        decl.add_ivar::<*mut c_void>("cleoWorkspaceState");
        extern "C" fn handle_workspace_activation(this: &mut Object, _: Sel, _: ObjcId) {
            unsafe {
                let state_ptr =
                    *this.get_ivar::<*mut c_void>("cleoWorkspaceState") as *mut WorkspaceState;
                if !state_ptr.is_null() {
                    (*state_ptr).handle_activation();
                }
            }
        }
        unsafe {
            decl.add_method(
                sel!(handleWorkspaceActivation:),
                handle_workspace_activation as extern "C" fn(&mut Object, Sel, ObjcId),
            );
        }
        decl.register()
    })
}

fn workspace_did_activate_notification() -> CFStringRef {
    static VALUE: OnceLock<StaticCFString> = OnceLock::new();
    VALUE
        .get_or_init(|| StaticCFString::from_str("NSWorkspaceDidActivateApplicationNotification"))
        .0
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
type ObjcId = *mut Object;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
}
