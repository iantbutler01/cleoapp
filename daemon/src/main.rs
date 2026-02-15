#![allow(unused_doc_comments)]
#![allow(unused_unsafe)]
#![allow(unsafe_op_in_unsafe_fn)]

mod accessibility;
mod api;
mod app;
mod banned_apps_window;
mod command_palette;
mod content_filter;
mod idle;
mod interval;
mod keyboard_tracker;
mod logging;
mod mouse_tracker;
mod workspace_tracker;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::env;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
enum BurstActionKind {
    AppSwitch,
    Click,
    Keypress,
}

#[derive(Clone, Copy)]
struct BurstAction {
    time: Instant,
    kind: BurstActionKind,
}

use chrono::{Local, Utc};
use log::{debug, error, info, warn};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadOnly, sel};
use objc2_app_kit::{NSAlert, NSAlertStyle, NSApplication, NSMenu, NSMenuItem, NSTextField};
use objc2_foundation::{MainThreadMarker, NSString};
use png::{BitDepth, ColorType, Encoder, EncodingError};
use screencapturekit::error::SCError;
use screencapturekit::prelude::*;
use screencapturekit::recording_output::{
    SCRecordingOutput, SCRecordingOutputCodec, SCRecordingOutputConfiguration,
    SCRecordingOutputFileType,
};
use screencapturekit::screenshot_manager::SCScreenshotManager;
use serde::{Deserialize, Serialize};
use url::Url;

use image_hasher::{HashAlg, HasherConfig, ImageHash};

use crate::accessibility::{ActiveWindowInfo, check_accessibility_trusted};
use crate::api::{ActivityEntry, ActivityEvent, ApiClient, ApiError, ImageFormat, VideoFormat};
use crate::app::{
    App, MenuBuilder, MenuItemHandle, StatusItem, TerminateReply,
    reply_to_application_should_terminate, terminate,
};
use crate::banned_apps_window::BannedAppsWindow;
use crate::command_palette::{CommandPalette, HotkeyTracker, PaletteCommand};
use crate::content_filter::{ContentFilter, NoOpFilter, NsfwFilter};
use crate::interval::current_interval_id;
use crate::keyboard_tracker::KeyboardTracker;
use crate::mouse_tracker::MouseTracker;
use crate::workspace_tracker::WorkspaceTracker;

const API_BASE_ENV: &str = "CLEO_CAPTURE_API_URL";
const DEFAULT_API_BASE: &str = "http://localhost:3000";
const SCREENSHOT_INTERVAL_SECS: u64 = 5;
const BURST_WINDOW_SECS: u64 = 5;
const BURST_THRESHOLD_WITH_SWITCH: usize = 1; // App switch alone triggers recording
const BURST_THRESHOLD_ACTIONS_ONLY: usize = 5; // Actions without app switch need higher threshold
const AUTO_RECORDING_TAIL_SECS: u64 = 30; // Stop recording after 30s of no activity
const MAX_RECORDING_DURATION_SECS: u64 = 5 * 60; // Hard cap at 5 minutes per recording
const TASK_SLEEP_CHUNK_MS: u64 = 100;
const ACTIVITY_FLUSH_INTERVAL_SECS: u64 = 30;
const UPLOAD_BATCH_INTERVAL_SECS: u64 = 60; // Batch classify and upload every 60 seconds
const BATCH_SIZE: usize = 30; // Max unique images per batch for classification
const IDLE_THRESHOLD_SECS: f64 = 60.0; // Skip screenshots if idle for 60+ seconds
const PHASH_DISTANCE_THRESHOLD: u32 = 10; // Max hamming distance to consider images similar (0 = identical)
const LIMITS_REFRESH_INTERVAL_SECS: u64 = 5 * 60; // Refresh recording limits every 5 minutes
const PENDING_SCREENSHOTS_DIR: &str = ".cleo/captures/screenshots";
const PENDING_RECORDINGS_DIR: &str = ".cleo/captures/recordings";

#[derive(Debug, Deserialize, Serialize)]
struct CleoConfig {
    api_token: String,
    /// API base URL (e.g. "https://cleo.example.com/api"). Falls back to
    /// CLEO_CAPTURE_API_URL env var, then http://localhost:3000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_url: Option<String>,
    #[serde(default)]
    privacy: PrivacySettings,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
struct PrivacySettings {
    /// Apps that should never be captured (by bundle ID or name)
    #[serde(default)]
    blocked_apps: Vec<String>,
    /// Window title patterns that should never be captured (glob-style, case-insensitive)
    #[serde(default)]
    blocked_window_patterns: Vec<String>,
    /// Whether to detect and blur secrets in captures
    #[serde(default)]
    secret_detection_enabled: bool,
    /// Apps the user has explicitly added to the ban list (user-curated, not auto-tracked).
    /// This persists even after unbanning so users can easily re-ban apps.
    #[serde(default)]
    known_apps: Vec<String>,
}

impl PrivacySettings {
    /// Check if capture should be blocked for the given app/window
    fn should_block(&self, app_name: &str, bundle_id: &str, window_title: &str) -> bool {
        // Check blocked apps by name or bundle ID (case-insensitive)
        let app_lower = app_name.to_lowercase();
        let bundle_lower = bundle_id.to_lowercase();
        for blocked in &self.blocked_apps {
            let blocked_lower = blocked.to_lowercase();
            if app_lower == blocked_lower || bundle_lower == blocked_lower {
                return true;
            }
        }

        // Check blocked window patterns against window title (glob-style)
        let title_lower = window_title.to_lowercase();
        for pattern in &self.blocked_window_patterns {
            if Self::glob_match(&pattern.to_lowercase(), &title_lower) {
                return true;
            }
        }

        false
    }

    /// Simple glob matching (only supports * wildcard)
    fn glob_match(pattern: &str, text: &str) -> bool {
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.len() == 1 {
            return pattern == text;
        }

        let mut pos = 0;
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            if let Some(found) = text[pos..].find(part) {
                if i == 0 && found != 0 {
                    return false; // First part must match start
                }
                pos += found + part.len();
            } else {
                return false;
            }
        }

        // If pattern ends with *, we're done; otherwise text must end here
        parts
            .last()
            .map_or(true, |p| p.is_empty() || pos == text.len())
    }
}

/// Message type for dispatching events to main thread
#[derive(Copy, Clone)]
enum AppMessage {
    ToggleRecording,
    TakeScreenshot,
    MouseClick,
    Keypress,
    AutoStopRecording,
    MaxDurationReached,
    RefreshLimits,
    FlushActivity,
    SetApiToken,
    PollHotkey,
    PaletteKey { key_code: u16 },
    ManageBannedApps,
}

/// Dispatch a message to the main thread using GCD
fn dispatch_main(message: AppMessage) {
    if MainThreadMarker::new().is_some() {
        // Already on main thread, process directly
        DAEMON.with(|d| {
            if let Some(ref daemon) = *d.borrow() {
                daemon.on_message(message);
            }
        });
    } else {
        // Dispatch to main queue
        dispatch2::Queue::main().exec_async(move || {
            DAEMON.with(|d| {
                if let Some(ref daemon) = *d.borrow() {
                    daemon.on_message(message);
                }
            });
        });
    }
}

/// Dispatch a ban toggle action to the main thread
fn dispatch_main_toggle_ban(app_name: String, should_ban: bool) {
    let action = move || {
        DAEMON.with(|d| {
            if let Some(ref daemon) = *d.borrow() {
                daemon.set_app_banned(&app_name, should_ban);
            }
        });
    };

    if MainThreadMarker::new().is_some() {
        action();
    } else {
        dispatch2::Queue::main().exec_async(action);
    }
}

thread_local! {
    static DAEMON: RefCell<Option<CleoDaemon>> = const { RefCell::new(None) };
}

fn main() {
    logging::init();

    // Get main thread marker
    let mtm = MainThreadMarker::new().expect("Must run on main thread");

    // Create the app
    let app = App::new(mtm);

    // Set up callbacks
    app.on_did_finish_launching({
        move || {
            info!("Launching Cleo Daemon");

            // Request accessibility permissions (will prompt user if not already granted)
            if !check_accessibility_trusted(true) {
                warn!("Accessibility permissions not granted - activity tracking will be limited");
            } else {
                info!("Accessibility permissions granted");
            }

            DAEMON.with(|d| {
                let mut daemon = CleoDaemon::new();
                daemon.initialize(mtm);
                d.replace(Some(daemon));
            });
            info!("Cleo Daemon started");
        }
    });

    app.on_should_terminate({
        move || {
            let (api, pending) = DAEMON.with(|d| {
                if let Some(ref daemon) = *d.borrow() {
                    let pending = daemon.take_activity_events();
                    let api = daemon.api_client().ok();
                    (api, pending)
                } else {
                    (None, Vec::new())
                }
            });

            let Some(api) = api else {
                return TerminateReply::Now;
            };
            if pending.is_empty() {
                return TerminateReply::Now;
            }

            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let result = api.upload_activity(&pending);
                let _ = tx.send(result.is_ok());
            });

            thread::spawn(move || {
                let _ = rx.recv_timeout(Duration::from_secs(2));
                dispatch2::DispatchQueue::main().exec_async(|| {
                    reply_to_application_should_terminate(true);
                });
            });

            TerminateReply::Later
        }
    });

    app.on_will_terminate({
        move || {
            DAEMON.with(|d| {
                if let Some(ref mut daemon) = *d.borrow_mut() {
                    daemon.shutdown();
                }
            });
        }
    });

    app.on_open_urls({
        move |urls| {
            DAEMON.with(|d| {
                if let Some(ref daemon) = *d.borrow() {
                    for url in urls {
                        let url_text = url.to_string();
                        if let Err(err) = daemon.handle_deep_link(url) {
                            error!("Failed to handle URL {url_text}: {err}");
                        }
                    }
                }
            });
        }
    });

    info!("Starting Cleo Daemon");

    // Run the application event loop (like cacao did)
    app.run();
}

