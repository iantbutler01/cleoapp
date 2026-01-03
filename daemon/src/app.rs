//! Pure objc2-based macOS application infrastructure.
//! Replaces the cacao crate for app lifecycle, menus, and status bar.

use std::cell::RefCell;
use std::sync::OnceLock;

use objc2::declare::ClassBuilder;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{class, msg_send, sel, ClassType, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSImage, NSMenu, NSMenuItem, NSStatusBar,
    NSStatusItem,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSBundle, NSObject, NSString, NSURL};
use url::Url;

/// Callback type for application lifecycle events
type DidFinishLaunchingCallback = Box<dyn FnMut() + 'static>;
type ShouldTerminateCallback = Box<dyn Fn() -> bool + 'static>;
type WillTerminateCallback = Box<dyn FnMut() + 'static>;
type OpenUrlsCallback = Box<dyn FnMut(Vec<Url>) + 'static>;

/// Thread-local storage for callbacks
thread_local! {
    static DID_FINISH_LAUNCHING: RefCell<Option<DidFinishLaunchingCallback>> = const { RefCell::new(None) };
    static SHOULD_TERMINATE: RefCell<Option<ShouldTerminateCallback>> = const { RefCell::new(None) };
    static WILL_TERMINATE: RefCell<Option<WillTerminateCallback>> = const { RefCell::new(None) };
    static OPEN_URLS: RefCell<Option<OpenUrlsCallback>> = const { RefCell::new(None) };
}

/// Status bar item wrapper
pub struct StatusItem {
    item: Retained<NSStatusItem>,
    _menu: Retained<NSMenu>,
}

impl StatusItem {
    pub fn new(
        mtm: MainThreadMarker,
        icon_resource_name: &str,
        fallback_sf_symbol: &str,
        tooltip: &str,
        menu: Retained<NSMenu>,
    ) -> Self {
        unsafe {
            let status_bar = NSStatusBar::systemStatusBar();
            let item = status_bar.statusItemWithLength(-1.0); // NSVariableStatusItemLength

            if let Some(button) = item.button(mtm) {
                // Set tooltip
                let tooltip_str = NSString::from_str(tooltip);
                button.setToolTip(Some(&tooltip_str));

                // Try to load custom icon from bundle Resources, fallback to SF Symbol
                let mut image_set = false;

                // Try to find icon in app bundle Resources/assets/ using NSBundle
                // cargo-bundle places resources in a subdirectory matching the source path
                let bundle = NSBundle::mainBundle();
                let resource_name = NSString::from_str(icon_resource_name);
                let resource_type = NSString::from_str("png");
                let subdir = NSString::from_str("assets");

                if let Some(path) = bundle.pathForResource_ofType_inDirectory(Some(&resource_name), Some(&resource_type), Some(&subdir)) {
                    // Load image from file path
                    let image_ptr: *mut NSImage = msg_send![class!(NSImage), alloc];
                    if !image_ptr.is_null() {
                        let image_ptr: *mut NSImage = msg_send![image_ptr, initWithContentsOfFile: &*path];
                        if let Some(image) = Retained::retain(image_ptr) {
                            image.setTemplate(true);
                            button.setImage(Some(&image));
                            image_set = true;
                        }
                    }
                }

                // Fallback to SF Symbol if bundle resource not found
                if !image_set {
                    let icon_str = NSString::from_str(fallback_sf_symbol);
                    if let Some(image) =
                        NSImage::imageWithSystemSymbolName_accessibilityDescription(&icon_str, None)
                    {
                        image.setTemplate(true);
                        button.setImage(Some(&image));
                    }
                }
            }

            item.setMenu(Some(&menu));

            Self { item, _menu: menu }
        }
    }
}

impl Drop for StatusItem {
    fn drop(&mut self) {
        unsafe {
            let status_bar = NSStatusBar::systemStatusBar();
            status_bar.removeStatusItem(&self.item);
        }
    }
}

