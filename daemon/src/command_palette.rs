//! Command palette UI - a Spotlight/Raycast-style floating panel for quick actions.

use std::cell::{Cell, RefCell};
use std::fmt;

use block::{ConcreteBlock, RcBlock};
use cacao::foundation::{NO, NSString, YES, id, nil};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager,
    hotkey::{Code, HotKey, Modifiers},
};
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use objc_id::ShareId;

/// Key codes for keyboard handling
const KEY_ESCAPE: u16 = 53;
const KEY_RETURN: u16 = 36;
const KEY_UP: u16 = 126;
const KEY_DOWN: u16 = 125;
const KEY_T: u16 = 17;
const KEY_R: u16 = 15;
const KEY_S: u16 = 1;

/// NSEvent type masks
const NS_KEY_DOWN_MASK: u64 = 1 << 10;

/// Commands available in the palette
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteCommand {
    ToggleCapture,
    ToggleRecording,
    TakeScreenshot,
}

impl PaletteCommand {
    pub fn all() -> &'static [PaletteCommand] {
        &[
            PaletteCommand::ToggleCapture,
            PaletteCommand::ToggleRecording,
            PaletteCommand::TakeScreenshot,
        ]
    }

    pub fn keybind(&self) -> &'static str {
        match self {
            PaletteCommand::ToggleCapture => "T",
            PaletteCommand::ToggleRecording => "R",
            PaletteCommand::TakeScreenshot => "S",
        }
    }

    pub fn icon_name(&self) -> &'static str {
        match self {
            PaletteCommand::ToggleCapture => "power",
            PaletteCommand::ToggleRecording => "record.circle",
            PaletteCommand::TakeScreenshot => "camera",
        }
    }

    pub fn label(&self, auto_capture_enabled: bool, recording: bool) -> &'static str {
        match self {
            PaletteCommand::ToggleCapture => {
                if auto_capture_enabled {
                    "Auto Capture: ON"
                } else {
                    "Auto Capture: OFF"
                }
            }
            PaletteCommand::ToggleRecording => {
                if recording {
                    "Stop Recording"
                } else {
                    "Start Recording"
                }
            }
            PaletteCommand::TakeScreenshot => "Take Screenshot",
        }
    }
}

#[derive(Debug)]
pub enum CommandPaletteError {
    PanelCreationFailed,
    MonitorUnavailable,
}

impl fmt::Display for CommandPaletteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandPaletteError::PanelCreationFailed => {
                write!(f, "Failed to create command palette panel")
            }
            CommandPaletteError::MonitorUnavailable => {
                write!(f, "Failed to install keyboard monitor")
            }
        }
    }
}

impl std::error::Error for CommandPaletteError {}

/// The command palette floating panel
pub struct CommandPalette {
    panel: ShareId<Object>,
    command_labels: RefCell<Vec<ShareId<Object>>>,
    icon_views: RefCell<Vec<ShareId<Object>>>,
    selection_views: RefCell<Vec<ShareId<Object>>>,
    local_monitor: RefCell<Option<ShareId<Object>>>,
    _monitor_block: RefCell<Option<RcBlock<(*mut Object,), *mut Object>>>,
    visible: Cell<bool>,
    selected_index: Cell<usize>,
    auto_capture_enabled: Cell<bool>,
    recording: Cell<bool>,
}