struct CleoDaemon {
    status_item: RefCell<Option<StatusItem>>,
    menu_handles: RefCell<Option<MenuHandles>>,
    menu_targets: RefCell<Vec<Retained<AnyObject>>>,
    recorder: RefCell<Option<ScreenRecorder>>,
    logging_daemon: RefCell<Option<LoggingDaemon>>,
    batch_uploader: RefCell<Option<BatchUploader>>,
    api: RefCell<Option<ApiClient>>,
    tracker: RefCell<Option<WorkspaceTracker>>,
    mouse_tracker: RefCell<Option<MouseTracker>>,
    keyboard_tracker: RefCell<Option<KeyboardTracker>>,
    command_palette: RefCell<Option<CommandPalette>>,
    hotkey_tracker: RefCell<Option<HotkeyTracker>>,
    hotkey_poll_task: RefCell<Option<RepeatingTask>>,
    screenshot_task: RefCell<Option<RepeatingTask>>,
    activity_flush_task: RefCell<Option<RepeatingTask>>,
    auto_stop_task: RefCell<Option<DelayedTask>>,
    max_duration_task: RefCell<Option<DelayedTask>>,
    limits_refresh_task: RefCell<Option<RepeatingTask>>,
    activity_window: RefCell<VecDeque<BurstAction>>,
    manual_recording: Cell<bool>,
    auto_capture_enabled: Cell<bool>,
    activity_events: RefCell<Vec<ActivityEntry>>,
    recording_limits: RefCell<Option<api::RecordingLimits>>,
    privacy_settings: RefCell<PrivacySettings>,
    /// The currently focused app name (for ban toggle in command palette)
    current_app_name: RefCell<Option<String>>,
    /// Window for managing banned apps
    banned_apps_window: RefCell<Option<BannedAppsWindow>>,
}

impl CleoDaemon {
    fn new() -> Self {
        eprintln!("[DEBUG] CleoDaemon::new() starting");

        Self {
            status_item: RefCell::new(None),
            menu_handles: RefCell::new(None),
            menu_targets: RefCell::new(Vec::new()),
            recorder: RefCell::new(None),
            logging_daemon: RefCell::new(None),
            batch_uploader: RefCell::new(None),
            api: RefCell::new(None),
            tracker: RefCell::new(None),
            mouse_tracker: RefCell::new(None),
            keyboard_tracker: RefCell::new(None),
            command_palette: RefCell::new(None),
            hotkey_tracker: RefCell::new(None),
            hotkey_poll_task: RefCell::new(None),
            screenshot_task: RefCell::new(None),
            activity_flush_task: RefCell::new(None),
            auto_stop_task: RefCell::new(None),
            max_duration_task: RefCell::new(None),
            limits_refresh_task: RefCell::new(None),
            activity_window: RefCell::new(VecDeque::new()),
            manual_recording: Cell::new(false),
            auto_capture_enabled: Cell::new(true),
            activity_events: RefCell::new(Vec::new()),
            recording_limits: RefCell::new(None),
            privacy_settings: RefCell::new(PrivacySettings::default()),
            current_app_name: RefCell::new(None),
            banned_apps_window: RefCell::new(None),
        }
    }

    fn initialize(&mut self, mtm: MainThreadMarker) {
        let (menu, handles, targets) = build_status_menu(mtm);
        handles.set_recording(false);
        self.menu_handles.replace(Some(handles));
        self.menu_targets.replace(targets);

        let status_item = StatusItem::new(
            mtm,
            "menubar-icon",
            "message.fill",
            "Cleo Screen Recorder",
            menu,
        );
        self.status_item.replace(Some(status_item));

        self.logging_daemon.replace(Some(LoggingDaemon::start()));
        self.batch_uploader.replace(Some(BatchUploader::start()));
        self.load_privacy_settings();
        self.ensure_api_client();
        self.start_activity_tracking();
        self.start_mouse_tracking();
        self.start_keyboard_tracking();
        self.start_screenshot_timer();
        self.start_activity_flush_timer();
        self.start_limits_refresh_timer();
        self.start_command_palette();
    }

    fn shutdown(&mut self) {
        // Try to save any in-progress recording before shutting down
        self.stop_recording();

        self.stop_logging_daemon();
        self.stop_batch_uploader();
        self.stop_tracker();
        self.stop_mouse_tracking();
        self.stop_keyboard_tracking();
        self.stop_command_palette();
        self.stop_screenshot_timer();
        self.stop_activity_flush_timer();
        self.stop_limits_refresh_timer();
        self.flush_activity_events_async();
    }

    fn on_message(&self, message: AppMessage) {
        match message {
            AppMessage::ToggleRecording => {
                if self.recorder.borrow().is_some() {
                    self.manual_recording.set(false);
                    self.stop_recording();
                } else {
                    self.start_recording();
                    if self.recorder.borrow().is_some() {
                        self.manual_recording.set(true);
                        self.cancel_auto_stop();
                    }
                }
            }
            AppMessage::TakeScreenshot => self.take_screenshot(),
            AppMessage::MouseClick => self.record_mouse_click(),
            AppMessage::Keypress => self.record_keypress(),
            AppMessage::AutoStopRecording => self.stop_recording_if_auto(),
            AppMessage::MaxDurationReached => self.stop_recording_max_duration(),
            AppMessage::RefreshLimits => self.fetch_recording_limits(),
            AppMessage::FlushActivity => self.flush_activity_events(),
            AppMessage::SetApiToken => self.show_api_token_dialog(),
            AppMessage::PollHotkey => self.poll_hotkey(),
            AppMessage::PaletteKey { key_code } => self.handle_palette_key(key_code),
            AppMessage::ManageBannedApps => self.show_banned_apps_window(),
        }
    }

    fn start_recording(&self) {
        if self.recorder.borrow().is_some() {
            warn!("Recording already in progress");
            return;
        }

        let privacy = self.privacy_settings.borrow().clone();
        match ScreenRecorder::start_with_exclusions(&privacy) {
            Ok(recorder) => {
                info!(
                    "Recording started, spooling to {}",
                    recorder.file_path().display()
                );
                self.update_menu_state(true);
                self.recorder.replace(Some(recorder));
                self.schedule_max_duration_stop();
            }
            Err(err) => error!("Failed to start recording: {err}"),
        }
    }

    fn stop_recording(&self) {
        self.manual_recording.set(false);
        if let Some(recorder) = self.recorder.borrow_mut().take() {
            match recorder.stop() {
                Ok(()) => {
                    eprintln!("[recording] Recording saved to pending folder");
                    info!("Recording saved to pending folder");
                }
                Err(err) => {
                    eprintln!("[recording] Failed to stop recording: {err}");
                    error!("Failed to stop recording: {err}");
                }
            }
        }
        self.update_menu_state(false);
        self.cancel_auto_stop();
        self.cancel_max_duration_stop();
    }

    fn take_screenshot(&self) {
        if !self.auto_capture_enabled.get() {
            debug!("Skipping screenshot - auto capture disabled");
            return;
        }
        // Skip screenshot if user is idle
        if idle::is_idle(IDLE_THRESHOLD_SECS) {
            debug!(
                "Skipping screenshot - user idle for {:.0}s",
                idle::seconds_since_last_input()
            );
            return;
        }
        // Skip screenshot if current app is banned
        if let Some(ref app_name) = *self.current_app_name.borrow() {
            if self.is_app_banned(app_name) {
                debug!("Skipping screenshot - current app '{}' is banned", app_name);
                return;
            }
        }
        let privacy = self.privacy_settings.borrow().clone();
        if let Err(err) = capture_screenshot_with_exclusions(&privacy) {
            error!("Failed to capture screenshot: {err}");
        }
    }

    fn update_menu_state(&self, recording: bool) {
        if let Some(handles) = self.menu_handles.borrow().as_ref() {
            handles.set_recording(recording);
        }
    }

    fn stop_logging_daemon(&self) {
        if let Some(daemon) = self.logging_daemon.borrow_mut().take() {
            daemon.shutdown();
        }
    }

    fn stop_batch_uploader(&self) {
        if let Some(uploader) = self.batch_uploader.borrow_mut().take() {
            uploader.stop();
        }
    }

    fn start_activity_tracking(&self) {
        // Use thread_local DAEMON to call record_focus_event directly
        // The handler runs on the main thread via accessibility notifications
        let handler = move |info: ActiveWindowInfo| {
            DAEMON.with(|d| {
                if let Some(ref daemon) = *d.borrow() {
                    daemon.record_focus_event(info);
                }
            });
        };

        match WorkspaceTracker::start(handler) {
            Ok(tracker) => {
                info!("Workspace tracker started");
                self.tracker.replace(Some(tracker));
            }
            Err(err) => error!("Workspace tracker unavailable: {err}"),
        }
    }

    fn stop_tracker(&self) {
        self.tracker.borrow_mut().take();
    }

    fn record_focus_event(&self, info: ActiveWindowInfo) {
        // Store the current app name for the ban toggle feature
        *self.current_app_name.borrow_mut() = Some(info.app_name.clone());

        // Update the command palette if visible
        if let Some(ref palette) = *self.command_palette.borrow() {
            let is_banned = self.is_app_banned(&info.app_name);
            palette.set_current_app(Some(info.app_name.clone()), is_banned);
        }

        let event = ActivityEvent::foreground_switch(info.app_name, info.window_title);
        let interval_id = current_interval_id();
        let entry = ActivityEntry::new(Utc::now(), interval_id, event);
        self.activity_events.borrow_mut().push(entry);
        self.handle_activity_event(BurstActionKind::AppSwitch);
    }

    /// Check if an app is in the blocked list
    fn is_app_banned(&self, app_name: &str) -> bool {
        let settings = self.privacy_settings.borrow();
        let app_lower = app_name.to_lowercase();
        settings
            .blocked_apps
            .iter()
            .any(|blocked| blocked.to_lowercase() == app_lower)
    }

