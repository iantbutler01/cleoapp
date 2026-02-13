#![allow(dead_code)]

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::OnceLock;

use core_foundation::base::{CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::dictionary::CFDictionaryRef;
use core_foundation::runloop::{
    CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRef, CFRunLoopRemoveSource,
    CFRunLoopSourceRef, kCFRunLoopDefaultMode,
};
use core_foundation::string::{CFString, CFStringRef};
use libc::pid_t;
use log::{info, warn};
use objc2::declare::ClassBuilder;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{ClassType, msg_send, sel};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSObject, NSString};

#[derive(Debug, Clone)]
pub struct ActiveWindowInfo {
    pub app_name: String,
    pub window_title: String,
}

pub struct AccessibilityTracker {
    callback_state: *mut CallbackState,
    workspace_observer: Retained<AnyObject>,
}

#[derive(Debug)]
pub enum AccessibilityError {
    NotTrusted,
    NotificationError(&'static str, AXError),
}

impl std::fmt::Display for AccessibilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccessibilityError::NotTrusted => write!(f, "Accessibility permissions not granted"),
            AccessibilityError::NotificationError(name, code) => {
                write!(f, "Failed to register {name} notification ({code})")
            }
        }
    }
}

impl std::error::Error for AccessibilityError {}

type Callback = Box<dyn Fn(ActiveWindowInfo) + 'static>;

struct CallbackState {
    handler: Callback,
    system_element: AXUIElementRef,
    run_loop: CFRunLoopRef,
    app_observer: RefCell<Option<AppObserver>>,
}

struct AppObserver {
    observer: AXObserverRef,
    run_loop_source: CFRunLoopSourceRef,
    app_element: AXUIElementRef,
}

impl CallbackState {
    unsafe fn sync_to_current_application(&self, context: *mut c_void) {
        match focused_app_pid() {
            Some(pid) => {
                if let Err(code) = unsafe { self.install_window_notification(context, pid) } {
                    warn!("Failed to observe focused app pid {pid} ({code})");
                }
            }
            None => unsafe { self.clear_app_notifications() },
        }
    }

    unsafe fn install_window_notification(
        &self,
        context: *mut c_void,
        pid: pid_t,
    ) -> Result<(), AXError> {
        unsafe { self.clear_app_notifications() };
        let app = unsafe { AXUIElementCreateApplication(pid) };
        if app.is_null() {
            return Err(KAX_ERROR_INVALID_UI_ELEMENT);
        }

        let mut observer: AXObserverRef = ptr::null_mut();
        let status = unsafe { AXObserverCreate(pid, observer_callback, &mut observer) };
        if status != KAX_ERROR_SUCCESS {
            unsafe { CFRelease(app as CFTypeRef) };
            return Err(status);
        }

        let result = unsafe {
            AXObserverAddNotification(
                observer,
                app,
                ax_focused_window_changed_notification(),
                context,
            )
        };
        if result == KAX_ERROR_SUCCESS {
            let run_loop_source = unsafe { AXObserverGetRunLoopSource(observer) };
            unsafe {
                CFRunLoopAddSource(self.run_loop, run_loop_source, kCFRunLoopDefaultMode);
            }
            self.app_observer.replace(Some(AppObserver {
                observer,
                run_loop_source,
                app_element: app,
            }));
            Ok(())
        } else {
            unsafe {
                CFRelease(app as CFTypeRef);
                CFRelease(observer as CFTypeRef);
            }
            Err(result)
        }
    }

    unsafe fn clear_app_notifications(&self) {
        if let Some(previous) = self.app_observer.borrow_mut().take() {
            unsafe {
                AXObserverRemoveNotification(
                    previous.observer,
                    previous.app_element,
                    ax_focused_window_changed_notification(),
                );
                CFRunLoopRemoveSource(
                    self.run_loop,
                    previous.run_loop_source,
                    kCFRunLoopDefaultMode,
                );
                CFRelease(previous.observer as CFTypeRef);
                CFRelease(previous.app_element as CFTypeRef);
            }
        }
    }
}

impl AccessibilityTracker {
    pub fn start<F>(handler: F) -> Result<Self, AccessibilityError>
    where
        F: Fn(ActiveWindowInfo) + 'static,
    {
        if !unsafe { AXIsProcessTrustedWithOptions(ptr::null()) } {
            return Err(AccessibilityError::NotTrusted);
        }

        let run_loop = unsafe { CFRunLoopGetCurrent() };
        unsafe { CFRetain(run_loop as CFTypeRef) };

        let system_element = unsafe { AXUIElementCreateSystemWide() };
        let callback_state = Box::into_raw(Box::new(CallbackState {
            handler: Box::new(handler),
            system_element,
            run_loop,
            app_observer: RefCell::new(None),
        }));

        unsafe {
            (*callback_state).sync_to_current_application(callback_state.cast());
        }

        let workspace_observer =
            unsafe { register_workspace_observer(callback_state) }.map_err(|err| {
                unsafe {
                    (*callback_state).clear_app_notifications();
                    CFRelease(system_element as CFTypeRef);
                    CFRelease(run_loop as CFTypeRef);
                    drop(Box::from_raw(callback_state));
                }
                err
            })?;

        Ok(Self {
            callback_state,
            workspace_observer,
        })
    }
}