// NSRect for layout
#[repr(C)]
#[derive(Clone, Copy)]
struct NSRect {
    origin: NSPoint,
    size: NSSize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSSize {
    width: f64,
    height: f64,
}

impl CommandPalette {
    /// Create a new command palette (hidden by default)
    pub fn new() -> Result<Self, CommandPaletteError> {
        let commands = PaletteCommand::all();

        // Refined dimensions - more Spotlight-like
        let row_height = 52.0;
        let header_height = 48.0;
        let content_padding = 6.0;
        let panel_width = 580.0;
        let panel_height =
            header_height + (commands.len() as f64 * row_height) + content_padding * 2.0 + 8.0;

        // Get screen frame to center the panel
        let screen_frame = unsafe {
            let screen: id = msg_send![class!(NSScreen), mainScreen];
            let frame: NSRect = msg_send![screen, frame];
            frame
        };

        let panel_x = (screen_frame.size.width - panel_width) / 2.0;
        let panel_y = screen_frame.size.height * 0.65; // Upper third of screen like Spotlight

        let frame = NSRect {
            origin: NSPoint {
                x: panel_x,
                y: panel_y,
            },
            size: NSSize {
                width: panel_width,
                height: panel_height,
            },
        };

        // Borderless window for clean look - use Titled style to allow becoming key window properly
        // NSWindowStyleMaskTitled = 1 << 0
        // NSWindowStyleMaskFullSizeContentView = 1 << 15
        let style_mask: u64 = (1 << 0) | (1 << 15); // Titled + FullSizeContentView

        let panel: id = unsafe {
            let panel: id = msg_send![class!(NSPanel), alloc];
            let panel: id = msg_send![panel,
                initWithContentRect: frame
                styleMask: style_mask
                backing: 2u64  // NSBackingStoreBuffered
                defer: NO
            ];

            if panel.is_null() {
                return Err(CommandPaletteError::PanelCreationFailed);
            }

            // Configure panel behavior
            let _: () = msg_send![panel, setLevel: 25isize]; // NSPopUpMenuWindowLevel
            let _: () = msg_send![panel, setMovableByWindowBackground: YES];
            let _: () = msg_send![panel, setTitlebarAppearsTransparent: YES];
            let _: () = msg_send![panel, setTitleVisibility: 1isize]; // NSWindowTitleHidden

            // Transparent background - the visual effect view handles appearance
            let _: () = msg_send![panel, setOpaque: NO];
            let clear: id = msg_send![class!(NSColor), clearColor];
            let _: () = msg_send![panel, setBackgroundColor: clear];

            // Panel behavior - must be able to become key to receive keyboard input
            let _: () = msg_send![panel, setFloatingPanel: YES];
            let _: () = msg_send![panel, setBecomesKeyOnlyIfNeeded: NO]; // Always become key
            let _: () = msg_send![panel, setWorksWhenModal: YES];

            // IMPORTANT: Disable hidesOnDeactivate so we control visibility ourselves
            let _: () = msg_send![panel, setHidesOnDeactivate: NO];

            // Add shadow for depth
            let _: () = msg_send![panel, setHasShadow: YES];

            panel
        };

        // Create visual effect view for frosted glass background
        let content_view: id = unsafe {
            let bounds = NSRect {
                origin: NSPoint { x: 0.0, y: 0.0 },
                size: NSSize {
                    width: panel_width,
                    height: panel_height,
                },
            };

            let view: id = msg_send![class!(NSVisualEffectView), alloc];
            let view: id = msg_send![view, initWithFrame: bounds];

            // Force dark appearance for Raycast-like look
            // This makes it always dark regardless of system appearance
            let dark_appearance_name = NSString::new("NSAppearanceNameDarkAqua");
            let dark_appearance: id = msg_send![class!(NSAppearance), appearanceNamed: &*dark_appearance_name];
            if !dark_appearance.is_null() {
                let _: () = msg_send![view, setAppearance: dark_appearance];
            }

            // Use sidebar material with dark appearance for translucent dark look
            // NSVisualEffectMaterialSidebar = 7 - works well with dark appearance
            let _: () = msg_send![view, setMaterial: 7isize];
            // NSVisualEffectBlendingModeBehindWindow = 0 for blur-through
            let _: () = msg_send![view, setBlendingMode: 0isize];
            let _: () = msg_send![view, setState: 1isize]; // Active
            let _: () = msg_send![view, setEmphasized: YES];

            // Round corners with proper clipping - larger radius for modern look
            let _: () = msg_send![view, setWantsLayer: YES];
            let layer: id = msg_send![view, layer];
            let _: () = msg_send![layer, setCornerRadius: 12.0f64];
            let _: () = msg_send![layer, setMasksToBounds: YES];

            let _: () = msg_send![panel, setContentView: view];

            view
        };

        // Create header with app name
        unsafe {
            let header_frame = NSRect {
                origin: NSPoint {
                    x: 20.0,
                    y: panel_height - header_height + 8.0,
                },
                size: NSSize {
                    width: panel_width - 40.0,
                    height: 28.0,
                },
            };

            let label: id = msg_send![class!(NSTextField), alloc];
            let label: id = msg_send![label, initWithFrame: header_frame];

            let title = NSString::new("Cleo");
            let _: () = msg_send![label, setStringValue: &*title];
            let _: () = msg_send![label, setBezeled: NO];
            let _: () = msg_send![label, setDrawsBackground: NO];
            let _: () = msg_send![label, setEditable: NO];
            let _: () = msg_send![label, setSelectable: NO];

            // Semibold system font - slightly larger
            let font: id = msg_send![class!(NSFont), systemFontOfSize: 14.0f64 weight: 0.3f64];
            let _: () = msg_send![label, setFont: font];
            let color: id = msg_send![class!(NSColor), secondaryLabelColor];
            let _: () = msg_send![label, setTextColor: color];

            let _: () = msg_send![content_view, addSubview: label];

            // Add subtle separator line
            let separator_frame = NSRect {
                origin: NSPoint {
                    x: 12.0,
                    y: panel_height - header_height,
                },
                size: NSSize {
                    width: panel_width - 24.0,
                    height: 1.0,
                },
            };
            let separator: id = msg_send![class!(NSBox), alloc];
            let separator: id = msg_send![separator, initWithFrame: separator_frame];
            let _: () = msg_send![separator, setBoxType: 3isize]; // NSBoxSeparator
            let _: () = msg_send![content_view, addSubview: separator];
        }

        // Create command rows
        let mut command_labels = Vec::new();
        let mut icon_views = Vec::new();
        let mut selection_views = Vec::new();

        for (i, cmd) in commands.iter().enumerate() {
            let row_y = panel_height - header_height - content_padding - ((i + 1) as f64 * row_height);
            let center_y = row_y + (row_height / 2.0);

            // Selection/hover background - rounded rect with larger radius
            let selection_frame = NSRect {
                origin: NSPoint {
                    x: content_padding + 6.0,
                    y: row_y + 4.0,
                },
                size: NSSize {
                    width: panel_width - content_padding * 2.0 - 12.0,
                    height: row_height - 8.0,
                },
            };

            let selection_view: id = unsafe {
                let view: id = msg_send![class!(NSView), alloc];
                let view: id = msg_send![view, initWithFrame: selection_frame];
                let _: () = msg_send![view, setWantsLayer: YES];
                let layer: id = msg_send![view, layer];
                let _: () = msg_send![layer, setCornerRadius: 8.0f64];
                let clear: id = msg_send![class!(NSColor), clearColor];
                let cg: id = msg_send![clear, CGColor];
                let _: () = msg_send![layer, setBackgroundColor: cg];
                let _: () = msg_send![content_view, addSubview: view];
                view
            };
            selection_views.push(unsafe { ShareId::from_ptr(selection_view) });

            // Icon - centered vertically with nice size
            let icon_size = 24.0;
            let icon_frame = NSRect {
                origin: NSPoint {
                    x: content_padding + 20.0,
                    y: center_y - icon_size / 2.0,
                },
                size: NSSize {
                    width: icon_size,
                    height: icon_size,
                },
            };
            let image_view: id = unsafe {
                let image_view: id = msg_send![class!(NSImageView), alloc];
                let image_view: id = msg_send![image_view, initWithFrame: icon_frame];

                let icon_name = NSString::new(cmd.icon_name());
                let config: id = msg_send![class!(NSImageSymbolConfiguration),
                    configurationWithPointSize: 18.0f64
                    weight: 2isize  // NSFontWeightMedium
                    scale: 2isize   // NSImageSymbolScaleMedium
                ];
                let image: id = msg_send![class!(NSImage),
                    imageWithSystemSymbolName: &*icon_name
                    accessibilityDescription: nil
                ];
                if !image.is_null() {
                    let configured: id = msg_send![image, imageWithSymbolConfiguration: config];
                    let _: () = msg_send![image_view, setImage: configured];
                    let color: id = msg_send![class!(NSColor), secondaryLabelColor];
                    let _: () = msg_send![image_view, setContentTintColor: color];
                }
                let _: () = msg_send![image_view, setImageAlignment: 0isize]; // NSImageAlignCenter

                let _: () = msg_send![content_view, addSubview: image_view];
                image_view
            };
            icon_views.push(unsafe { ShareId::from_ptr(image_view) });

            // Command label - centered vertically, medium weight
            let label_height = 20.0;
            let label_frame = NSRect {
                origin: NSPoint {
                    x: content_padding + 56.0,
                    y: center_y - label_height / 2.0,
                },
                size: NSSize {
                    width: panel_width - 180.0,
                    height: label_height,
                },
            };
            let label: id = unsafe {
                let label: id = msg_send![class!(NSTextField), alloc];
                let label: id = msg_send![label, initWithFrame: label_frame];

                let text = NSString::new(cmd.label(true, false));
                let _: () = msg_send![label, setStringValue: &*text];
                let _: () = msg_send![label, setBezeled: NO];
                let _: () = msg_send![label, setDrawsBackground: NO];
                let _: () = msg_send![label, setEditable: NO];
                let _: () = msg_send![label, setSelectable: NO];

                // Medium weight for commands - slightly larger
                let font: id = msg_send![class!(NSFont), systemFontOfSize: 14.0f64 weight: 0.0f64];
                let _: () = msg_send![label, setFont: font];
                let color: id = msg_send![class!(NSColor), labelColor];
                let _: () = msg_send![label, setTextColor: color];

                let _: () = msg_send![content_view, addSubview: label];
                label
            };
            command_labels.push(unsafe { ShareId::from_ptr(label) });

            // Keyboard shortcut badge - pill-shaped background
            let badge_width = 26.0;
            let badge_height = 22.0;
            let badge_x = panel_width - content_padding - 24.0 - badge_width;
            let badge_y = center_y - badge_height / 2.0;

            unsafe {
                // Badge background - subtle rounded rect
                let badge_frame = NSRect {
                    origin: NSPoint {
                        x: badge_x,
                        y: badge_y,
                    },
                    size: NSSize {
                        width: badge_width,
                        height: badge_height,
                    },
                };
                let badge_bg: id = msg_send![class!(NSView), alloc];
                let badge_bg: id = msg_send![badge_bg, initWithFrame: badge_frame];
                let _: () = msg_send![badge_bg, setWantsLayer: YES];
                let layer: id = msg_send![badge_bg, layer];
                let _: () = msg_send![layer, setCornerRadius: 5.0f64];

                // Use a subtle background
                let bg_color: id = msg_send![class!(NSColor), tertiaryLabelColor];
                let alpha_color: id = msg_send![bg_color, colorWithAlphaComponent: 0.15f64];
                let cg: id = msg_send![alpha_color, CGColor];
                let _: () = msg_send![layer, setBackgroundColor: cg];

                let _: () = msg_send![content_view, addSubview: badge_bg];

                // Badge text
                let text_frame = NSRect {
                    origin: NSPoint {
                        x: badge_x,
                        y: badge_y + 2.0,
                    },
                    size: NSSize {
                        width: badge_width,
                        height: badge_height - 4.0,
                    },
                };
                let hint: id = msg_send![class!(NSTextField), alloc];
                let hint: id = msg_send![hint, initWithFrame: text_frame];

                let text = NSString::new(cmd.keybind());
                let _: () = msg_send![hint, setStringValue: &*text];
                let _: () = msg_send![hint, setBezeled: NO];
                let _: () = msg_send![hint, setDrawsBackground: NO];
                let _: () = msg_send![hint, setEditable: NO];
                let _: () = msg_send![hint, setSelectable: NO];
                let _: () = msg_send![hint, setAlignment: 1isize]; // NSTextAlignmentCenter

                // Slightly bolder monospace font
                let font: id = msg_send![class!(NSFont),
                    monospacedSystemFontOfSize: 12.0f64
                    weight: 0.3f64
                ];
                let _: () = msg_send![hint, setFont: font];
                let color: id = msg_send![class!(NSColor), tertiaryLabelColor];
                let _: () = msg_send![hint, setTextColor: color];

                let _: () = msg_send![content_view, addSubview: hint];
            }
        }

        let palette = Self {
            panel: unsafe { ShareId::from_ptr(panel) },
            command_labels: RefCell::new(command_labels),
            icon_views: RefCell::new(icon_views),
            selection_views: RefCell::new(selection_views),
            local_monitor: RefCell::new(None),
            _monitor_block: RefCell::new(None),
            visible: Cell::new(false),
            selected_index: Cell::new(0),
            auto_capture_enabled: Cell::new(true),
            recording: Cell::new(false),
        };

        palette.update_selection();

        Ok(palette)
    }

