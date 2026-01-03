//! Command palette UI - a Spotlight/Raycast-style floating panel for quick actions.

use std::cell::{Cell, RefCell};
use std::fmt;

use block2::RcBlock;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager,
    hotkey::{Code, HotKey, Modifiers},
};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadOnly, msg_send, ClassType};
use objc2_app_kit::{
    NSColor, NSEvent, NSEventMask, NSFont, NSImage, NSImageSymbolConfiguration,
    NSImageSymbolScale, NSImageView, NSPanel, NSScreen, NSTextField, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowStyleMask, NSBox, NSBoxType,
};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGColor;
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

// Font weight constants (CGFloat values)
const FONT_WEIGHT_REGULAR: f64 = 0.0;
const FONT_WEIGHT_MEDIUM: f64 = 0.23;
const FONT_WEIGHT_SEMIBOLD: f64 = 0.3;

/// Create a clear (transparent) CGColor
fn cg_clear_color() -> CFRetained<CGColor> {
    CGColor::new_srgb(0.0, 0.0, 0.0, 0.0)
}

/// Create the system accent color as CGColor (approximating controlAccentColor)
fn cg_accent_color() -> CFRetained<CGColor> {
    // macOS default accent color is blue, but we use orange for this app
    // RGB: 255, 149, 0 -> normalized: 1.0, 0.584, 0.0
    CGColor::new_srgb(1.0, 0.584, 0.0, 1.0)
}

/// Create a tertiary label color as CGColor (semi-transparent for badges)
fn cg_tertiary_label_color() -> CFRetained<CGColor> {
    // Approximate tertiary label - light gray with transparency
    CGColor::new_srgb(0.0, 0.0, 0.0, 0.1)
}


/// Key codes for keyboard handling
const KEY_ESCAPE: u16 = 53;
const KEY_RETURN: u16 = 36;
const KEY_UP: u16 = 126;
const KEY_DOWN: u16 = 125;
const KEY_T: u16 = 17;
const KEY_R: u16 = 15;
const KEY_S: u16 = 1;
const KEY_B: u16 = 11;

/// Commands available in the palette
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteCommand {
    ToggleCapture,
    ToggleRecording,
    TakeScreenshot,
    ToggleBanApp,
}

/// State needed to render command labels
pub struct PaletteState {
    pub auto_capture_enabled: bool,
    pub recording: bool,
    pub current_app_name: Option<String>,
    pub current_app_banned: bool,
}

