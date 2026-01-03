//! Banned apps management window - System Settings style UI for managing blocked apps.

use std::cell::RefCell;
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
use objc2::sel;
use objc2::{ClassType, MainThreadOnly, msg_send};
use objc2::declare::ClassBuilder;
use objc2_app_kit::{
    NSColor, NSFont, NSImage, NSImageView, NSScrollView, NSSwitch,
    NSTextField, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSObject, NSPoint, NSRect, NSSize, NSString};

// NSControlStateValue constants
const NS_CONTROL_STATE_VALUE_ON: isize = 1;
const NS_CONTROL_STATE_VALUE_OFF: isize = 0;

const FONT_WEIGHT_MEDIUM: f64 = 0.23;
const ROW_HEIGHT: f64 = 44.0;
const WINDOW_WIDTH: f64 = 400.0;
const CONTENT_PADDING: f64 = 16.0;
const ICON_SIZE: f64 = 24.0;

/// Callback type for when a switch is toggled
pub type ToggleCallback = Box<dyn Fn(usize, bool) + Send + Sync + 'static>;

/// Storage for switch toggle callbacks
static SWITCH_CALLBACKS: OnceLock<std::sync::Mutex<Vec<ToggleCallback>>> = OnceLock::new();

fn switch_callbacks() -> &'static std::sync::Mutex<Vec<ToggleCallback>> {
    SWITCH_CALLBACKS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Create the SwitchActionTarget class for handling toggle events
fn switch_action_target_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = NSObject::class();
        let mut builder = ClassBuilder::new(c"CleoSwitchActionTarget", superclass)
            .expect("Failed to create SwitchActionTarget class");

        // Add instance variable for callback index
        builder.add_ivar::<usize>(c"callbackIndex");

        // Add toggle action method
        unsafe extern "C" fn on_toggle(this: *mut AnyObject, _sel: Sel, sender: *mut AnyObject) {
            unsafe {
                let cls = (*this).class();
                let ivar = cls.instance_variable(c"callbackIndex").unwrap();
                let idx = *ivar.load::<usize>(&*this);

                // Get the switch state
                let state: isize = msg_send![sender, state];
                let is_on = state == NS_CONTROL_STATE_VALUE_ON;

                if let Ok(callbacks) = switch_callbacks().lock() {
                    if let Some(callback) = callbacks.get(idx) {
                        callback(idx, is_on);
                    }
                }
            }
        }

        unsafe {
            builder.add_method(
                sel!(onToggle:),
                on_toggle as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
        }

        builder.register()
    })
}

/// Create window delegate class that hides instead of closes
fn window_delegate_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = NSObject::class();
        let mut builder = ClassBuilder::new(c"CleoBannedAppsWindowDelegate", superclass)
            .expect("Failed to create WindowDelegate class");

        // windowShouldClose: - return NO and hide the window instead
        unsafe extern "C" fn window_should_close(_this: *mut AnyObject, _sel: Sel, window: *mut AnyObject) -> Bool {
            // Hide the window instead of closing it
            let _: () = msg_send![window, orderOut: std::ptr::null::<AnyObject>()];
            Bool::NO
        }

        unsafe {
            builder.add_method(
                sel!(windowShouldClose:),
                window_should_close as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool,
            );
        }

        builder.register()
    })
}

/// Create a flipped NSView subclass (coordinates start from top-left)
fn flipped_view_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = NSView::class();
        let mut builder = ClassBuilder::new(c"CleoFlippedView", superclass)
            .expect("Failed to create FlippedView class");

        // Override isFlipped to return YES
        unsafe extern "C" fn is_flipped(_this: *mut AnyObject, _sel: Sel) -> Bool {
            Bool::YES
        }

        unsafe {
            builder.add_method(
                sel!(isFlipped),
                is_flipped as unsafe extern "C" fn(*mut AnyObject, Sel) -> Bool,
            );
        }

        builder.register()
    })
}

/// Get app icon by app name
fn get_app_icon(app_name: &str, mtm: MainThreadMarker) -> Option<Retained<NSImage>> {
    unsafe {
        let workspace = NSWorkspace::sharedWorkspace();

        // Try to find the running app first
        let apps = workspace.runningApplications();
        for app in apps.iter() {
            if let Some(name) = app.localizedName() {
                if name.to_string() == app_name {
                    return app.icon();
                }
            }
        }

        // Fallback: try to find app in /Applications
        let app_path = format!("/Applications/{}.app", app_name);
        let path_str = NSString::from_str(&app_path);
        let icon = workspace.iconForFile(&path_str);

        // If that didn't work, use a generic app icon
        let generic_icon_name = NSString::from_str("NSApplicationIcon");
        if let Some(generic) = NSImage::imageNamed(&generic_icon_name) {
            return Some(generic);
        }

        Some(icon)
    }
}