    /// Toggle the ban status of the currently focused app
    fn toggle_ban_current_app(&self) {
        let app_name = match self.current_app_name.borrow().clone() {
            Some(name) => name,
            None => {
                warn!("No current app to ban/unban");
                return;
            }
        };

        let is_banned = self.is_app_banned(&app_name);
        {
            let mut settings = self.privacy_settings.borrow_mut();
            if is_banned {
                // Remove from blocked list (case-insensitive match)
                let app_lower = app_name.to_lowercase();
                settings
                    .blocked_apps
                    .retain(|blocked| blocked.to_lowercase() != app_lower);
                info!("Unbanned app: {}", app_name);
            } else {
                // Add to blocked list
                settings.blocked_apps.push(app_name.clone());
                info!("Banned app: {}", app_name);
            }

            // Always add to known_apps when banning (user-curated list)
            if !is_banned {
                let app_lower = app_name.to_lowercase();
                let already_known = settings
                    .known_apps
                    .iter()
                    .any(|k| k.to_lowercase() == app_lower);
                if !already_known {
                    settings.known_apps.push(app_name.clone());
                }
            }
        }

        // Save the updated settings
        let settings = self.privacy_settings.borrow().clone();
        if let Err(err) = save_privacy_settings(&settings) {
            error!("Failed to save privacy settings: {}", err);
        }

        // Update the command palette
        if let Some(ref palette) = *self.command_palette.borrow() {
            palette.set_current_app(Some(app_name), !is_banned);
        }
    }

    /// Set the ban status of an app by name (used by banned apps window)
    fn set_app_banned(&self, app_name: &str, should_ban: bool) {
        let is_currently_banned = self.is_app_banned(app_name);

        // No change needed
        if is_currently_banned == should_ban {
            return;
        }

        {
            let mut settings = self.privacy_settings.borrow_mut();
            if should_ban {
                // Add to blocked list
                settings.blocked_apps.push(app_name.to_string());
                info!("Banned app: {}", app_name);
            } else {
                // Remove from blocked list (case-insensitive match)
                let app_lower = app_name.to_lowercase();
                settings
                    .blocked_apps
                    .retain(|blocked| blocked.to_lowercase() != app_lower);
                info!("Unbanned app: {}", app_name);
            }
        }

        // Save the updated settings
        let settings = self.privacy_settings.borrow().clone();
        if let Err(err) = save_privacy_settings(&settings) {
            error!("Failed to save privacy settings: {}", err);
        }

        // Update command palette if this is the current app
        if let Some(current) = self.current_app_name.borrow().as_ref() {
            if current.to_lowercase() == app_name.to_lowercase() {
                if let Some(ref palette) = *self.command_palette.borrow() {
                    palette.set_current_app(Some(current.clone()), should_ban);
                }
            }
        }
    }

    fn start_mouse_tracking(&self) {
        if self.mouse_tracker.borrow().is_some() {
            return;
        }
        let handler = || dispatch_main(AppMessage::MouseClick);
        match MouseTracker::start(handler) {
            Ok(tracker) => {
                info!("Mouse tracker started");
                self.mouse_tracker.replace(Some(tracker));
            }
            Err(err) => error!("Mouse tracker unavailable: {err}"),
        }
    }

    fn stop_mouse_tracking(&self) {
        self.mouse_tracker.borrow_mut().take();
    }

    fn record_mouse_click(&self) {
        info!("Mouse click recorded");
        let event = ActivityEvent::mouse_click();
        let interval_id = current_interval_id();
        let entry = ActivityEntry::new(Utc::now(), interval_id, event);
        self.activity_events.borrow_mut().push(entry);
        self.handle_activity_event(BurstActionKind::Click);
    }

    fn start_keyboard_tracking(&self) {
        if self.keyboard_tracker.borrow().is_some() {
            return;
        }
        let handler = || dispatch_main(AppMessage::Keypress);
        match KeyboardTracker::start(handler) {
            Ok(tracker) => {
                info!("Keyboard tracker started");
                self.keyboard_tracker.replace(Some(tracker));
            }
            Err(err) => error!("Keyboard tracker unavailable: {err}"),
        }
    }

    fn stop_keyboard_tracking(&self) {
        self.keyboard_tracker.borrow_mut().take();
    }

    fn start_command_palette(&self) {
        // Create the command palette
        match CommandPalette::new() {
            Ok(palette) => {
                info!("Command palette created");
                self.command_palette.replace(Some(palette));
            }
            Err(err) => {
                error!("Failed to create command palette: {err}");
                return;
            }
        }

        // Create global hotkey tracker (Cmd+Shift+C)
        // Must be created on main thread for macOS
        match HotkeyTracker::new() {
            Ok(tracker) => {
                info!("Hotkey tracker created (Cmd+Shift+C to open palette)");
                self.hotkey_tracker.replace(Some(tracker));
            }
            Err(err) => {
                error!("Hotkey tracker unavailable: {err}");
                return;
            }
        }

        // Start polling timer for hotkey events (poll every 50ms for responsiveness)
        let task = RepeatingTask::start(Duration::from_millis(50), || {
            dispatch_main(AppMessage::PollHotkey);
        });
        self.hotkey_poll_task.replace(Some(task));
    }

    fn stop_command_palette(&self) {
        self.hotkey_poll_task.borrow_mut().take();
        self.hotkey_tracker.borrow_mut().take();
        self.command_palette.borrow_mut().take();
    }

    fn poll_hotkey(&self) {
        let should_mark_deactivated = {
            let palette_ref = self.command_palette.borrow();
            palette_ref
                .as_ref()
                .is_some_and(|palette| palette.is_visible() && !palette.panel_ptr().isKeyWindow())
        };

        if should_mark_deactivated {
            if let Some(palette) = self.command_palette.borrow().as_ref() {
                palette.hide();
            }
        }

        let should_toggle = {
            let tracker_ref = self.hotkey_tracker.borrow();
            tracker_ref.as_ref().is_some_and(|tracker| tracker.poll())
        };

        if should_toggle {
            if let Some(palette) = self.command_palette.borrow().as_ref() {
                palette.toggle();
            } else {
                self.show_command_palette();
            }
        }
    }

    fn show_command_palette(&self) {
        if let Some(palette) = self.command_palette.borrow().as_ref() {
            // Update state before showing
            let auto_capture_enabled = self.auto_capture_enabled.get();
            let recording = self.recorder.borrow().is_some();
            palette.update_state(auto_capture_enabled, recording);

            // Show first (this cleans up any stale monitor)
            palette.show();

            // Install local keyboard monitor AFTER show() since show() uninstalls stale monitors
            palette.install_local_monitor(|key_code| {
                dispatch_main(AppMessage::PaletteKey { key_code });
            });

            info!("Command palette shown");
        }
    }

    fn show_banned_apps_window(&self) {
        let mtm = match MainThreadMarker::new() {
            Some(m) => m,
            None => {
                error!("show_banned_apps_window must be called on main thread");
                return;
            }
        };

        // Create window if it doesn't exist
        if self.banned_apps_window.borrow().is_none() {
            match BannedAppsWindow::new(mtm) {
                Ok(window) => {
                    self.banned_apps_window.replace(Some(window));
                }
                Err(e) => {
                    error!("Failed to create banned apps window: {:?}", e);
                    return;
                }
            }
        }

        // Update the window with current apps and show it
        if let Some(ref window) = *self.banned_apps_window.borrow() {
            let settings = self.privacy_settings.borrow();

            // Build list of (app_name, is_banned) from known_apps
            let apps: Vec<(String, bool)> = settings
                .known_apps
                .iter()
                .map(|app| {
                    let is_banned = settings
                        .blocked_apps
                        .iter()
                        .any(|b| b.to_lowercase() == app.to_lowercase());
                    (app.clone(), is_banned)
                })
                .collect();

            // Clone what we need for the callback
            let on_toggle = move |app_name: String, is_banned: bool| {
                dispatch_main_toggle_ban(app_name, is_banned);
            };

            window.update_apps(&apps, on_toggle);
            window.show();

            // Bring app to front
            unsafe {
                let app = NSApplication::sharedApplication(mtm);
                app.activateIgnoringOtherApps(true);
            }

            info!("Banned apps window shown");
        }
    }

    fn toggle_capture_mode(&self) {
        let new_state = !self.auto_capture_enabled.get();
        self.auto_capture_enabled.set(new_state);

        // Update palette display
        if let Some(palette) = self.command_palette.borrow().as_ref() {
            let recording = self.recorder.borrow().is_some();
            palette.update_state(new_state, recording);
        }

        if new_state {
            info!("Auto capture enabled");
        } else {
            info!("Auto capture disabled");
        }
    }

    fn handle_palette_key(&self, key_code: u16) {
        let command = {
            if let Some(palette) = self.command_palette.borrow().as_ref() {
                palette.handle_key(key_code)
            } else {
                return;
            }
        };

        if let Some(cmd) = command {
            match cmd {
                PaletteCommand::ToggleCapture => {
                    self.toggle_capture_mode();
                }
                PaletteCommand::ToggleRecording => {
                    // Dispatch to existing toggle recording handler
                    if self.recorder.borrow().is_some() {
                        self.manual_recording.set(false);
                        self.stop_recording();
                    } else {
                        self.start_recording();
                        if self.recorder.borrow().is_some() {
                            self.manual_recording.set(true);
                            self.cancel_auto_stop();
                        }
                    }
                    // Update palette display
                    if let Some(palette) = self.command_palette.borrow().as_ref() {
                        let auto_capture_enabled = self.auto_capture_enabled.get();
                        let recording = self.recorder.borrow().is_some();
                        palette.update_state(auto_capture_enabled, recording);
                    }
                }
                PaletteCommand::TakeScreenshot => {
                    // Take screenshot directly (bypass capture_enabled check for manual trigger)
                    let privacy = self.privacy_settings.borrow().clone();
                    if let Err(err) = capture_screenshot_with_exclusions(&privacy) {
                        error!("Failed to capture screenshot: {err}");
                    }
                }
                PaletteCommand::ToggleBanApp => {
                    self.toggle_ban_current_app();
                }
            }
        }
    }