    /// Show the command palette
    pub fn show(&self) {
        eprintln!("[palette.show] called, visible={}", self.visible.get());
        if self.visible.get() {
            eprintln!("[palette.show] already visible, returning early");
            return;
        }

        // Clean up any stale local monitor
        self.uninstall_local_monitor();
        eprintln!("[palette.show] proceeding to show panel");

        unsafe {
            // Center on screen (in case resolution changed)
            let _: () = msg_send![&*self.panel, center];

            // Show panel and make it key
            let _: () = msg_send![&*self.panel, makeKeyAndOrderFront: nil];

            // Activate the app so the panel can receive keyboard input
            let app: id = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, activateIgnoringOtherApps: YES];

            // Ensure panel is key window
            let _: () = msg_send![&*self.panel, makeKeyWindow];
        }

        self.visible.set(true);
        self.selected_index.set(0);
        self.update_selection();
    }

    /// Hide the command palette
    pub fn hide(&self) {
        eprintln!("[palette.hide] called, setting visible=false");
        self.visible.set(false);
        self.uninstall_local_monitor();
        unsafe {
            let _: () = msg_send![&*self.panel, orderOut: nil];
        }
    }

    /// Call this when the panel loses focus (for HidesOnDeactivate handling)
    pub fn on_deactivate(&self) {
        eprintln!("[palette.on_deactivate] called, setting visible=false");
        self.visible.set(false);
        self.uninstall_local_monitor();
    }