/// Error type for window creation
#[derive(Debug)]
pub enum BannedAppsWindowError {
    CreationFailed,
}

/// The banned apps management window
pub struct BannedAppsWindow {
    window: Retained<NSWindow>,
    scroll_view: Retained<NSScrollView>,
    /// Keep targets alive
    _targets: RefCell<Vec<Retained<AnyObject>>>,
    /// Keep delegate alive
    _delegate: Retained<AnyObject>,
}

impl BannedAppsWindow {
    /// Create a new banned apps window (hidden by default)
    pub fn new(mtm: MainThreadMarker) -> Result<Self, BannedAppsWindowError> {
        // Initial window size (will adjust based on content)
        let window_height: f64 = 300.0;

        let frame = NSRect::new(
            NSPoint::new(100.0, 100.0),
            NSSize::new(WINDOW_WIDTH, window_height),
        );

        let style_mask = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Resizable;

        // Create window delegate to hide instead of close
        let delegate: Retained<AnyObject> = unsafe {
            let cls = window_delegate_class();
            let delegate: *mut AnyObject = msg_send![cls, new];
            Retained::retain(delegate).unwrap()
        };

        let window = unsafe {
            let window = NSWindow::alloc(mtm);
            let window: Retained<NSWindow> = msg_send![
                window,
                initWithContentRect: frame,
                styleMask: style_mask,
                backing: 2u64,  // NSBackingStoreBuffered
                defer: false
            ];

            let title = NSString::from_str("Banned Apps");
            window.setTitle(&title);

            // Set delegate to intercept close and hide instead
            let _: () = msg_send![&window, setDelegate: &*delegate];

            // Center on screen
            window.center();

            window
        };

        // Get content view and set up visual effect background
        let content_view = unsafe {
            let content_view = window.contentView().ok_or(BannedAppsWindowError::CreationFailed)?;
            let bounds = content_view.bounds();

            content_view.setWantsLayer(true);

            let effect_view = NSVisualEffectView::new(mtm);
            effect_view.setFrame(bounds);
            effect_view.setMaterial(NSVisualEffectMaterial::Sidebar);
            effect_view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            effect_view.setState(NSVisualEffectState::Active);
            // Autoresizing: width + height sizable
            effect_view.setAutoresizingMask(std::mem::transmute(18u64));

            content_view.addSubview_positioned_relativeTo(
                &effect_view,
                objc2_app_kit::NSWindowOrderingMode::Below,
                None,
            );

            content_view
        };

        // Create scroll view for the list
        let scroll_view = unsafe {
            let bounds = content_view.bounds();
            let scroll = NSScrollView::new(mtm);
            scroll.setFrame(bounds);
            scroll.setHasVerticalScroller(true);
            scroll.setAutoresizingMask(std::mem::transmute(18u64)); // width + height sizable

            // Create a flipped document view so content starts from the top
            let flipped_cls = flipped_view_class();
            let doc_view: *mut AnyObject = msg_send![flipped_cls, new];
            let doc_view: Retained<NSView> = Retained::retain(doc_view as *mut NSView).unwrap();
            doc_view.setFrame(bounds);
            scroll.setDocumentView(Some(&doc_view));

            content_view.addSubview(&scroll);
            scroll
        };

        Ok(Self {
            window,
            scroll_view,
            _targets: RefCell::new(Vec::new()),
            _delegate: delegate,
        })
    }