    fn record_keypress(&self) {
        // Just track activity for recording triggers - don't log what was typed
        self.handle_activity_event(BurstActionKind::Keypress);
    }

    fn flush_activity_events(&self) {
        let pending = {
            let buffer = self.activity_events.borrow();
            if buffer.is_empty() {
                return;
            }
            buffer.clone()
        };

        let api = match self.api_client() {
            Ok(client) => client,
            Err(err) => {
                error!("Cannot upload activity events: {err}");
                return;
            }
        };

        if let Err(err) = api.upload_activity(&pending) {
            error!("Failed to upload activity events: {err}");
            return;
        }

        let mut buffer = self.activity_events.borrow_mut();
        let sent = pending.len().min(buffer.len());
        buffer.drain(0..sent);
        info!(target: "activity", "Flushed {sent} activity event(s) to API");
    }

    fn take_activity_events(&self) -> Vec<ActivityEntry> {
        let mut buffer = self.activity_events.borrow_mut();
        if buffer.is_empty() {
            return Vec::new();
        }
        buffer.drain(..).collect()
    }

    fn flush_activity_events_async(&self) {
        let pending = {
            let buffer = self.activity_events.borrow();
            if buffer.is_empty() {
                return;
            }
            buffer.clone()
        };

        let api = match self.api_client() {
            Ok(client) => client,
            Err(err) => {
                error!("Cannot upload activity events during shutdown: {err}");
                return;
            }
        };

        thread::spawn(move || {
            if let Err(err) = api.upload_activity(&pending) {
                error!("Failed to upload activity events during shutdown: {err}");
            }
        });
    }

    fn ensure_api_client(&self) {
        if self.api.borrow().is_some() {
            return;
        }
        match build_api_client() {
            Ok(client) => {
                info!("Using capture API at {}", client.base_url());
                self.api.replace(Some(client));
                self.fetch_recording_limits();
            }
            Err(err) => error!("Capture API unavailable: {err}"),
        }
    }

    fn api_client(&self) -> Result<ApiClient, CaptureError> {
        if self.api.borrow().is_none() {
            self.ensure_api_client();
        }
        self.api
            .borrow()
            .clone()
            .ok_or(CaptureError::ApiUnavailable)
    }

    fn start_screenshot_timer(&self) {
        if self.screenshot_task.borrow().is_some() {
            return;
        }
        let task = RepeatingTask::start(Duration::from_secs(SCREENSHOT_INTERVAL_SECS), || {
            dispatch_main(AppMessage::TakeScreenshot);
        });
        self.screenshot_task.replace(Some(task));
    }

    fn stop_screenshot_timer(&self) {
        self.screenshot_task.borrow_mut().take();
    }

    fn start_activity_flush_timer(&self) {
        if self.activity_flush_task.borrow().is_some() {
            return;
        }
        let task = RepeatingTask::start(Duration::from_secs(ACTIVITY_FLUSH_INTERVAL_SECS), || {
            dispatch_main(AppMessage::FlushActivity);
        });
        self.activity_flush_task.replace(Some(task));
    }

    fn stop_activity_flush_timer(&self) {
        self.activity_flush_task.borrow_mut().take();
    }

    fn start_limits_refresh_timer(&self) {
        if self.limits_refresh_task.borrow().is_some() {
            return;
        }
        let task = RepeatingTask::start(Duration::from_secs(LIMITS_REFRESH_INTERVAL_SECS), || {
            dispatch_main(AppMessage::RefreshLimits);
        });
        self.limits_refresh_task.replace(Some(task));
    }

    fn stop_limits_refresh_timer(&self) {
        self.limits_refresh_task.borrow_mut().take();
    }

    fn handle_activity_event(&self, kind: BurstActionKind) {
        // Skip activity tracking if current app is banned
        if let Some(ref app_name) = *self.current_app_name.borrow() {
            if self.is_app_banned(app_name) {
                return;
            }
        }

        self.track_activity_burst(kind);
        self.schedule_auto_stop();
    }

    fn track_activity_burst(&self, kind: BurstActionKind) {
        let mut window = self.activity_window.borrow_mut();
        let now = Instant::now();

        // First, clean out stale events older than the burst window
        let threshold = Duration::from_secs(BURST_WINDOW_SECS);
        while let Some(front) = window.front() {
            if now.duration_since(front.time) > threshold {
                window.pop_front();
            } else {
                break;
            }
        }

        // Add the new event
        window.push_back(BurstAction { time: now, kind });

        let app_switch_count = window
            .iter()
            .filter(|event| event.kind == BurstActionKind::AppSwitch)
            .count();
        let action_count = window
            .iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    BurstActionKind::Click | BurstActionKind::Keypress
                )
            })
            .count();
        let burst_triggered = app_switch_count >= BURST_THRESHOLD_WITH_SWITCH
            || action_count >= BURST_THRESHOLD_ACTIONS_ONLY;

        if burst_triggered && self.recorder.borrow().is_none() && self.auto_capture_enabled.get() {
            eprintln!(
                "[recording] Automatic recording triggered by activity burst ({} events in {}s window)",
                window.len(),
                BURST_WINDOW_SECS
            );
            self.start_recording();
        }
    }

    fn schedule_auto_stop(&self) {
        if self.manual_recording.get() || self.recorder.borrow().is_none() {
            self.cancel_auto_stop();
            return;
        }
        let mut slot = self.auto_stop_task.borrow_mut();
        let had_task = slot.is_some();
        if let Some(task) = slot.take() {
            task.cancel();
        }
        if !had_task {
            info!(
                "Automatic recording will stop in {} seconds without more activity",
                AUTO_RECORDING_TAIL_SECS
            );
        }
        let task = DelayedTask::schedule(Duration::from_secs(AUTO_RECORDING_TAIL_SECS), || {
            dispatch_main(AppMessage::AutoStopRecording);
        });
        slot.replace(task);
    }

    fn cancel_auto_stop(&self) {
        if let Some(task) = self.auto_stop_task.borrow_mut().take() {
            task.cancel();
        }
    }

    fn stop_recording_if_auto(&self) {
        if self.manual_recording.get() {
            return;
        }
        if self.recorder.borrow().is_some() {
            eprintln!("[recording] Stopping automatic recording after inactivity");
            info!("Stopping automatic recording after inactivity");
            self.stop_recording();
        }
    }

    fn stop_recording_max_duration(&self) {
        if self.recorder.borrow().is_some() {
            eprintln!("[recording] Stopping recording: max duration reached");
            info!("Stopping recording: max duration reached");
            self.manual_recording.set(false);
            self.stop_recording();
        }
    }

    fn schedule_max_duration_stop(&self) {
        // Cancel any existing max duration task
        if let Some(task) = self.max_duration_task.borrow_mut().take() {
            task.cancel();
        }

        // Get max duration from limits (fall back to constant if not fetched)
        let max_secs = self
            .recording_limits
            .borrow()
            .as_ref()
            .map(|l| l.max_recording_duration_secs)
            .unwrap_or(MAX_RECORDING_DURATION_SECS);

        info!("Max recording duration set to {}s", max_secs);

        let task = DelayedTask::schedule(Duration::from_secs(max_secs), || {
            dispatch_main(AppMessage::MaxDurationReached);
        });
        self.max_duration_task.borrow_mut().replace(task);
    }

    fn cancel_max_duration_stop(&self) {
        if let Some(task) = self.max_duration_task.borrow_mut().take() {
            task.cancel();
        }
    }

    fn fetch_recording_limits(&self) {
        if let Some(api) = self.api.borrow().as_ref() {
            match api.fetch_limits() {
                Ok(limits) => {
                    let storage_remaining = limits.storage_remaining();
                    let storage_exceeded = limits.storage_exceeded();
                    info!(
                        "Fetched recording limits: max_duration={}s, budget={}s, inactivity={}s, storage={}/{}, remaining={}B, exceeded={}",
                        limits.max_recording_duration_secs,
                        limits.recording_budget_secs,
                        limits.inactivity_timeout_secs,
                        limits.storage_used_bytes,
                        limits.storage_limit_bytes,
                        storage_remaining,
                        storage_exceeded
                    );
                    self.recording_limits.borrow_mut().replace(limits);
                }
                Err(err) => {
                    warn!("Failed to fetch recording limits: {}", err);
                }
            }
        }
    }

    fn load_privacy_settings(&self) {
        match load_privacy_settings() {
            Ok(settings) => {
                info!(
                    "Loaded privacy settings: {} blocked apps, {} blocked patterns",
                    settings.blocked_apps.len(),
                    settings.blocked_window_patterns.len()
                );
                self.privacy_settings.replace(settings);
            }
            Err(_) => {
                // Config might not exist yet, use defaults
                info!("Using default privacy settings");
            }
        }
    }

    fn handle_deep_link(&self, url: Url) -> Result<(), CaptureError> {
        if !url.scheme().eq_ignore_ascii_case("cleo") {
            warn!("Ignoring unsupported URL {}", url);
            return Ok(());
        }
        match CleoRoute::from_url(&url)? {
            CleoRoute::Login { api_key } => {
                info!("Received cleo://login callback");
                self.apply_api_token(api_key)
            }
        }
    }

    fn apply_api_token(&self, api_key: String) -> Result<(), CaptureError> {
        save_api_token(&api_key)?;
        let base = resolve_api_base();
        let client = ApiClient::new(base, Some(api_key)).map_err(CaptureError::from)?;
        self.api.replace(Some(client));
        info!("API token saved from login link");
        Ok(())
    }

    fn show_api_token_dialog(&self) {
        let Some(mtm) = MainThreadMarker::new() else {
            error!("show_api_token_dialog must be called on main thread");
            return;
        };

        unsafe {
            // Add Edit menu temporarily so Cmd+V paste works
            let app = NSApplication::sharedApplication(mtm);
            let main_menu = app.mainMenu();
            let had_menu = main_menu.is_some();

            // Create main menu if needed
            let menu_bar = if let Some(menu) = main_menu {
                menu
            } else {
                let m = NSMenu::new(mtm);
                app.setMainMenu(Some(&m));
                m
            };

            // Create Edit menu with standard items
            let edit_title = NSString::from_str("Edit");
            let edit_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &edit_title);

            // Add Paste item (Cmd+V)
            let paste_title = NSString::from_str("Paste");
            let paste_key = NSString::from_str("v");
            let paste_item = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &paste_title,
                Some(sel!(paste:)),
                &paste_key,
            );
            edit_menu.addItem(&paste_item);

            // Add Cut item (Cmd+X)
            let cut_title = NSString::from_str("Cut");
            let cut_key = NSString::from_str("x");
            let cut_item = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &cut_title,
                Some(sel!(cut:)),
                &cut_key,
            );
            edit_menu.addItem(&cut_item);

            // Add Copy item (Cmd+C)
            let copy_title = NSString::from_str("Copy");
            let copy_key = NSString::from_str("c");
            let copy_item = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &copy_title,
                Some(sel!(copy:)),
                &copy_key,
            );
            edit_menu.addItem(&copy_item);

            // Add Select All item (Cmd+A)
            let select_title = NSString::from_str("Select All");
            let select_key = NSString::from_str("a");
            let select_item = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &select_title,
                Some(sel!(selectAll:)),
                &select_key,
            );
            edit_menu.addItem(&select_item);

            // Create menu bar item for Edit menu
            let edit_menu_item = NSMenuItem::new(mtm);
            edit_menu_item.setSubmenu(Some(&edit_menu));
            menu_bar.addItem(&edit_menu_item);

            // Create the alert
            let alert = NSAlert::new(mtm);
            let title = NSString::from_str("Set API Token");
            let message = NSString::from_str(
                "Enter your Cleo API token.\n\nYou can get this from the web dashboard after logging in with Twitter.",
            );
            alert.setMessageText(&title);
            alert.setInformativeText(&message);

            // Set alert style to informational
            alert.setAlertStyle(NSAlertStyle::Informational);

            // Set a custom icon (key symbol for API token)
            let icon_name = NSString::from_str("key.fill");
            if let Some(icon) =
                objc2_app_kit::NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &icon_name, None,
                )
            {
                alert.setIcon(Some(&icon));
            }

            // Add OK and Cancel buttons
            let ok_title = NSString::from_str("Save");
            let cancel_title = NSString::from_str("Cancel");
            alert.addButtonWithTitle(&ok_title);
            alert.addButtonWithTitle(&cancel_title);

            // Create text input field with frame
            let frame = objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(0.0, 0.0),
                objc2_foundation::NSSize::new(400.0, 24.0),
            );
            let text_field = NSTextField::new(mtm);
            text_field.setFrame(frame);

            // Make it editable and selectable
            text_field.setEditable(true);
            text_field.setSelectable(true);

            // Load existing token as placeholder/default
            if let Ok(existing_token) = load_api_token() {
                let placeholder = NSString::from_str(&existing_token);
                text_field.setStringValue(&placeholder);
            } else {
                let placeholder = NSString::from_str("cleo_your_token_here");
                text_field.setPlaceholderString(Some(&placeholder));
            }

            alert.setAccessoryView(Some(&text_field));

            // Bring app to front so the dialog is visible
            app.activateIgnoringOtherApps(true);

            // Get the alert's window and make text field first responder for paste support
            let window = alert.window();
            window.makeFirstResponder(Some(&text_field));

            // Run the alert and get response
            let response = alert.runModal();

            // Clean up: remove the Edit menu we added
            menu_bar.removeItem(&edit_menu_item);
            if !had_menu {
                app.setMainMenu(None);
            }

            // NSAlertFirstButtonReturn = 1000
            if response == 1000 {
                let value = text_field.stringValue();
                let token = value.to_string();

                if !token.trim().is_empty() {
                    match self.apply_api_token(token) {
                        Ok(()) => {
                            info!("API token saved from dialog");
                            show_notification("Cleo", "API token saved successfully!");
                        }
                        Err(err) => {
                            error!("Failed to save API token: {err}");
                            show_notification("Cleo", &format!("Failed to save token: {err}"));
                        }
                    }
                }
            }
        }
    }
}