impl PaletteCommand {
    pub fn all() -> &'static [PaletteCommand] {
        &[
            PaletteCommand::ToggleCapture,
            PaletteCommand::ToggleRecording,
            PaletteCommand::TakeScreenshot,
            PaletteCommand::ToggleBanApp,
        ]
    }

    pub fn keybind(&self) -> &'static str {
        match self {
            PaletteCommand::ToggleCapture => "T",
            PaletteCommand::ToggleRecording => "R",
            PaletteCommand::TakeScreenshot => "S",
            PaletteCommand::ToggleBanApp => "B",
        }
    }

    pub fn icon_name(&self) -> &'static str {
        match self {
            PaletteCommand::ToggleCapture => "power",
            PaletteCommand::ToggleRecording => "record.circle",
            PaletteCommand::TakeScreenshot => "camera",
            PaletteCommand::ToggleBanApp => "eye.slash",
        }
    }

    pub fn label(&self, state: &PaletteState) -> String {
        match self {
            PaletteCommand::ToggleCapture => {
                if state.auto_capture_enabled {
                    "Auto Capture: ON".to_string()
                } else {
                    "Auto Capture: OFF".to_string()
                }
            }
            PaletteCommand::ToggleRecording => {
                if state.recording {
                    "Stop Recording".to_string()
                } else {
                    "Start Recording".to_string()
                }
            }
            PaletteCommand::TakeScreenshot => "Take Screenshot".to_string(),
            PaletteCommand::ToggleBanApp => {
                match &state.current_app_name {
                    Some(name) if state.current_app_banned => format!("Unban {}", name),
                    Some(name) => format!("Ban {}", name),
                    None => "Ban Current App".to_string(),
                }
            }
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
    panel: Retained<NSPanel>,
    command_labels: RefCell<Vec<Retained<NSTextField>>>,
    icon_views: RefCell<Vec<Retained<NSImageView>>>,
    selection_views: RefCell<Vec<Retained<NSView>>>,
    local_monitor: RefCell<Option<Retained<AnyObject>>>,
    _monitor_block: RefCell<Option<RcBlock<dyn Fn(*mut AnyObject) -> *mut AnyObject>>>,
    visible: Cell<bool>,
    selected_index: Cell<usize>,
    auto_capture_enabled: Cell<bool>,
    recording: Cell<bool>,
    current_app_name: RefCell<Option<String>>,
    current_app_banned: Cell<bool>,
}

impl CommandPalette {
    /// Create a new command palette (hidden by default)
    pub fn new() -> Result<Self, CommandPaletteError> {
        let mtm = MainThreadMarker::new().ok_or(CommandPaletteError::PanelCreationFailed)?;
        let commands = PaletteCommand::all();

        // Refined dimensions - more Spotlight-like
        let row_height: f64 = 52.0;
        let header_height: f64 = 48.0;
        let content_padding: f64 = 6.0;
        let panel_width: f64 = 580.0;
        let panel_height: f64 =
            header_height + (commands.len() as f64 * row_height) + content_padding * 2.0 + 8.0;

        // Get screen frame to center the panel
        let screen_frame = unsafe {
            let screen = NSScreen::mainScreen(mtm).ok_or(CommandPaletteError::PanelCreationFailed)?;
            screen.frame()
        };

        let panel_x = (screen_frame.size.width - panel_width) / 2.0;
        let panel_y = screen_frame.size.height * 0.65; // Upper third of screen like Spotlight

        let frame = NSRect::new(
            NSPoint::new(panel_x, panel_y),
            NSSize::new(panel_width, panel_height),
        );

        // NSWindowStyleMaskTitled | NSWindowStyleMaskNonactivatingPanel | NSWindowStyleMaskFullSizeContentView
        let style_mask = NSWindowStyleMask::Titled
            | NSWindowStyleMask::NonactivatingPanel
            | NSWindowStyleMask::FullSizeContentView;

        let panel = unsafe {
            let panel = NSPanel::alloc(mtm);
            let panel: Retained<NSPanel> = msg_send![
                panel,
                initWithContentRect: frame,
                styleMask: style_mask,
                backing: 2u64,  // NSBackingStoreBuffered
                defer: false
            ];

            // Panel behavior
            panel.setFloatingPanel(true);
            panel.setLevel(25); // NSPopUpMenuWindowLevel for proper stacking
            panel.setMovableByWindowBackground(true);

            // Hide title bar completely
            panel.setTitlebarAppearsTransparent(true);
            panel.setTitleVisibility(objc2_app_kit::NSWindowTitleVisibility::Hidden);

            // Transparent background - the visual effect view handles appearance
            panel.setOpaque(false);
            let clear = NSColor::clearColor();
            panel.setBackgroundColor(Some(&clear));

            // Panel behavior - must be able to become key to receive keyboard input
            let _: () = msg_send![&panel, setBecomesKeyOnlyIfNeeded: false];
            let _: () = msg_send![&panel, setWorksWhenModal: true];

            // IMPORTANT: Disable hidesOnDeactivate so we control visibility ourselves
            panel.setHidesOnDeactivate(false);

            // Add shadow for depth
            panel.setHasShadow(true);

            panel
        };

        // Add NSVisualEffectView as a subview behind other content
        let content_view = unsafe {
            let content_view = panel.contentView().ok_or(CommandPaletteError::PanelCreationFailed)?;
            let bounds = content_view.bounds();

            // Make content view layer-backed
            content_view.setWantsLayer(true);

            let effect_view = NSVisualEffectView::new(mtm);
            effect_view.setFrame(bounds);

            effect_view.setMaterial(NSVisualEffectMaterial::Menu);
            effect_view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            effect_view.setState(NSVisualEffectState::Active);
            // Autoresizing: NSViewWidthSizable (2) | NSViewHeightSizable (16) = 18
            effect_view.setAutoresizingMask(unsafe { std::mem::transmute(18u64) });

            // Add as subview positioned below other views
            content_view.addSubview_positioned_relativeTo(
                &effect_view,
                objc2_app_kit::NSWindowOrderingMode::Below,
                None,
            );

            content_view
        };

        // Create header with app name
        unsafe {
            let header_frame = NSRect::new(
                NSPoint::new(20.0, panel_height - header_height + 8.0),
                NSSize::new(panel_width - 40.0, 28.0),
            );

            let label = NSTextField::new(mtm);
            label.setFrame(header_frame);

            let title = NSString::from_str("Cleo");
            label.setStringValue(&title);
            label.setBezeled(false);
            label.setDrawsBackground(false);
            label.setEditable(false);
            label.setSelectable(false);

            // Semibold system font
            let font = NSFont::systemFontOfSize_weight(14.0, FONT_WEIGHT_SEMIBOLD);
            label.setFont(Some(&font));
            let color = NSColor::secondaryLabelColor();
            label.setTextColor(Some(&color));

            content_view.addSubview(&label);

            // Add subtle separator line
            let separator_frame = NSRect::new(
                NSPoint::new(12.0, panel_height - header_height),
                NSSize::new(panel_width - 24.0, 1.0),
            );
            let separator = NSBox::new(mtm);
            separator.setFrame(separator_frame);
            separator.setBoxType(NSBoxType::Separator);
            content_view.addSubview(&separator);
        }

        // Create command rows
        let mut command_labels = Vec::new();
        let mut icon_views = Vec::new();
        let mut selection_views = Vec::new();

        for (i, cmd) in commands.iter().enumerate() {
            let row_y =
                panel_height - header_height - content_padding - ((i + 1) as f64 * row_height);
            let center_y = row_y + (row_height / 2.0);

            // Selection/hover background - rounded rect with larger radius
            let selection_frame = NSRect::new(
                NSPoint::new(content_padding + 6.0, row_y + 4.0),
                NSSize::new(panel_width - content_padding * 2.0 - 12.0, row_height - 8.0),
            );

            let selection_view = unsafe {
                let view = NSView::new(mtm);
                view.setFrame(selection_frame);
                view.setWantsLayer(true);
                if let Some(layer) = view.layer() {
                    layer.setCornerRadius(8.0);
                    let cg = cg_clear_color();
                    layer.setBackgroundColor(Some(&cg));
                }
                content_view.addSubview(&view);
                view
            };
            selection_views.push(selection_view);

            // Icon - centered vertically with nice size
            let icon_size: f64 = 24.0;
            let icon_frame = NSRect::new(
                NSPoint::new(content_padding + 20.0, center_y - icon_size / 2.0),
                NSSize::new(icon_size, icon_size),
            );
            let image_view = unsafe {
                let image_view = NSImageView::new(mtm);
                image_view.setFrame(icon_frame);

                let icon_name = NSString::from_str(cmd.icon_name());
                let config = NSImageSymbolConfiguration::configurationWithPointSize_weight_scale(
                    18.0,
                    FONT_WEIGHT_MEDIUM,
                    NSImageSymbolScale::Medium,
                );
                if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &icon_name,
                    None,
                ) {
                    if let Some(configured) = image.imageWithSymbolConfiguration(&config) {
                        image_view.setImage(Some(&configured));
                    }
                    let color = NSColor::secondaryLabelColor();
                    image_view.setContentTintColor(Some(&color));
                }

                content_view.addSubview(&image_view);
                image_view
            };
            icon_views.push(image_view);

            // Command label - centered vertically, medium weight
            let label_height: f64 = 20.0;
            let label_frame = NSRect::new(
                NSPoint::new(content_padding + 56.0, center_y - label_height / 2.0),
                NSSize::new(panel_width - 180.0, label_height),
            );
            let label = unsafe {
                let label = NSTextField::new(mtm);
                label.setFrame(label_frame);

                let initial_state = PaletteState {
                    auto_capture_enabled: true,
                    recording: false,
                    current_app_name: None,
                    current_app_banned: false,
                };
                let text = NSString::from_str(&cmd.label(&initial_state));
                label.setStringValue(&text);
                label.setBezeled(false);
                label.setDrawsBackground(false);
                label.setEditable(false);
                label.setSelectable(false);

                // Medium weight for commands
                let font = NSFont::systemFontOfSize_weight(14.0, FONT_WEIGHT_REGULAR);
                label.setFont(Some(&font));
                let color = NSColor::labelColor();
                label.setTextColor(Some(&color));

                content_view.addSubview(&label);
                label
            };
            command_labels.push(label);

            // Keyboard shortcut badge - pill-shaped background
            let badge_width: f64 = 26.0;
            let badge_height: f64 = 22.0;
            let badge_x = panel_width - content_padding - 24.0 - badge_width;
            let badge_y = center_y - badge_height / 2.0;

            unsafe {
                // Badge background - subtle rounded rect
                let badge_frame = NSRect::new(
                    NSPoint::new(badge_x, badge_y),
                    NSSize::new(badge_width, badge_height),
                );
                let badge_bg = NSView::new(mtm);
                badge_bg.setFrame(badge_frame);
                badge_bg.setWantsLayer(true);
                if let Some(layer) = badge_bg.layer() {
                    layer.setCornerRadius(5.0);
                    // Semi-transparent background for keybind badges
                    let cg = cg_tertiary_label_color();
                    layer.setBackgroundColor(Some(&cg));
                }

                content_view.addSubview(&badge_bg);

                // Badge text
                let text_frame = NSRect::new(
                    NSPoint::new(badge_x, badge_y + 2.0),
                    NSSize::new(badge_width, badge_height - 4.0),
                );
                let hint = NSTextField::new(mtm);
                hint.setFrame(text_frame);

                let text = NSString::from_str(cmd.keybind());
                hint.setStringValue(&text);
                hint.setBezeled(false);
                hint.setDrawsBackground(false);
                hint.setEditable(false);
                hint.setSelectable(false);
                hint.setAlignment(objc2_app_kit::NSTextAlignment::Center);

                // Slightly bolder monospace font
                let font = NSFont::monospacedSystemFontOfSize_weight(12.0, FONT_WEIGHT_SEMIBOLD);
                hint.setFont(Some(&font));
                let color = NSColor::tertiaryLabelColor();
                hint.setTextColor(Some(&color));

                content_view.addSubview(&hint);
            }
        }

        let palette = Self {
            panel,
            command_labels: RefCell::new(command_labels),
            icon_views: RefCell::new(icon_views),
            selection_views: RefCell::new(selection_views),
            local_monitor: RefCell::new(None),
            _monitor_block: RefCell::new(None),
            visible: Cell::new(false),
            selected_index: Cell::new(0),
            auto_capture_enabled: Cell::new(true),
            recording: Cell::new(false),
            current_app_name: RefCell::new(None),
            current_app_banned: Cell::new(false),
        };

        palette.update_selection();

        Ok(palette)
    }

    /// Show the command palette
    pub fn show(&self) {
        if self.visible.get() {
            return;
        }

        // Clean up any stale local monitor
        self.uninstall_local_monitor();

        unsafe {
            // Center on screen (in case resolution changed)
            self.panel.center();

            // With NonactivatingPanel style, we don't activate the app.
            self.panel.makeKeyAndOrderFront(None);
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
            self.panel.orderOut(None);
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
    pub fn panel_ptr(&self) -> &NSPanel {
        &self.panel
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
            KEY_B => Some(PaletteCommand::ToggleBanApp),
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

        let block = RcBlock::new(move |event: *mut AnyObject| -> *mut AnyObject {
            let key_code: u16 = unsafe { msg_send![event, keyCode] };
            on_key(key_code);
            // Return nil to consume the event
            std::ptr::null_mut()
        });

        let monitor: *mut AnyObject = unsafe {
            msg_send![
                NSEvent::class(),
                addLocalMonitorForEventsMatchingMask: NSEventMask::KeyDown,
                handler: &*block
            ]
        };

        if !monitor.is_null() {
            self.local_monitor
                .replace(Some(unsafe { Retained::retain(monitor).unwrap() }));
            self._monitor_block.replace(Some(block));
        }
    }

    /// Remove local keyboard monitor
    pub fn uninstall_local_monitor(&self) {
        if let Some(monitor) = self.local_monitor.borrow_mut().take() {
            unsafe {
                let _: () = msg_send![NSEvent::class(), removeMonitor: &*monitor];
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
            if let Some(layer) = view.layer() {
                if is_selected {
                    // Use system accent color (orange) for selection
                    let cg = cg_accent_color();
                    layer.setBackgroundColor(Some(&cg));
                } else {
                    let cg = cg_clear_color();
                    layer.setBackgroundColor(Some(&cg));
                }
            }

            // Update icon color based on selection - white on accent, gray otherwise
            if let Some(icon) = icons.get(i) {
                let color = if is_selected {
                    NSColor::whiteColor()
                } else {
                    NSColor::secondaryLabelColor()
                };
                icon.setContentTintColor(Some(&color));
            }
        }

        // Update label colors for selected row - white on accent, normal otherwise
        let labels = self.command_labels.borrow();
        for (i, label) in labels.iter().enumerate() {
            let is_selected = i == selected;
            let color = if is_selected {
                NSColor::whiteColor()
            } else {
                NSColor::labelColor()
            };
            label.setTextColor(Some(&color));
        }
    }

    fn update_labels(&self) {
        let state = PaletteState {
            auto_capture_enabled: self.auto_capture_enabled.get(),
            recording: self.recording.get(),
            current_app_name: self.current_app_name.borrow().clone(),
            current_app_banned: self.current_app_banned.get(),
        };
        let labels = self.command_labels.borrow();

        for (i, cmd) in PaletteCommand::all().iter().enumerate() {
            if let Some(label) = labels.get(i) {
                let text = cmd.label(&state);
                let ns_text = NSString::from_str(&text);
                label.setStringValue(&ns_text);
            }
        }
    }

    /// Update the current app state for the ban toggle
    pub fn set_current_app(&self, app_name: Option<String>, is_banned: bool) {
        *self.current_app_name.borrow_mut() = app_name;
        self.current_app_banned.set(is_banned);
        self.update_labels();
    }
}

impl Drop for CommandPalette {
    fn drop(&mut self) {
        self.uninstall_local_monitor();
        unsafe {
            self.panel.close();
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