    /// Toggle visibility
    pub fn toggle(&self) {
        if self.visible.get() {
            self.hide();
        } else {
            self.show();
        }
    }

    /// Check if visible
    pub fn is_visible(&self) -> bool {
        self.visible.get()
    }

    /// Get the panel pointer for visibility checks
    pub fn panel_ptr(&self) -> *mut Object {
        &*self.panel as *const Object as *mut Object
    }

    /// Update state (call when auto_capture_enabled or recording changes)
    pub fn update_state(&self, auto_capture_enabled: bool, recording: bool) {
        self.auto_capture_enabled.set(auto_capture_enabled);
        self.recording.set(recording);
        self.update_labels();
    }

    /// Handle a key event, returns the command if one was triggered
    pub fn handle_key(&self, key_code: u16) -> Option<PaletteCommand> {
        match key_code {
            KEY_ESCAPE => {
                self.hide();
                None
            }
            KEY_UP => {
                let idx = self.selected_index.get();
                if idx > 0 {
                    self.selected_index.set(idx - 1);
                    self.update_selection();
                }
                None
            }
            KEY_DOWN => {
                let idx = self.selected_index.get();
                let max = PaletteCommand::all().len() - 1;
                if idx < max {
                    self.selected_index.set(idx + 1);
                    self.update_selection();
                }
                None
            }
            KEY_RETURN => {
                let cmd = PaletteCommand::all()[self.selected_index.get()];
                // Hide for screenshot, keep open for toggles
                if cmd == PaletteCommand::TakeScreenshot {
                    self.hide();
                }
                Some(cmd)
            }
            KEY_T => Some(PaletteCommand::ToggleCapture),
            KEY_R => Some(PaletteCommand::ToggleRecording),
            KEY_S => {
                self.hide();
                Some(PaletteCommand::TakeScreenshot)
            }
            _ => None,
        }
    }