impl Drop for AccessibilityTracker {
    fn drop(&mut self) {
        unsafe {
            (*self.callback_state).clear_app_notifications();
            let run_loop = (*self.callback_state).run_loop;
            let workspace = NSWorkspace::sharedWorkspace();
            let center = workspace.notificationCenter();
            let _: () = msg_send![&center, removeObserver: &*self.workspace_observer];
            CFRelease((*self.callback_state).system_element as CFTypeRef);
            CFRelease(run_loop as CFTypeRef);
            drop(Box::from_raw(self.callback_state));
        }
    }
}

impl ActiveWindowInfo {
    fn current(system_element: AXUIElementRef) -> Option<Self> {
        let app_name = frontmost_app_name().unwrap_or_else(|| "Unknown App".to_string());
        let window_title =
            focused_window_title(system_element).unwrap_or_else(|| "Unknown Window".to_string());
        Some(Self {
            app_name,
            window_title,
        })
    }
}

fn frontmost_app_name() -> Option<String> {
    unsafe {
        let workspace = NSWorkspace::sharedWorkspace();
        let app = workspace.frontmostApplication()?;
        let name = app.localizedName()?;
        Some(name.to_string())
    }
}

fn focused_app_pid() -> Option<pid_t> {
    unsafe {
        let workspace = NSWorkspace::sharedWorkspace();
        let app = workspace.frontmostApplication()?;
        let pid = app.processIdentifier();
        if pid == 0 { None } else { Some(pid) }
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

extern "C" fn observer_callback(
    _observer: AXObserverRef,
    _element: AXUIElementRef,
    _notification: CFStringRef,
    refcon: *mut c_void,
) {
    if refcon.is_null() {
        return;
    }
    let state_ptr = refcon as *mut CallbackState;
    let state = unsafe { &*state_ptr };

    if _notification == ax_focused_window_changed_notification() {
        info!(target: "accessibility", "Focused window changed");
    }

    if let Some(info) = ActiveWindowInfo::current(state.system_element) {
        info!(
            target: "accessibility",
            "Active window: {} - {}",
            info.app_name,
            info.window_title
        );
        (state.handler)(info);
    }
}

const KAX_ERROR_SUCCESS: AXError = 0;
const KAX_ERROR_INVALID_UI_ELEMENT: AXError = -25202;

type AXError = i32;
type AXObserverRef = *mut c_void;
type AXUIElementRef = *mut c_void;

unsafe fn register_workspace_observer(
    callback_state: *mut CallbackState,
) -> Result<Retained<AnyObject>, AccessibilityError> {
    let cls = workspace_observer_class();
    let observer: *mut AnyObject = msg_send![cls, new];
    if observer.is_null() {
        return Err(AccessibilityError::NotificationError(
            "NSWorkspaceDidActivateApplicationNotification",
            -1,
        ));
    }

    // Set the ivar
    unsafe {
        let ivar = (*cls).instance_variable(c"cleoState").unwrap();
        let ptr = ivar.load_ptr::<*mut c_void>(&*observer);
        *ptr = callback_state as *mut c_void;
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

fn ax_focused_window_changed_notification() -> CFStringRef {
    static VALUE: OnceLock<StaticCFString> = OnceLock::new();
    VALUE
        .get_or_init(|| StaticCFString::from_str("AXFocusedWindowChanged"))
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

fn workspace_observer_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = NSObject::class();
        let mut builder = ClassBuilder::new(c"CleoWorkspaceObserver", superclass)
            .expect("Failed to declare CleoWorkspaceObserver");
        builder.add_ivar::<*mut c_void>(c"cleoState");

        unsafe extern "C" fn handle_workspace_activation(
            this: *mut AnyObject,
            _: Sel,
            _: *mut AnyObject,
        ) {
            unsafe {
                let cls = (*this).class();
                let ivar = cls.instance_variable(c"cleoState").unwrap();
                let state_ptr = *ivar.load_ptr::<*mut c_void>(&*this) as *mut CallbackState;
                if !state_ptr.is_null() {
                    (*state_ptr).sync_to_current_application(state_ptr.cast());
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

/// Check if the app has accessibility permissions, optionally prompting the user
pub fn check_accessibility_trusted(prompt: bool) -> bool {
    if prompt {
        // Create a dictionary with kAXTrustedCheckOptionPrompt = true
        use core_foundation::boolean::CFBoolean;
        use core_foundation::dictionary::CFDictionary;
        use core_foundation::string::CFString;

        let key = CFString::from_static_string("AXTrustedCheckOptionPrompt");
        let value = CFBoolean::true_value();
        let dict = CFDictionary::from_CFType_pairs(&[(key, value)]);
        unsafe { AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef()) }
    } else {
        unsafe { AXIsProcessTrustedWithOptions(ptr::null()) }
    }
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    fn AXObserverCreate(
        application: pid_t,
        callback: extern "C" fn(AXObserverRef, AXUIElementRef, CFStringRef, *mut c_void),
        out: *mut AXObserverRef,
    ) -> AXError;
    fn AXObserverAddNotification(
        observer: AXObserverRef,
        element: AXUIElementRef,
        notification: CFStringRef,
        context: *mut c_void,
    ) -> AXError;
    fn AXObserverRemoveNotification(
        observer: AXObserverRef,
        element: AXUIElementRef,
        notification: CFStringRef,
    );
    fn AXObserverGetRunLoopSource(observer: AXObserverRef) -> CFRunLoopSourceRef;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCreateApplication(pid: pid_t) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
}