/// Storage for menu action callbacks
static MENU_ACTIONS: OnceLock<std::sync::Mutex<Vec<Box<dyn Fn() + Send + Sync + 'static>>>> =
    OnceLock::new();

fn menu_actions() -> &'static std::sync::Mutex<Vec<Box<dyn Fn() + Send + Sync + 'static>>> {
    MENU_ACTIONS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Create the MenuActionTarget class
fn menu_action_target_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = NSObject::class();
        let mut builder = ClassBuilder::new(c"CleoMenuActionTarget", superclass)
            .expect("Failed to create class");

        // Add instance variable for action index
        builder.add_ivar::<usize>(c"actionIndex");

        // Add performAction: method
        unsafe extern "C" fn perform_action(this: *mut AnyObject, _sel: Sel, _sender: *mut AnyObject) {
            let cls = (*this).class();
            let ivar = cls.instance_variable(c"actionIndex").unwrap();
            let idx = *ivar.load::<usize>(&*this);
            if let Ok(actions) = menu_actions().lock() {
                if let Some(action) = actions.get(idx) {
                    action();
                }
            }
        }

        unsafe {
            builder.add_method(
                sel!(performAction:),
                perform_action as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
        }

        builder.register()
    })
}

/// Menu builder for creating NSMenu with actions
pub struct MenuBuilder {
    menu: Retained<NSMenu>,
    mtm: MainThreadMarker,
    targets: Vec<Retained<AnyObject>>,
}

impl MenuBuilder {
    pub fn new(mtm: MainThreadMarker, title: &str) -> Self {
        let title_str = NSString::from_str(title);
        let menu = unsafe { NSMenu::initWithTitle(NSMenu::alloc(mtm), &title_str) };
        Self {
            menu,
            mtm,
            targets: Vec::new(),
        }
    }

    /// Add a menu item with an action selector
    pub fn add_item(self, title: &str, action: Option<Sel>, key: &str) -> Self {
        let title_str = NSString::from_str(title);
        let key_str = NSString::from_str(key);
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.mtm),
                &title_str,
                action,
                &key_str,
            )
        };
        self.menu.addItem(&item);
        self
    }

    /// Add a menu item with a closure action
    pub fn add_action_item<F>(mut self, title: &str, key: &str, action: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        let title_str = NSString::from_str(title);
        let key_str = NSString::from_str(key);

        // Store action and get its index
        let idx = {
            let mut actions = menu_actions().lock().unwrap();
            let idx = actions.len();
            actions.push(Box::new(action));
            idx
        };

        // Create target
        let cls = menu_action_target_class();
        let target: *mut AnyObject = unsafe { msg_send![cls, new] };
        let target = unsafe { Retained::retain(target).unwrap() };

        // Set action index
        unsafe {
            let ivar = cls.instance_variable(c"actionIndex").unwrap();
            *ivar.load_mut::<usize>(&mut *Retained::as_ptr(&target).cast_mut()) = idx;
        }

        // Create menu item
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.mtm),
                &title_str,
                Some(sel!(performAction:)),
                &key_str,
            )
        };

        unsafe {
            let _: () = msg_send![&item, setTarget: &*target];
        }

        self.menu.addItem(&item);
        self.targets.push(target);
        self
    }

    /// Add a menu item with a closure action and return a handle
    pub fn add_action_item_with_handle<F>(
        mut self,
        title: &str,
        key: &str,
        action: F,
    ) -> (Self, MenuItemHandle)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let title_str = NSString::from_str(title);
        let key_str = NSString::from_str(key);

        // Store action and get its index
        let idx = {
            let mut actions = menu_actions().lock().unwrap();
            let idx = actions.len();
            actions.push(Box::new(action));
            idx
        };

        // Create target
        let cls = menu_action_target_class();
        let target: *mut AnyObject = unsafe { msg_send![cls, new] };
        let target = unsafe { Retained::retain(target).unwrap() };

        // Set action index
        unsafe {
            let ivar = cls.instance_variable(c"actionIndex").unwrap();
            *ivar.load_mut::<usize>(&mut *Retained::as_ptr(&target).cast_mut()) = idx;
        }

        // Create menu item
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.mtm),
                &title_str,
                Some(sel!(performAction:)),
                &key_str,
            )
        };

        unsafe {
            let _: () = msg_send![&item, setTarget: &*target];
        }

        let handle = MenuItemHandle::new(item.clone());
        self.menu.addItem(&item);
        self.targets.push(target);
        (self, handle)
    }

    /// Add a menu item and return a handle to it
    pub fn add_item_with_handle(
        self,
        title: &str,
        action: Option<Sel>,
        key: &str,
    ) -> (Self, MenuItemHandle) {
        let title_str = NSString::from_str(title);
        let key_str = NSString::from_str(key);
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.mtm),
                &title_str,
                action,
                &key_str,
            )
        };
        let handle = MenuItemHandle::new(item.clone());
        self.menu.addItem(&item);
        (self, handle)
    }

    /// Add a separator item
    pub fn add_separator(self) -> Self {
        let sep = NSMenuItem::separatorItem(self.mtm);
        self.menu.addItem(&sep);
        self
    }

    /// Add a disabled label item
    pub fn add_disabled_label(self, title: &str) -> Self {
        let title_str = NSString::from_str(title);
        let key_str = NSString::from_str("");
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.mtm),
                &title_str,
                None,
                &key_str,
            )
        };
        item.setEnabled(false);
        self.menu.addItem(&item);
        self
    }

    /// Build and return the menu and targets (targets must be kept alive)
    pub fn build(self) -> (Retained<NSMenu>, Vec<Retained<AnyObject>>) {
        (self.menu, self.targets)
    }
}