    /// Install local keyboard monitor for when panel is visible
    pub fn install_local_monitor<F>(&self, on_key: F)
    where
        F: Fn(u16) + Send + 'static,
    {
        if self.local_monitor.borrow().is_some() {
            return;
        }

        let block = ConcreteBlock::new(move |event: *mut Object| -> *mut Object {
            let key_code: u16 = unsafe { msg_send![event, keyCode] };
            on_key(key_code);
            // Return nil to consume the event
            std::ptr::null_mut()
        })
        .copy();

        let monitor: *mut Object = unsafe {
            msg_send![
                class!(NSEvent),
                addLocalMonitorForEventsMatchingMask: NS_KEY_DOWN_MASK
                handler: &*block
            ]
        };

        if !monitor.is_null() {
            self.local_monitor
                .replace(Some(unsafe { ShareId::from_ptr(monitor) }));
            self._monitor_block.replace(Some(block));
        }
    }

    /// Remove local keyboard monitor
    pub fn uninstall_local_monitor(&self) {
        if let Some(monitor) = self.local_monitor.borrow_mut().take() {
            unsafe {
                let _: () = msg_send![class!(NSEvent), removeMonitor: &*monitor];
            }
        }
        self._monitor_block.borrow_mut().take();
    }

    fn update_selection(&self) {
        let selected = self.selected_index.get();
        let views = self.selection_views.borrow();
        let icons = self.icon_views.borrow();

        for (i, view) in views.iter().enumerate() {
            let is_selected = i == selected;
            unsafe {
                let layer: id = msg_send![&**view, layer];

                if is_selected {
                    // Use system accent color (orange) for selection
                    let accent: id = msg_send![class!(NSColor), controlAccentColor];
                    let cg: id = msg_send![accent, CGColor];
                    let _: () = msg_send![layer, setBackgroundColor: cg];
                } else {
                    let clear: id = msg_send![class!(NSColor), clearColor];
                    let cg: id = msg_send![clear, CGColor];
                    let _: () = msg_send![layer, setBackgroundColor: cg];
                }

                // Update icon color based on selection - white on accent, gray otherwise
                if let Some(icon) = icons.get(i) {
                    let color: id = if is_selected {
                        msg_send![class!(NSColor), whiteColor]
                    } else {
                        msg_send![class!(NSColor), secondaryLabelColor]
                    };
                    let _: () = msg_send![&**icon, setContentTintColor: color];
                }
            }
        }

        // Update label colors for selected row - white on accent, normal otherwise
        let labels = self.command_labels.borrow();
        for (i, label) in labels.iter().enumerate() {
            let is_selected = i == selected;
            unsafe {
                let color: id = if is_selected {
                    msg_send![class!(NSColor), whiteColor]
                } else {
                    msg_send![class!(NSColor), labelColor]
                };
                let _: () = msg_send![&**label, setTextColor: color];
            }
        }
    }