    /// Update the window with the current list of apps
    /// `apps` is a list of (app_name, is_banned) tuples
    pub fn update_apps(&self, apps: &[(String, bool)], on_toggle: impl Fn(String, bool) + Send + Sync + 'static + Clone) {
        let mtm = match MainThreadMarker::new() {
            Some(m) => m,
            None => return,
        };

        unsafe {
            // Get or create document view
            let doc_view = match self.scroll_view.documentView() {
                Some(v) => v,
                None => return,
            };

            // Remove all subviews
            for subview in doc_view.subviews().iter() {
                subview.removeFromSuperview();
            }

            // Clear previous targets and callbacks
            self._targets.borrow_mut().clear();
            if let Ok(mut callbacks) = switch_callbacks().lock() {
                callbacks.clear();
            }

            // Calculate content height
            let content_height = (apps.len() as f64 * ROW_HEIGHT).max(100.0);
            let scroll_bounds = self.scroll_view.bounds();

            // Resize document view
            let doc_frame = NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(scroll_bounds.size.width, content_height),
            );
            doc_view.setFrame(doc_frame);

            // Create rows for each app (top to bottom since view is flipped)
            for (i, (app_name, is_banned)) in apps.iter().enumerate() {
                let row_y = i as f64 * ROW_HEIGHT;

                // Row container
                let row_frame = NSRect::new(
                    NSPoint::new(0.0, row_y),
                    NSSize::new(scroll_bounds.size.width, ROW_HEIGHT),
                );
                let row_view = NSView::new(mtm);
                row_view.setFrame(row_frame);

                // App icon
                let icon_frame = NSRect::new(
                    NSPoint::new(CONTENT_PADDING, (ROW_HEIGHT - ICON_SIZE) / 2.0),
                    NSSize::new(ICON_SIZE, ICON_SIZE),
                );
                let icon_view = NSImageView::new(mtm);
                icon_view.setFrame(icon_frame);

                if let Some(icon) = get_app_icon(app_name, mtm) {
                    // Resize the icon to fit
                    let size = NSSize::new(ICON_SIZE, ICON_SIZE);
                    icon.setSize(size);
                    icon_view.setImage(Some(&icon));
                }

                row_view.addSubview(&icon_view);

                // App name label (positioned after icon)
                let label_x = CONTENT_PADDING + ICON_SIZE + 12.0;
                let label_frame = NSRect::new(
                    NSPoint::new(label_x, (ROW_HEIGHT - 20.0) / 2.0),
                    NSSize::new(scroll_bounds.size.width - label_x - 80.0, 20.0),
                );
                let label = NSTextField::new(mtm);
                label.setFrame(label_frame);
                let text = NSString::from_str(app_name);
                label.setStringValue(&text);
                label.setBezeled(false);
                label.setDrawsBackground(false);
                label.setEditable(false);
                label.setSelectable(false);

                let font = NSFont::systemFontOfSize_weight(14.0, FONT_WEIGHT_MEDIUM);
                label.setFont(Some(&font));
                let color = NSColor::labelColor();
                label.setTextColor(Some(&color));

                row_view.addSubview(&label);

                // Toggle switch
                let switch_frame = NSRect::new(
                    NSPoint::new(scroll_bounds.size.width - 70.0, (ROW_HEIGHT - 22.0) / 2.0),
                    NSSize::new(40.0, 22.0),
                );
                let switch = NSSwitch::new(mtm);
                switch.setFrame(switch_frame);

                // Set initial state
                if *is_banned {
                    let _: () = msg_send![&switch, setState: NS_CONTROL_STATE_VALUE_ON];
                } else {
                    let _: () = msg_send![&switch, setState: NS_CONTROL_STATE_VALUE_OFF];
                }

                // Create action target for this switch
                let app_name_clone = app_name.clone();
                let on_toggle_clone = on_toggle.clone();
                let callback: ToggleCallback = Box::new(move |_idx, is_on| {
                    on_toggle_clone(app_name_clone.clone(), is_on);
                });

                let callback_idx = {
                    let mut callbacks = switch_callbacks().lock().unwrap();
                    let idx = callbacks.len();
                    callbacks.push(callback);
                    idx
                };

                // Create target object
                let cls = switch_action_target_class();
                let target: *mut AnyObject = msg_send![cls, new];
                let target = Retained::retain(target).unwrap();

                // Set callback index
                {
                    let ivar = cls.instance_variable(c"callbackIndex").unwrap();
                    *ivar.load_mut::<usize>(&mut *Retained::as_ptr(&target).cast_mut()) = callback_idx;
                }

                // Set target and action on the switch
                let _: () = msg_send![&switch, setTarget: &*target];
                let _: () = msg_send![&switch, setAction: sel!(onToggle:)];

                // Keep target alive
                self._targets.borrow_mut().push(target);

                row_view.addSubview(&switch);
                doc_view.addSubview(&row_view);
            }

            // If no apps, show a message
            if apps.is_empty() {
                let label_frame = NSRect::new(
                    NSPoint::new(CONTENT_PADDING, 40.0),
                    NSSize::new(scroll_bounds.size.width - CONTENT_PADDING * 2.0, 40.0),
                );
                let label = NSTextField::new(mtm);
                label.setFrame(label_frame);
                let text = NSString::from_str("No apps have been banned yet.\nUse the command palette (Cmd+Shift+C) to ban apps.");
                label.setStringValue(&text);
                label.setBezeled(false);
                label.setDrawsBackground(false);
                label.setEditable(false);
                label.setSelectable(false);
                label.setAlignment(objc2_app_kit::NSTextAlignment::Center);

                let color = NSColor::secondaryLabelColor();
                label.setTextColor(Some(&color));

                doc_view.addSubview(&label);
            }
        }
    }

    /// Show the window
    pub fn show(&self) {
        unsafe {
            self.window.makeKeyAndOrderFront(None);
        }
    }

    /// Hide the window
    #[allow(dead_code)]
    pub fn hide(&self) {
        unsafe {
            self.window.orderOut(None);
        }
    }

    /// Check if window is visible
    #[allow(dead_code)]
    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }
}

// Note: No Drop impl - window is hidden instead of closed via delegate,
// and persists for the lifetime of the app.