fn build_status_menu(
    mtm: MainThreadMarker,
) -> (Retained<NSMenu>, MenuHandles, Vec<Retained<AnyObject>>) {
    let builder = MenuBuilder::new(mtm, "");

    let (builder, record_handle) =
        builder.add_action_item_with_handle("Start Recording", "", || {
            dispatch_main(AppMessage::ToggleRecording);
        });

    let (menu, targets) = builder
        .add_action_item("Take Screenshot", "", || {
            dispatch_main(AppMessage::TakeScreenshot);
        })
        .add_separator()
        .add_action_item("Manage Banned Apps...", "", || {
            dispatch_main(AppMessage::ManageBannedApps);
        })
        .add_action_item("Set API Token...", "", || {
            dispatch_main(AppMessage::SetApiToken);
        })
        .add_separator()
        .add_action_item("Quit Cleo Recorder", "", || {
            terminate();
        })
        .build();

    (menu, MenuHandles::new(record_handle), targets)
}

fn build_api_client() -> Result<ApiClient, CaptureError> {
    let base = resolve_api_base();
    let auth_token = load_api_token()?;
    ApiClient::new(base, Some(auth_token)).map_err(CaptureError::from)
}

fn load_config() -> Result<CleoConfig, CaptureError> {
    let path = cleo_config_path()?;
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return Err(CaptureError::Config(format!(
                "Missing Cleo config at {}. Use cleo://login/<api_key> or create the file with an `api_token` field.",
                path.display()
            )));
        }
        Err(err) => return Err(CaptureError::from(err)),
    };

    serde_json::from_str(&contents).map_err(|err| {
        CaptureError::Config(format!(
            "Failed to parse Cleo config {}: {err}",
            path.display()
        ))
    })
}

fn load_api_token() -> Result<String, CaptureError> {
    let config = load_config()?;
    let path = cleo_config_path()?;
    validate_api_token(&config.api_token, &format!("Config {}", path.display()))
}

/// Resolve API base URL: config file → env var → default
fn resolve_api_base() -> String {
    if let Ok(config) = load_config() {
        if let Some(url) = config.api_url {
            if !url.is_empty() {
                return url;
            }
        }
    }
    env::var(API_BASE_ENV).unwrap_or_else(|_| DEFAULT_API_BASE.to_string())
}

fn cleo_config_path() -> Result<PathBuf, CaptureError> {
    let home = env::var("HOME").map_err(|_| {
        CaptureError::Config(
            "HOME environment variable must be set to locate ~/.config/cleo.json".into(),
        )
    })?;
    let mut path = PathBuf::from(home);
    path.push(".config");
    path.push("cleo.json");
    Ok(path)
}

fn save_api_token(token: &str) -> Result<(), CaptureError> {
    let path = cleo_config_path()?;
    let api_token = validate_api_token(token, "cleo://login token")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(CaptureError::from)?;
    }

    // Preserve existing settings if config exists
    let existing = load_config().ok();
    let config = CleoConfig {
        api_token,
        api_url: existing.as_ref().and_then(|c| c.api_url.clone()),
        privacy: existing.map(|c| c.privacy).unwrap_or_default(),
    };
    let payload = serde_json::to_string_pretty(&config).map_err(|err| {
        CaptureError::Config(format!(
            "Failed to serialize Cleo config at {}: {err}",
            path.display()
        ))
    })?;

    fs::write(&path, payload).map_err(CaptureError::from)
}

fn load_privacy_settings() -> Result<PrivacySettings, CaptureError> {
    load_config().map(|c| c.privacy)
}

fn save_privacy_settings(privacy: &PrivacySettings) -> Result<(), CaptureError> {
    let path = cleo_config_path()?;

    // Load existing config to preserve API token and api_url
    let existing = load_config().ok();
    let api_token = load_api_token().unwrap_or_default();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(CaptureError::from)?;
    }

    let config = CleoConfig {
        api_token,
        api_url: existing.and_then(|c| c.api_url),
        privacy: privacy.clone(),
    };
    let payload = serde_json::to_string_pretty(&config).map_err(|err| {
        CaptureError::Config(format!(
            "Failed to serialize Cleo config at {}: {err}",
            path.display()
        ))
    })?;

    fs::write(&path, payload).map_err(CaptureError::from)
}

fn validate_api_token(token: &str, context: &str) -> Result<String, CaptureError> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        Err(CaptureError::Config(format!(
            "{context} must provide a non-empty API token"
        )))
    } else {
        Ok(trimmed.to_owned())
    }
}

enum CleoRoute {
    Login { api_key: String },
}

impl CleoRoute {
    fn from_url(url: &Url) -> Result<Self, CaptureError> {
        if !url.scheme().eq_ignore_ascii_case("cleo") {
            return Err(CaptureError::Config(format!(
                "Unsupported URL scheme `{}` for {}",
                url.scheme(),
                url
            )));
        }

        if url
            .host_str()
            .map(|host| host.eq_ignore_ascii_case("login"))
            .unwrap_or(false)
        {
            let api_key = url
                .path_segments()
                .and_then(|mut segments| segments.next().map(|segment| segment.to_owned()))
                .ok_or_else(|| {
                    CaptureError::Config(format!(
                        "URL {url} must include an API key, e.g. cleo://login/<api_key>"
                    ))
                })?;
            let api_key = validate_api_token(&api_key, &format!("URL {url}"))?;
            return Ok(CleoRoute::Login { api_key });
        }

        let mut segments = url.path_segments().ok_or_else(|| {
            CaptureError::Config(format!(
                "URL {url} must include a route like cleo://login/<api_key>"
            ))
        })?;

        match segments.next() {
            Some(segment) if segment.eq_ignore_ascii_case("login") => {
                let api_key = segments.next().ok_or_else(|| {
                    CaptureError::Config(format!(
                        "URL {url} must include an API key, e.g. cleo://login/<api_key>"
                    ))
                })?;
                let api_key = validate_api_token(api_key, &format!("URL {url}"))?;
                Ok(CleoRoute::Login { api_key })
            }
            _ => Err(CaptureError::Config(format!(
                "Unrecognized cleo:// route in {url}. Expected cleo://login/<api_key>"
            ))),
        }
    }
}