    fn update_labels(&self) {
        let auto_capture_enabled = self.auto_capture_enabled.get();
        let recording = self.recording.get();
        let labels = self.command_labels.borrow();

        for (i, cmd) in PaletteCommand::all().iter().enumerate() {
            if let Some(label) = labels.get(i) {
                let text = cmd.label(auto_capture_enabled, recording);
                unsafe {
                    let ns_text = NSString::new(text);
                    let _: () = msg_send![&**label, setStringValue: &*ns_text];
                }
            }
        }
    }
}

impl Drop for CommandPalette {
    fn drop(&mut self) {
        self.uninstall_local_monitor();
        unsafe {
            let _: () = msg_send![&*self.panel, close];
        }
    }
}

/// Global hotkey tracker using the global-hotkey crate
/// This properly intercepts the hotkey before other apps see it
///
/// IMPORTANT: On macOS, this must be created and polled on the main thread!
pub struct HotkeyTracker {
    manager: GlobalHotKeyManager,
    hotkey: HotKey,
}

impl HotkeyTracker {
    /// Create a new hotkey tracker for Cmd+Shift+C
    /// Must be called on the main thread on macOS!
    pub fn new() -> Result<Self, CommandPaletteError> {
        // Create the hotkey manager (must be on main thread for macOS)
        let manager =
            GlobalHotKeyManager::new().map_err(|_| CommandPaletteError::MonitorUnavailable)?;

        // Define Cmd+Shift+C hotkey
        let hotkey = HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyC);

        // Register the hotkey
        manager
            .register(hotkey)
            .map_err(|_| CommandPaletteError::MonitorUnavailable)?;

        Ok(Self { manager, hotkey })
    }

    /// Check for hotkey events (call this from main thread periodically)
    /// Returns true if the hotkey was pressed
    pub fn poll(&self) -> bool {
        let receiver = GlobalHotKeyEvent::receiver();
        while let Ok(event) = receiver.try_recv() {
            if event.id == self.hotkey.id() && event.state == global_hotkey::HotKeyState::Pressed {
                return true;
            }
        }
        false
    }
}

impl Drop for HotkeyTracker {
    fn drop(&mut self) {
        // Unregister the hotkey when dropped
        let _ = self.manager.unregister(self.hotkey);
    }
}