/// Handle to a menu item for updating it later
#[derive(Clone)]
pub struct MenuItemHandle {
    item: Retained<NSMenuItem>,
}

impl MenuItemHandle {
    pub fn new(item: Retained<NSMenuItem>) -> Self {
        Self { item }
    }

    pub fn set_title(&self, title: &str) {
        let title_str = NSString::from_str(title);
        self.item.setTitle(&title_str);
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.item.setEnabled(enabled);
    }

    pub fn item(&self) -> &NSMenuItem {
        &self.item
    }
}

/// Create the AppDelegate class using ClassBuilder
fn app_delegate_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = NSObject::class();
        let mut builder =
            ClassBuilder::new(c"CleoAppDelegate", superclass).expect("Failed to create class");

        // Add applicationDidFinishLaunching: method
        unsafe extern "C" fn did_finish_launching(_this: *mut AnyObject, _sel: Sel, _notif: *mut AnyObject) {
            DID_FINISH_LAUNCHING.with(|cb| {
                if let Some(callback) = cb.borrow_mut().as_mut() {
                    callback();
                }
            });
        }

        unsafe {
            builder.add_method(
                sel!(applicationDidFinishLaunching:),
                did_finish_launching as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
        }

        // Add applicationShouldTerminate: method
        unsafe extern "C" fn should_terminate(
            _this: *mut AnyObject,
            _sel: Sel,
            _sender: *mut AnyObject,
        ) -> usize {
            let should = SHOULD_TERMINATE.with(|cb| {
                if let Some(callback) = cb.borrow().as_ref() {
                    callback()
                } else {
                    true
                }
            });
            if should { 1 } else { 2 } // NSTerminateNow = 1, NSTerminateCancel = 2
        }

        unsafe {
            builder.add_method(
                sel!(applicationShouldTerminate:),
                should_terminate as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize,
            );
        }

        // Add applicationWillTerminate: method
        unsafe extern "C" fn will_terminate(_this: *mut AnyObject, _sel: Sel, _notif: *mut AnyObject) {
            WILL_TERMINATE.with(|cb| {
                if let Some(callback) = cb.borrow_mut().as_mut() {
                    callback();
                }
            });
        }

        unsafe {
            builder.add_method(
                sel!(applicationWillTerminate:),
                will_terminate as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
        }

        // Add application:openURLs: method
        unsafe extern "C" fn open_urls(
            _this: *mut AnyObject,
            _sel: Sel,
            _app: *mut AnyObject,
            urls: *mut AnyObject,
        ) {
            OPEN_URLS.with(|cb| {
                if let Some(callback) = cb.borrow_mut().as_mut() {
                    // Convert NSArray<NSURL> to Vec<Url>
                    let ns_urls: &NSArray<NSURL> = &*(urls as *const NSArray<NSURL>);
                    let mut rust_urls = Vec::new();
                    for i in 0..ns_urls.len() {
                        let nsurl = ns_urls.objectAtIndex(i);
                        if let Some(string) = nsurl.absoluteString() {
                            if let Ok(url) = Url::parse(&string.to_string()) {
                                rust_urls.push(url);
                            }
                        }
                    }
                    callback(rust_urls);
                }
            });
        }

        unsafe {
            builder.add_method(
                sel!(application:openURLs:),
                open_urls as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
            );
        }

        builder.register()
    })
}