fn show_notification(title: &str, message: &str) {
    // NSUserNotificationCenter is deprecated and returns null for accessory apps
    // Just log the notification for now
    info!("[notification] {}: {}", title, message);
}

struct MenuHandles {
    recording: MenuItemHandle,
}

impl MenuHandles {
    fn new(recording: MenuItemHandle) -> Self {
        Self { recording }
    }

    fn set_recording(&self, recording: bool) {
        let title = if recording {
            "Stop Recording"
        } else {
            "Start Recording"
        };
        self.recording.set_title(title);
        self.recording.set_enabled(true);
    }
}

struct LoggingDaemon {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl LoggingDaemon {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_flag = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !thread_flag.load(Ordering::Relaxed) {
                info!(
                    target: "daemon",
                    "heartbeat {}",
                    Local::now().format("%H:%M:%S")
                );
                if sleep_with_cancellation(&thread_flag, Duration::from_secs(1)) {
                    break;
                }
            }
            info!(target: "daemon", "stopping");
        });

        LoggingDaemon {
            stop,
            handle: Some(handle),
        }
    }

    fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        join_task_handle(&mut self.handle, "logging daemon");
    }
}

impl Drop for LoggingDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        join_task_handle(&mut self.handle, "logging daemon");
    }
}

struct RepeatingTask {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl RepeatingTask {
    fn start<F>(interval: Duration, mut action: F) -> Self
    where
        F: FnMut() + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            loop {
                if sleep_with_cancellation(&flag, interval) {
                    break;
                }
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                action();
                if flag.load(Ordering::Relaxed) {
                    break;
                }
            }
        });

        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for RepeatingTask {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        join_task_handle(&mut self.handle, "repeating task");
    }
}

struct DelayedTask {
    cancel_flag: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl DelayedTask {
    fn schedule<F>(delay: Duration, action: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let flag = Arc::new(AtomicBool::new(false));
        let wait_flag = Arc::clone(&flag);
        let handle = thread::spawn(move || {
            if sleep_with_cancellation(&wait_flag, delay) {
                return;
            }
            action();
        });

        Self {
            cancel_flag: flag,
            handle: Some(handle),
        }
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }
}

impl Drop for DelayedTask {
    fn drop(&mut self) {
        self.cancel();
        join_task_handle(&mut self.handle, "delayed task");
    }
}

fn sleep_with_cancellation(flag: &AtomicBool, duration: Duration) -> bool {
    let mut elapsed = Duration::ZERO;
    while elapsed < duration {
        if flag.load(Ordering::Relaxed) {
            return true;
        }
        let remaining = duration - elapsed;
        let step = if remaining > Duration::from_millis(TASK_SLEEP_CHUNK_MS) {
            Duration::from_millis(TASK_SLEEP_CHUNK_MS)
        } else {
            remaining
        };
        thread::sleep(step);
        elapsed += step;
    }
    flag.load(Ordering::Relaxed)
}

fn join_task_handle(handle: &mut Option<thread::JoinHandle<()>>, task_name: &str) {
    if let Some(handle) = handle.take() {
        if handle.thread().id() == thread::current().id() {
            warn!(
                "Skipping join for `{}` because shutdown was called from the same thread",
                task_name
            );
            return;
        }

        if handle.join().is_err() {
            error!("`{}` thread panicked during shutdown", task_name);
        }
    }
}

struct ScreenRecorder {
    stream: SCStream,
    recording_output: SCRecordingOutput,
    file_path: PathBuf,
    started_at: Instant,
    running: bool,
}

impl ScreenRecorder {
    fn start_with_exclusions(privacy: &PrivacySettings) -> Result<Self, CaptureError> {
        let content = SCShareableContent::get().map_err(CaptureError::from)?;
        let display = content
            .displays()
            .into_iter()
            .next()
            .ok_or(CaptureError::NoDisplay)?;

        // Find windows to exclude based on app name/bundle ID or window title
        let all_windows = content.windows();
        let excluded_windows: Vec<&_> = all_windows
            .iter()
            .filter(|w| {
                let window_title = w.title().unwrap_or_default();
                if let Some(app) = w.owning_application() {
                    let app_name = app.application_name();
                    let bundle_id = app.bundle_identifier();
                    privacy.should_block(&app_name, &bundle_id, &window_title)
                } else {
                    // No owning app - just check window title
                    privacy.should_block("", "", &window_title)
                }
            })
            .collect();

        if !excluded_windows.is_empty() {
            info!(
                "Excluding {} windows from privacy settings",
                excluded_windows.len()
            );
        }

        let filter = SCContentFilter::builder()
            .display(&display)
            .exclude_windows(&excluded_windows)
            .build();

        let config = SCStreamConfiguration::new()
            .with_width(display.width())
            .with_height(display.height())
            .with_shows_cursor(true)
            .with_fps(30);

        let stream = SCStream::new(&filter, &config);
        let file_path = recording_file_path();

        let recording_config = SCRecordingOutputConfiguration::new()
            .with_output_url(&file_path)
            .with_video_codec(SCRecordingOutputCodec::H264)
            .with_output_file_type(SCRecordingOutputFileType::MP4);

        let recording_output =
            SCRecordingOutput::new(&recording_config).ok_or(CaptureError::RecordingUnavailable)?;

        stream
            .add_recording_output(&recording_output)
            .map_err(CaptureError::from)?;
        stream.start_capture().map_err(CaptureError::from)?;

        Ok(Self {
            stream,
            recording_output,
            file_path,
            started_at: Instant::now(),
            running: true,
        })
    }

    /// Stop recording and move file to pending folder for batch processing
    fn stop(mut self) -> Result<(), CaptureError> {
        self.stop_stream()?;
        thread::sleep(Duration::from_millis(100));
        let recorded_for = self.started_at.elapsed();

        // Move to pending recordings folder
        let pending_dir = pending_recordings_dir();
        fs::create_dir_all(&pending_dir)?;

        let filename = self.file_path.file_name().ok_or_else(|| {
            CaptureError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid recording path",
            ))
        })?;
        let pending_path = pending_dir.join(filename);

        fs::rename(&self.file_path, &pending_path)?;
        info!(
            "Recording saved to {} (duration {:.1}s)",
            pending_path.display(),
            recorded_for.as_secs_f32()
        );

        Ok(())
    }

    fn file_path(&self) -> &Path {
        &self.file_path
    }

    fn stop_stream(&mut self) -> Result<(), CaptureError> {
        if self.running {
            self.stream.stop_capture().map_err(CaptureError::from)?;
            self.stream
                .remove_recording_output(&self.recording_output)
                .map_err(CaptureError::from)?;
            self.running = false;
        }
        Ok(())
    }
}

impl Drop for ScreenRecorder {
    fn drop(&mut self) {
        if let Err(err) = self.stop_stream() {
            error!("Failed to stop stream during drop: {err}");
        }
        let _ = fs::remove_file(&self.file_path);
    }
}

#[derive(Debug)]
enum CaptureError {
    NoDisplay,
    RecordingUnavailable,
    Io(std::io::Error),
    ScreenKit(SCError),
    Api(ApiError),
    ApiUnavailable,
    ImageEncoding(String),
    Config(String),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptureError::NoDisplay => write!(f, "No displays available for capture"),
            CaptureError::RecordingUnavailable => {
                write!(f, "Recording output is unavailable on this system")
            }
            CaptureError::Io(err) => write!(f, "Filesystem error: {err}"),
            CaptureError::ScreenKit(err) => write!(f, "ScreenCaptureKit error: {err}"),
            CaptureError::Api(err) => write!(f, "Capture API error: {err}"),
            CaptureError::ApiUnavailable => {
                write!(
                    f,
                    "Capture API client is unavailable. Set {} to point to the server.",
                    API_BASE_ENV
                )
            }
            CaptureError::ImageEncoding(err) => write!(f, "Failed to encode image: {err}"),
            CaptureError::Config(err) => write!(f, "{err}"),
        }
    }
}

impl From<SCError> for CaptureError {
    fn from(value: SCError) -> Self {
        CaptureError::ScreenKit(value)
    }
}

impl From<std::io::Error> for CaptureError {
    fn from(value: std::io::Error) -> Self {
        CaptureError::Io(value)
    }
}

impl From<ApiError> for CaptureError {
    fn from(value: ApiError) -> Self {
        CaptureError::Api(value)
    }
}

impl From<EncodingError> for CaptureError {
    fn from(value: EncodingError) -> Self {
        CaptureError::ImageEncoding(value.to_string())
    }
}

fn recording_file_path() -> PathBuf {
    let mut path = env::temp_dir();
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    path.push(format!("cleo-recording-{stamp}.mp4"));
    path
}

fn pending_screenshots_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(PENDING_SCREENSHOTS_DIR)
}

fn pending_recordings_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(PENDING_RECORDINGS_DIR)
}

fn image_format_from_path(path: &Path) -> Option<ImageFormat> {
    let ext = path.extension()?.to_str()?;
    if ext.eq_ignore_ascii_case("png") {
        Some(ImageFormat::Png)
    } else if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") {
        Some(ImageFormat::Jpeg)
    } else if ext.eq_ignore_ascii_case("gif") {
        Some(ImageFormat::Gif)
    } else if ext.eq_ignore_ascii_case("webp") {
        Some(ImageFormat::Webp)
    } else {
        None
    }
}