/// Application runner
pub struct App {
    mtm: MainThreadMarker,
    app: Retained<NSApplication>,
    delegate: Retained<AnyObject>,
}

impl App {
    /// Create a new application
    pub fn new(mtm: MainThreadMarker) -> Self {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

        // Create delegate instance
        let delegate_class = app_delegate_class();
        let delegate: *mut AnyObject = unsafe { msg_send![delegate_class, new] };
        let delegate = unsafe { Retained::retain(delegate).unwrap() };

        // Set delegate
        unsafe {
            let _: () = msg_send![&app, setDelegate: &*delegate];
        }

        Self { mtm, app, delegate }
    }

    /// Set the callback for applicationDidFinishLaunching
    pub fn on_did_finish_launching<F: FnMut() + 'static>(&self, callback: F) {
        DID_FINISH_LAUNCHING.with(|cb| {
            *cb.borrow_mut() = Some(Box::new(callback));
        });
    }

    /// Set the callback for applicationShouldTerminate
    pub fn on_should_terminate<F: Fn() -> bool + 'static>(&self, callback: F) {
        SHOULD_TERMINATE.with(|cb| {
            *cb.borrow_mut() = Some(Box::new(callback));
        });
    }

    /// Set the callback for applicationWillTerminate
    pub fn on_will_terminate<F: FnMut() + 'static>(&self, callback: F) {
        WILL_TERMINATE.with(|cb| {
            *cb.borrow_mut() = Some(Box::new(callback));
        });
    }

    /// Set the callback for application:openURLs:
    pub fn on_open_urls<F: FnMut(Vec<Url>) + 'static>(&self, callback: F) {
        OPEN_URLS.with(|cb| {
            *cb.borrow_mut() = Some(Box::new(callback));
        });
    }

    /// Get the main thread marker
    pub fn mtm(&self) -> MainThreadMarker {
        self.mtm
    }

    /// Get the NSApplication instance
    pub fn ns_app(&self) -> &NSApplication {
        &self.app
    }

    /// Run the application event loop
    pub fn run(self) {
        unsafe {
            self.app.run();
        }
    }

    /// Run one iteration of the event loop (for polling)
    pub fn run_once(&self) {
        unsafe {
            let run_loop = objc2_foundation::NSRunLoop::currentRunLoop();
            let date: *mut AnyObject = msg_send![class!(NSDate), distantPast];
            let _: bool = msg_send![&run_loop, runMode: objc2_foundation::NSDefaultRunLoopMode, beforeDate: date];
        }
    }
}

/// Terminate the application
pub fn terminate() {
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        unsafe {
            app.terminate(None);
        }
    }
}