fn video_format_from_path(path: &Path) -> Option<VideoFormat> {
    let ext = path.extension()?.to_str()?;
    if ext.eq_ignore_ascii_case("mov") {
        Some(VideoFormat::QuickTime)
    } else if ext.eq_ignore_ascii_case("mp4") {
        Some(VideoFormat::Mp4)
    } else if ext.eq_ignore_ascii_case("webm") {
        Some(VideoFormat::Webm)
    } else {
        None
    }
}

/// Save screenshot to local pending folder (no classification, no upload)
fn capture_screenshot_with_exclusions(privacy: &PrivacySettings) -> Result<(), CaptureError> {
    let content = SCShareableContent::get().map_err(CaptureError::from)?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .ok_or(CaptureError::NoDisplay)?;

    // Find windows to exclude based on app name/bundle ID or window title
    let all_windows = content.windows();
    let excluded_windows: Vec<&_> = all_windows
        .iter()
        .filter(|w| {
            let window_title = w.title().unwrap_or_default();
            if let Some(app) = w.owning_application() {
                let app_name = app.application_name();
                let bundle_id = app.bundle_identifier();
                privacy.should_block(&app_name, &bundle_id, &window_title)
            } else {
                // No owning app - just check window title
                privacy.should_block("", "", &window_title)
            }
        })
        .collect();

    let filter = SCContentFilter::builder()
        .display(&display)
        .exclude_windows(&excluded_windows)
        .build();

    let config = SCStreamConfiguration::new()
        .with_width(display.width())
        .with_height(display.height())
        .with_shows_cursor(true);

    let image = SCScreenshotManager::capture_image(&filter, &config).map_err(CaptureError::from)?;
    let rgba = image.rgba_data().map_err(CaptureError::from)?;
    let width = image.width() as u32;
    let height = image.height() as u32;

    let png = encode_png(width, height, &rgba)?;

    // Save to local pending folder
    let dir = pending_screenshots_dir();
    fs::create_dir_all(&dir)?;
    let stamp = Local::now().format("%Y%m%d-%H%M%S-%3f");
    let path = dir.join(format!("screenshot-{stamp}.png"));
    fs::write(&path, &png)?;
    eprintln!("[DEBUG] Screenshot saved to {}", path.display());
    info!("Screenshot saved to {}", path.display());

    Ok(())
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, CaptureError> {
    let mut buffer = Vec::new();
    {
        let mut encoder = Encoder::new(&mut buffer, width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
    }
    Ok(buffer)
}

/// Background task that periodically processes pending captures:
/// - Reads all files from ~/.cleo/captures/screenshots and recordings
/// - Classifies each with NSFW filter
/// - Uploads passing files to API
/// - Deletes all files (pass or fail)
struct BatchUploader {
    stop: Arc<AtomicBool>,
}

impl BatchUploader {
    fn start() -> Self {
        eprintln!("[DEBUG] BatchUploader::start() called");
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);

        thread::spawn(move || {
            eprintln!("[DEBUG] BatchUploader thread spawned");
            // Create our own API client and content filter
            eprintln!("[DEBUG] BatchUploader: building API client");
            let api = match build_api_client() {
                Ok(client) => {
                    eprintln!("[DEBUG] BatchUploader: API client created successfully");
                    client
                }
                Err(e) => {
                    error!("BatchUploader: Failed to create API client: {}", e);
                    return;
                }
            };

            eprintln!("[DEBUG] BatchUploader: loading NSFW filter");
            let content_filter: Box<dyn ContentFilter> = match NsfwFilter::new() {
                Ok(filter) => {
                    info!("BatchUploader: NSFW filter loaded");
                    Box::new(filter)
                }
                Err(e) => {
                    warn!(
                        "BatchUploader: NSFW filter unavailable ({}), using no-op",
                        e
                    );
                    Box::new(NoOpFilter::new())
                }
            };

            eprintln!("[DEBUG] BatchUploader: entering main loop");
            info!(
                "BatchUploader: Started, processing every {}s",
                UPLOAD_BATCH_INTERVAL_SECS
            );

            while !flag.load(Ordering::Relaxed) {
                eprintln!(
                    "[DEBUG] BatchUploader: sleeping for {}s",
                    UPLOAD_BATCH_INTERVAL_SECS
                );
                if sleep_with_cancellation(&flag, Duration::from_secs(UPLOAD_BATCH_INTERVAL_SECS)) {
                    eprintln!("[DEBUG] BatchUploader: sleep cancelled, exiting");
                    break;
                }
                if flag.load(Ordering::Relaxed) {
                    eprintln!("[DEBUG] BatchUploader: stop flag set, exiting");
                    break;
                }
                eprintln!("[DEBUG] BatchUploader: calling process_pending");
                Self::process_pending(&api, &*content_filter, &flag);
            }
            eprintln!("[DEBUG] BatchUploader: thread exiting");
        });

        Self { stop }
    }

    fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        // Keep termination responsive: the uploader may be inside a long
        // classify/upload call, so we avoid blocking the main thread on join.
    }

    fn process_pending(
        api: &ApiClient,
        content_filter: &dyn ContentFilter,
        cancel_flag: &AtomicBool,
    ) {
        if cancel_flag.load(Ordering::Relaxed) {
            return;
        }
        eprintln!("[DEBUG] process_pending() called");
        // Process screenshots - batch classify then upload, processing ALL files continuously
        let screenshot_dir = pending_screenshots_dir();
        eprintln!("[DEBUG] Checking screenshot dir: {:?}", screenshot_dir);
        if let Ok(entries) = fs::read_dir(&screenshot_dir) {
            let mut files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| image_format_from_path(p).is_some())
                .collect();
            files.sort(); // Process oldest first
            eprintln!("[DEBUG] Found {} screenshots to process", files.len());

            // Process all files continuously until exhausted (dedup happens inside)
            if !files.is_empty() {
                info!("Processing {} pending screenshots", files.len());
                Self::batch_process_screenshots(api, content_filter, cancel_flag, &files);
            }
        } else {
            eprintln!("[DEBUG] Could not read screenshot dir");
        }

        if cancel_flag.load(Ordering::Relaxed) {
            return;
        }

        // Process recordings - batch classify then upload
        let recording_dir = pending_recordings_dir();
        if let Ok(entries) = fs::read_dir(&recording_dir) {
            let mut files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| video_format_from_path(p).is_some())
                .collect();
            files.sort(); // Process oldest first

            // Process all recordings continuously
            if !files.is_empty() {
                info!("Processing {} pending recordings", files.len());
                Self::batch_process_recordings(api, content_filter, cancel_flag, &files);
            }
        }
    }

    fn batch_process_screenshots(
        api: &ApiClient,
        content_filter: &dyn ContentFilter,
        cancel_flag: &AtomicBool,
        files: &[PathBuf],
    ) {
        eprintln!(
            "[DEBUG] batch_process_screenshots() called with {} files",
            files.len()
        );

        // Create perceptual hasher - using mean algorithm for speed, 8x8 hash
        let hasher = HasherConfig::new()
            .hash_alg(HashAlg::Mean)
            .hash_size(8, 8)
            .to_hasher();

        // Process all files continuously, filling batches of BATCH_SIZE unique images
        let mut last_hash: Option<ImageHash> = None;
        let mut duplicates_skipped = 0usize;
        let mut total_processed = 0usize;
        let mut file_iter = files.iter().peekable();

        while file_iter.peek().is_some() {
            if cancel_flag.load(Ordering::Relaxed) {
                info!("Cancelling screenshot batch processing");
                return;
            }

            // Fill a batch with up to BATCH_SIZE unique images
            let mut prepared: Vec<(PathBuf, Vec<u8>, Vec<u8>, ImageFormat)> = Vec::new(); // (path, media bytes, scaled_rgb, format)

            while prepared.len() < BATCH_SIZE {
                if cancel_flag.load(Ordering::Relaxed) {
                    info!("Cancelling screenshot batch preparation");
                    return;
                }

                let path = match file_iter.next() {
                    Some(p) => p,
                    None => break, // No more files
                };

                let format = match image_format_from_path(path) {
                    Some(format) => format,
                    None => {
                        warn!("Unsupported screenshot format for {}", path.display());
                        let _ = fs::remove_file(path);
                        continue;
                    }
                };

                let bytes = match fs::read(path) {
                    Ok(b) => b,
                    Err(e) => {
                        error!("Failed to read {}: {}", path.display(), e);
                        let _ = fs::remove_file(path);
                        continue;
                    }
                };

                let img = match image::load_from_memory(&bytes) {
                    Ok(img) => img,
                    Err(e) => {
                        error!("Failed to decode {}: {}", path.display(), e);
                        let _ = fs::remove_file(path);
                        continue;
                    }
                };

                let rgba = img.to_rgba8();
                let (width, height) = rgba.dimensions();

                let scaled = match content_filter.scale(rgba.as_raw(), width, height) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to scale {}: {}", path.display(), e);
                        let _ = fs::remove_file(path);
                        continue;
                    }
                };

                // Compute perceptual hash on 512x512 (faster than full-res, more detail than 224)
                let phash_img = img.resize(512, 512, image::imageops::FilterType::Triangle);
                let current_hash = hasher.hash_image(&phash_img);

                // Skip similar frames (perceptual hash with hamming distance threshold)
                if let Some(ref prev_hash) = last_hash {
                    let distance = prev_hash.dist(&current_hash);
                    eprintln!(
                        "[DEBUG] phash distance={} (threshold={}) for {}",
                        distance,
                        PHASH_DISTANCE_THRESHOLD,
                        path.display()
                    );
                    if distance <= PHASH_DISTANCE_THRESHOLD {
                        eprintln!(
                            "[DEBUG] Skipping similar frame, deleting: {}",
                            path.display()
                        );
                        duplicates_skipped += 1;
                        if let Err(e) = fs::remove_file(path) {
                            eprintln!("[DEBUG] Failed to delete {}: {}", path.display(), e);
                        }
                        continue;
                    }
                } else {
                    eprintln!("[DEBUG] First frame (no previous hash): {}", path.display());
                }
                last_hash = Some(current_hash);

                prepared.push((path.clone(), bytes, scaled, format));
            }

            if prepared.is_empty() {
                continue; // Try next iteration (may have more files after duplicates)
            }

            if cancel_flag.load(Ordering::Relaxed) {
                info!("Cancelling screenshot classification");
                return;
            }

            // Classify this batch
            eprintln!(
                "[DEBUG] Starting batch classification of {} images",
                prepared.len()
            );
            info!("Classifying batch of {} screenshots", prepared.len());
            let scaled_batch: Vec<Vec<u8>> =
                prepared.iter().map(|(_, _, s, _)| s.clone()).collect();
            let results = match content_filter.classify(&scaled_batch) {
                Ok(r) => {
                    eprintln!("[DEBUG] Classification complete, got {} results", r.len());
                    r
                }
                Err(e) => {
                    error!("Batch classification failed: {}", e);
                    for (path, _, _, _) in &prepared {
                        let _ = fs::remove_file(path);
                    }
                    continue;
                }
            };

            let mut safe_uploads: Vec<(PathBuf, Vec<u8>, ImageFormat)> = Vec::new();
            for ((path, bytes, _, format), is_safe) in prepared.into_iter().zip(results) {
                if is_safe {
                    safe_uploads.push((path, bytes, format));
                } else {
                    info!("BLOCKED: {}", path.display());
                    let _ = fs::remove_file(&path);
                }
            }

            // Upload this batch
            if !safe_uploads.is_empty() {
                if cancel_flag.load(Ordering::Relaxed) {
                    info!("Cancelling screenshot upload");
                    return;
                }

                eprintln!(
                    "[DEBUG] Starting batch upload of {} safe screenshots",
                    safe_uploads.len()
                );
                info!(
                    "{} screenshots passed filter, uploading as batch",
                    safe_uploads.len()
                );
                let batch: Vec<_> = safe_uploads
                    .iter()
                    .map(|(_, bytes, format)| (bytes.clone(), *format))
                    .collect();
                match api.upload_images(batch) {
                    Ok(result) => {
                        eprintln!("[DEBUG] Batch upload finished");
                        info!(
                            "Batch upload complete: {} uploaded, {} failed",
                            result.uploaded, result.failed
                        );
                        total_processed += result.uploaded;
                        // Delete only files confirmed as successfully uploaded.
                        if result.failed == 0 {
                            for (path, _, _) in &safe_uploads {
                                let _ = fs::remove_file(path);
                            }
                        } else if !result.successful_indices.is_empty() {
                            let mut deleted = 0usize;
                            for idx in result.successful_indices {
                                match safe_uploads.get(idx) {
                                    Some((path, _, _)) => {
                                        let _ = fs::remove_file(path);
                                        deleted += 1;
                                    }
                                    None => {
                                        warn!(
                                            "Upload result returned out-of-range screenshot index {} (batch size {})",
                                            idx,
                                            safe_uploads.len()
                                        );
                                    }
                                }
                            }
                            info!(
                                "Deleted {} uploaded screenshots; retained {} for retry",
                                deleted,
                                safe_uploads.len().saturating_sub(deleted)
                            );
                        } else {
                            warn!(
                                "Partial screenshot batch upload without per-file success metadata; retaining all {} files for retry",
                                safe_uploads.len()
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "[DEBUG] Batch upload failed, keeping files for retry: {}",
                            e
                        );
                        error!("Batch upload failed: {}", e);
                    }
                }
            }
        }

        if duplicates_skipped > 0 {
            info!(
                "Skipped {} duplicate static frames total",
                duplicates_skipped
            );
        }
        if total_processed > 0 {
            info!(
                "Total screenshots processed this cycle: {}",
                total_processed
            );
        }
        eprintln!("[DEBUG] batch_process_screenshots() done");
    }

    fn batch_process_recordings(
        api: &ApiClient,
        content_filter: &dyn ContentFilter,
        cancel_flag: &AtomicBool,
        files: &[PathBuf],
    ) {
        // Step 1: Sample frames from all recordings
        let mut prepared: Vec<(PathBuf, Vec<Vec<u8>>)> = Vec::new(); // (path, scaled_frames)

        for path in files {
            if cancel_flag.load(Ordering::Relaxed) {
                info!("Cancelling recording batch processing");
                return;
            }

            let frames = match content_filter.sample(path, 2) {
                Ok(f) => f,
                Err(e) => {
                    error!("Failed to sample {}: {}", path.display(), e);
                    let _ = fs::remove_file(path);
                    continue;
                }
            };

            let mut scaled_frames = Vec::new();
            let mut ok = true;
            for frame in &frames {
                if cancel_flag.load(Ordering::Relaxed) {
                    info!("Cancelling recording frame processing");
                    return;
                }

                match content_filter.scale(&frame.rgba, frame.width, frame.height) {
                    Ok(s) => scaled_frames.push(s),
                    Err(e) => {
                        error!("Failed to scale frame in {}: {}", path.display(), e);
                        ok = false;
                        break;
                    }
                }
            }

            if ok {
                prepared.push((path.clone(), scaled_frames));
            } else {
                let _ = fs::remove_file(path);
            }
        }

        if prepared.is_empty() {
            return;
        }

        if cancel_flag.load(Ordering::Relaxed) {
            return;
        }

        // Step 2: Batch classify all frames from all recordings in single forward pass
        // Flatten all frames into one batch, track which video each belongs to
        let mut all_frames: Vec<Vec<u8>> = Vec::new();
        let mut frame_counts: Vec<usize> = Vec::new();
        for (_, scaled_frames) in &prepared {
            frame_counts.push(scaled_frames.len());
            all_frames.extend(scaled_frames.iter().cloned());
        }

        info!(
            "Classifying {} frames from {} recordings",
            all_frames.len(),
            prepared.len()
        );

        let results = match content_filter.classify(&all_frames) {
            Ok(r) => r,
            Err(e) => {
                error!("Batch classification failed: {}", e);
                for (path, _) in &prepared {
                    let _ = fs::remove_file(path);
                }
                return;
            }
        };

        // Map results back to videos
        let mut safe_paths: Vec<PathBuf> = Vec::new();
        let mut result_idx = 0;
        for (path, _) in prepared {
            if cancel_flag.load(Ordering::Relaxed) {
                info!("Cancelling recording result mapping");
                return;
            }

            let frame_count = frame_counts.remove(0);
            let frame_results = &results[result_idx..result_idx + frame_count];
            result_idx += frame_count;

            if frame_results.iter().all(|&safe| safe) {
                safe_paths.push(path);
            } else {
                let blocked_frame = frame_results.iter().position(|&s| !s).unwrap_or(0);
                info!("BLOCKED at frame {}: {}", blocked_frame, path.display());
                let _ = fs::remove_file(&path);
            }
        }

        // Step 3: Read and collect all safe recordings for batch upload
        let mut safe_uploads: Vec<(PathBuf, Vec<u8>, VideoFormat)> = Vec::new();
        for path in safe_paths {
            if cancel_flag.load(Ordering::Relaxed) {
                info!("Cancelling recording upload preparation");
                return;
            }

            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    error!("Failed to read {}: {}", path.display(), e);
                    let _ = fs::remove_file(&path);
                    continue;
                }
            };

            let format = match video_format_from_path(&path) {
                Some(format) => format,
                None => {
                    warn!("Unsupported recording format for {}", path.display());
                    let _ = fs::remove_file(&path);
                    continue;
                }
            };

            safe_uploads.push((path, bytes, format));
        }

        // Step 4: Batch upload all safe recordings
        if !safe_uploads.is_empty() {
            if cancel_flag.load(Ordering::Relaxed) {
                info!("Cancelling recording upload");
                return;
            }

            info!(
                "{} recordings passed filter, uploading as batch",
                safe_uploads.len()
            );
            let batch: Vec<_> = safe_uploads
                .iter()
                .map(|(_, bytes, format)| (bytes.clone(), *format))
                .collect();
            match api.upload_videos(batch) {
                Ok(result) => {
                    eprintln!(
                        "[recording] Batch upload complete: {} uploaded, {} failed",
                        result.uploaded, result.failed
                    );
                    info!(
                        "Batch upload complete: {} uploaded, {} failed",
                        result.uploaded, result.failed
                    );
                    // Delete only files confirmed as successfully uploaded.
                    if result.failed == 0 {
                        for (path, _, _) in &safe_uploads {
                            let _ = fs::remove_file(path);
                        }
                    } else if !result.successful_indices.is_empty() {
                        let mut deleted = 0usize;
                        for idx in result.successful_indices {
                            match safe_uploads.get(idx) {
                                Some((path, _, _)) => {
                                    let _ = fs::remove_file(path);
                                    deleted += 1;
                                }
                                None => {
                                    warn!(
                                        "Upload result returned out-of-range recording index {} (batch size {})",
                                        idx,
                                        safe_uploads.len()
                                    );
                                }
                            }
                        }
                        info!(
                            "Deleted {} uploaded recordings; retained {} for retry",
                            deleted,
                            safe_uploads.len().saturating_sub(deleted)
                        );
                    } else {
                        warn!(
                            "Partial recording batch upload without per-file success metadata; retaining all {} files for retry",
                            safe_uploads.len()
                        );
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[recording] Batch upload failed, keeping files for retry: {}",
                        e
                    );
                    error!("Batch upload failed: {}", e);
                }
            }
        }
    }
}

impl Drop for BatchUploader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Thread will exit on next sleep_with_cancellation check (max 100ms)
        // Don't block on join during app termination
    }
}
